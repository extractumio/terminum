// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The board's routes (`terminum-kanban.md` §9), behind the page's sign-in and write checks. Store
//! work runs off the async workers; every change tells every page (`taskchange`).

use std::collections::HashMap;

use axum::extract::{Path, Query as Params, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use rusqlite::Transaction;
use serde::Serialize;
use serde_json::{json, Value};

use super::files;
use crate::dispatcher::runs;
use super::ops::{self, Edit, NewTask, ProjectMove, Task};
use super::query::{self, Query};
use super::store::{DEFAULT_PROJECT, HUMAN};
use crate::app::Shared;
use crate::http::{off, refuse};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/tasks", get(board).post(make))
        .route("/api/tasks/meta", get(meta))
        .route("/api/tasks/start-empty", post(start_empty))
        .merge(super::projectapi::routes())
        .merge(super::agents::routes())
        .merge(super::limits::routes())
        .merge(super::integration::routes())
        .merge(super::auto::routes())
        .merge(super::folder::routes())
        .merge(super::schedulesapi::routes())
        .merge(crate::dispatcher::api::routes())
        .route("/api/tasks/{id}", get(window).patch(change))
        .route("/api/tasks/{id}/comments", post(add_comment))
        .route("/api/tasks/{id}/comments/{event}", patch(edit_comment).delete(drop_comment))
        .route("/api/tasks/{id}/purge", post(purge))
        .route("/api/tasks/{id}/archive", post(archive))
        .route("/api/tasks/{id}/seen", post(seen))
        .merge(super::fileapi::routes())
}

/// Runs `f` in one transaction off the async workers.
pub(crate) async fn store<T: Send + 'static>(app: &Shared, f: impl FnOnce(&Transaction) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let a = app.clone();
    off(move || a.tasks.with(f)).await
}

pub(crate) fn answer<T: Serialize>(done: Result<T, String>) -> Response {
    match done {
        Ok(v) => Json(v).into_response(),
        Err(e) => refuse(StatusCode::BAD_REQUEST, e),
    }
}

/// `done`'s answer; every page hears when it changed a project or its agents.
pub(super) fn told_projects<T: Serialize>(app: &Shared, done: Result<T, String>) -> Response {
    if done.is_ok() {
        crate::page::projects::tell(app);
    }
    answer(done)
}

/// `done`'s answer; every page hears when it changed task `id`.
fn told_answer<T: Serialize>(app: &Shared, id: &str, done: Result<T, String>) -> Response {
    if done.is_ok() {
        told(app, id);
    }
    answer(done)
}

/// Every open page hears a task changed ("": several); its window reads the history anew.
fn change_heard(app: &Shared, task: &str) {
    app.pages.tell(&json!({"t": "taskchange", "id": task}).to_string());
}

/// Only its history changed: its window reads that anew, and a board has no card to read again.
pub(super) fn history_heard(app: &Shared, task: &str) {
    app.pages.tell(&json!({"t": "taskchange", "id": task, "history": true}).to_string());
}

/// A task's change: every page hears of it, and the session list of what it shows of tasks.
pub fn told(app: &Shared, task: &str) {
    change_heard(app, task);
    super::marks::read_later(app);
}

/// A link only to a session that is there: a page's picker can be older than the list (AC-133).
fn session_gone(app: &Shared, body: &Value) -> bool {
    let layout = app.layout.read();
    ["session", "made_in"].iter().any(|k| body[k].as_str().is_some_and(|key| crate::page::marks::find_item(&layout, key).is_none()))
}

/// A task with its session: `row` its row in the list, its live state, agent and why it failed or
/// waits; none: closed.
fn with_session(t: &Task, row: Option<&Value>) -> Value {
    let mut v = serde_json::to_value(t).unwrap_or_default();
    if let Some(key) = &t.session {
        v["session"] = match row {
            Some(it) => json!({"key": key, "state": it["state"], "agent": it["agent"], "why": it["why"], "background": it["background"].as_bool().unwrap_or(false), "closed": false}),
            None => json!({"key": key, "closed": true}),
        };
    }
    v
}

fn card(app: &Shared, t: &Task) -> Value {
    let row = t.session.as_deref().and_then(|key| crate::page::marks::find_item(&app.layout.read(), key).cloned());
    with_session(t, row.as_ref())
}

/// What a text is compared by: a save says which it began from (AC-133).
fn text_rev(text: &str) -> String {
    crate::auth::crypto::sha256_hex(text.as_bytes())
}

fn csv(q: &HashMap<String, String>, key: &str) -> Vec<String> {
    q.get(key).map(|v| v.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect()).unwrap_or_default()
}

/// A page of cards under the board's filters; with `counts=1`, each column's count too.
async fn board(State(app): State<Shared>, Params(q): Params<HashMap<String, String>>) -> Response {
    let sort = match query::Sort::parse(q.get("sort").map(String::as_str)) {
        Ok(sort) => sort,
        Err(e) => return refuse(StatusCode::BAD_REQUEST, e),
    };
    let filter = Query {
        projects: csv(&q, "project"),
        severities: csv(&q, "severity"),
        statuses: csv(&q, "status"),
        owner: q.get("owner").filter(|o| !o.is_empty()).cloned(),
        number: None,
        reach: None,
        text: q.get("q").cloned(),
        made_in: q.get("made_in").filter(|m| !m.is_empty()).cloned(),
        with_subtasks: q.get("subtasks").is_some_and(|c| c == "1"),
        archived: q.get("archived").is_some_and(|c| c == "1"),
        sort,
        after: q.get("after").and_then(|a| a.parse().ok()).unwrap_or(0),
        limit: q.get("limit").and_then(|l| l.parse().ok()).unwrap_or(0),
    };
    let with_counts = q.get("counts").is_some_and(|c| c == "1");
    let one = q.get("id").cloned();
    let done = store(&app, move |tx| {
        let mut filter = filter;
        if let Some(id) = &one {
            filter.number = Some(super::slugs::resolve(tx, id)?.unwrap_or(-1));   // none such: the filter shows nothing
        }
        let tasks = query::list(tx, &filter)?;
        let keys: Vec<&str> = tasks.iter().map(|t| t.key.as_str()).collect();
        let (run_marks, on_agents) = (runs::card_marks(tx, &keys)?, runs::card_agents(tx, &keys)?);
        let waits = crate::dispatcher::waits::on_cards(tx, &keys)?;
        let from = super::schedules::on_cards(tx, &keys)?;
        let parents: Vec<&str> = tasks.iter().filter(|t| t.subtasks > 0).map(|t| t.key.as_str()).collect();
        let boxes = super::family::boxes(tx, &parents)?;
        // only a finished task says what it spent (AC-129)
        let finished: Vec<&str> = tasks.iter().filter(|t| crate::dispatcher::plan::finished(&t.status)).map(|t| t.key.as_str()).collect();
        let spent = crate::dispatcher::usage::of_tasks(tx, &finished)?;
        let counts = if with_counts { Some((query::counts(tx, &filter)?, crate::dispatcher::pool::board(tx)?)) } else { None };
        Ok((tasks, counts, run_marks, on_agents, spent, boxes, (waits, from)))
    }).await;
    match done {
        Ok((tasks, counts, run_marks, on_agents, spent, mut boxes, (waits, from))) => {
            let layout = app.layout.read();
            let linked = tasks.iter().any(|t| t.session.is_some());
            let rows: HashMap<&str, &Value> = crate::page::marks::layout_rows(&layout).filter(|_| linked).filter_map(|it| Some((it["id"].as_str()?, it))).collect();
            let shown: Vec<Value> = tasks.iter().map(|t| {
                let mut v = with_session(t, t.session.as_deref().and_then(|k| rows.get(k).copied()));
                if let Some(state) = run_marks.get(&t.key) {
                    v["run"] = json!(state);
                }
                if let Some(s) = spent.get(&t.key) {
                    v["spent"] = s.clone();
                }
                if let Some(agent) = on_agents.get(&t.key) {
                    v["agent"] = agent.clone();
                }
                if let Some(on) = waits.get(&t.key) {
                    v["waits"] = json!(on);               // RUN waits for that agent's room (terminum-dispatcher.md AC-158)
                }
                if let Some(kids) = boxes.remove(&t.key) {
                    v["boxes"] = json!(kids);
                }
                if let Some(s) = from.get(&t.key) {
                    v["schedule"] = json!(s);             // made by that schedule (terminum-schedules.md AC-163)
                }
                v
            }).collect();
            let with = counts.is_some();
            let (counts, slots) = counts.unzip();
            let mut page = json!({"counts": counts, "slots": slots});
            if with {
                page["subscriptions"] = app.dispatcher.subscriptions.shown();   // terminum-dispatcher.md AC-168
            }
            page["tasks"] = Value::Array(shown);       // json! would copy every card
            Json(page).into_response()
        }
        Err(e) => refuse(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

/// What the board's pickers offer, and the store's trouble if any.
async fn meta(State(app): State<Shared>) -> Response {
    let sessions: Vec<Value> = crate::page::marks::layout_rows(&app.layout.read())
        .map(|it| json!({"key": it["id"], "title": it["title"]})).collect();
    // the store's lock waits out a running query: off the async workers, its state too
    let a = app.clone();
    let (mut projects, users, agents, limits, last_agents, at_once, problem, set_aside) = tokio::task::spawn_blocking(move || {
        let mut projects: Vec<Value> = std::iter::once(json!({"id": DEFAULT_PROJECT, "name": "Default", "dir": "~"}))
            .chain(a.projects.lock().list().iter().map(|p| json!({"id": p.id, "name": p.name, "color": p.color, "dir": p.dir, "host": p.host}))).collect();
        let (users, agents, limits, last_agents, at_once, own) = a.tasks.with(|tx| Ok((
            ops::rows(tx, "SELECT id, name FROM users WHERE kind = 'human' ORDER BY name", [], |r| Ok(json!({"id": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?})))?,
            super::agents::listed(tx, ops::now(), None)?,
            super::limits::limits(tx)?,
            runs::last_agents(tx)?,
            super::limits::agent_limits(tx)?,
            super::projectapi::settings(tx)?,
        ))).unwrap_or_default();
        for p in &mut projects {
            if let (Some(v), Some(p)) = (p["id"].as_str().and_then(|id| own.get(id)).and_then(Value::as_object).cloned(), p.as_object_mut()) {
                p.extend(v);
            }
        }
        (projects, users, agents, limits, last_agents, at_once, a.tasks.problem(), a.tasks.is_set_aside())
    }).await.unwrap_or_default();
    let global_limits = { let s = app.looks.settings.read(); json!({"warn": s.memory_warn_gib, "end": s.memory_end_gib}) };
    for p in &mut projects {
        if let Some(limit) = p["id"].as_str().and_then(|id| at_once.get(id)) {
            p["agent_limit"] = json!(limit);
        }
    }
    Json(json!({"projects": projects, "agents": agents, "last_agents": last_agents, "efforts": super::agents::EFFORTS.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(), "limits": limits.iter().map(|(project, warn, end)| json!({"project": project, "warn": warn, "end": end})).collect::<Vec<_>>(), "global_limits": global_limits, "hosts": crate::hosts::names(&app), "sessions": sessions, "users": users, "problem": problem, "set_aside": set_aside, "most_bytes": files::MOST_BYTES, "task_template": super::template::DEFAULT_TEMPLATE, "theme": app.looks.theme(None, "").1, "scheme": app.looks.settings.read().page.color_scheme})).into_response()
}

async fn start_empty(State(app): State<Shared>) -> Response {
    let a = app.clone();
    let done = off(move || {
        a.tasks.start_empty()?;
        let _ = a.task_marks.refresh(&a.tasks);
        a.projects.lock().reload()
    }).await;
    if done.is_ok() {
        crate::page::projects::tell(&app);
        super::marks::tell(&app);
    }
    answer(done.map(|()| json!({"ok": true})))
}

pub(super) fn text(v: &Value, key: &str) -> Option<String> {
    v[key].as_str().map(str::to_string)
}

async fn make(State(app): State<Shared>, Json(body): Json<Value>) -> Response {
    if session_gone(&app, &body) {
        return refuse(StatusCode::BAD_REQUEST, "That session is gone.");
    }
    let _converting = match crate::page::marks::claim(&app, body["record"].as_str()).await {
        Ok(held) => held, Err(why) => return refuse(StatusCode::CONFLICT, why),     // held until the record goes
    };
    let new = NewTask {
        title: text(&body, "title").unwrap_or_default(),
        text: text(&body, "text").unwrap_or_default(),
        project: text(&body, "project"),
        parent: text(&body, "parent"),
        severity: text(&body, "severity"),
        owner: text(&body, "owner"),
        session: text(&body, "session"),
        by_agent: false,
        made_in: text(&body, "made_in"),           // a record's session it was made of (AC-116, AC-150)
        scheduled: false,
    };
    let (attached, runs_on) = (crate::config::strings(&body["files"]), text(&body, "agent"));
    let done = store(&app, move |tx| {
        let task = ops::create(tx, HUMAN, new, ops::now())?;
        files::attach(tx, &attached, &task.id, None)?;
        super::auto::set_agent(tx, &task.id, runs_on.as_deref(), ops::now())?;   // the agent a human chose to run it
        ops::get(tx, &task.id)
    }).await;
    if let Ok(t) = &done {
        told(&app, &t.id);
        if let Some(up) = &t.parent {
            told(&app, up);                            // its card counts it, its window lists it (AC-112)
        }
        // the record it was made of goes once the task is kept (AC-116); refused, it stays
        if let Some(record) = text(&body, "record") && let Err(e) = crate::page::marks::forget(&app, &record).await {
            tracing::warn!(event = "tasks.record_kept", error = %e);
        }
    }
    answer(done.map(|t| card(&app, &t)))
}

/// A task's window: its fields, text, comments and history, files.
async fn window(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    let done = store(&app, move |tx| {
        let t = ops::get(tx, &id)?;
        let held = runs::holding(tx, &t.key)?.map(|r| r.names());
        let agent = runs::card_agents(tx, &[&t.key])?.remove(&t.key);
        let waits = crate::dispatcher::waits::on_cards(tx, &[&t.key])?.remove(&t.key);
        let kids = query::subtasks(tx, &t.key)?;
        let from = super::schedules::of_task(tx, &t.key)?.map(|s| json!({"key": s.key, "title": s.title}));
        let runs_on = super::auto::agent_of(tx, &t.key)?;
        Ok((ops::text_of(tx, &id)?, ops::history(tx, &id)?, files::of_task(tx, &id)?, (held, waits, from, runs_on), agent, kids, t))
    }).await;
    answer(done.map(|(body, history, attached, (held, waits, from, runs_on), agent, kids, t)| {
        let mut v = card(&app, &t);
        v["children"] = json!(kids);
        v["text_rev"] = json!(text_rev(&body));
        v["text"] = json!(body);
        v["history"] = history;
        v["files"] = json!(attached);
        v["run_holds"] = json!(held);
        v["waits"] = json!(waits);
        v["schedule"] = json!(from);
        v["runs_on"] = json!(runs_on);                // the agent a human chose to run it (terminum-dispatcher.md AC-170)
        if let Some(agent) = agent {
            v["agent"] = agent;
        }
        v
    }))
}

/// What a human changes on a card.
async fn change(State(app): State<Shared>, Path(id): Path<String>, Json(body): Json<Value>) -> Response {
    if session_gone(&app, &body) {
        return refuse(StatusCode::BAD_REQUEST, "That session is gone.");
    }
    let began_from = text(&body, "text_rev");
    // a task moves when it runs and when its run ends: a human runs it (terminum-dispatcher.md AC-138)
    let to_status = text(&body, "status");
    if to_status.is_some() && !crate::config::test_store() {   // the live tests place cards by it
        return refuse(StatusCode::BAD_REQUEST, "A task moves by running it: drop it on an agent, or press Run.");
    }
    let to_project = text(&body, "project");
    let choice = text(&body, "session_choice");
    let edit = Edit { title: text(&body, "title"), text: text(&body, "text"), severity: text(&body, "severity"), owner: text(&body, "owner") };
    let dropped = crate::config::strings(&body["files"]);
    let before = body.get("before").filter(|b| !b.is_null());
    let reorder = before.is_some() || body.get("order_end").is_some();
    let before = before.and_then(Value::as_str).map(str::to_string);
    let session = body.get("session").cloned();
    let parent = body.get("parent").cloned();       // null: out of its parent (AC-112)
    let reparented = parent.is_some();
    // only null takes it off: another value is refused, never read as none (AC-126)
    let budget = match body.get("budget_minutes") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => match v.as_i64() {
            Some(m) => Some(Some(m)),
            None => return refuse(StatusCode::BAD_REQUEST, "Budget: 5 minutes to 7 days"),
        },
    };
    let done = store(&app, move |tx| {
        let at = ops::now();
        let was = ops::get(tx, &id)?;
        // the text is no longer the one the human began from: the page asks (AC-133)
        if began_from.is_some_and(|rev| rev != text_rev(&ops::text_of(tx, &id).unwrap_or_default())) {
            return Ok(Err((StatusCode::PRECONDITION_FAILED, "The text changed meanwhile.")));
        }
        // its run holds it and its session until it ends, or the human gives up (AC-138)
        if (to_project.is_some() || session.is_some()) && runs::holding(tx, &was.key)?.is_some() {
            return Ok(Err((StatusCode::BAD_REQUEST, "Its run holds it: give up on the run first.")));
        }
        let mut follow = None;
        if let Some(project) = &to_project {
            match ops::change_project(tx, HUMAN, &id, project, choice.as_deref(), at)? {
                // it has a session: the page's dialog asks what becomes of it, and sends `session_choice`
                ProjectMove::Ask => return Ok(Err((StatusCode::CONFLICT, "session_choice"))),
                ProjectMove::SessionFollows(s) => follow = Some((s, project.clone(), was.project.clone())),
                ProjectMove::Done => {}
            }
        }
        if let Some(s) = &session {
            ops::link(tx, HUMAN, &id, s.as_str(), at)?;
        }
        if let Some(up) = &parent {
            super::family::set_parent(tx, HUMAN, &id, up.as_str(), at)?;
        }
        ops::edit(tx, HUMAN, &id, edit, at)?;
        files::attach(tx, &dropped, &id, None)?;   // what was dropped in its text as it was edited (AC-109)
        if let Some(minutes) = budget {
            ops::set_budget(tx, HUMAN, &id, minutes, at)?;
        }
        if let Some(status) = &to_status {
            ops::move_to(tx, HUMAN, &id, status, at)?;
        }
        if reorder {
            ops::reorder(tx, &id, before.as_deref(), at)?;
        }
        Ok(Ok((ops::get(tx, &id)?, follow)))
    }).await;
    let (task, follow) = match done {
        Ok(Ok(d)) => d,
        Ok(Err((status, why))) => return refuse(status, why),
        Err(e) => return refuse(StatusCode::BAD_REQUEST, e),
    };
    // the session follows its task, else the task goes back where it was: never one of them moved (D02)
    let (task, note) = match follow {
        Some((session, project, was)) => match super::moves::follow_session(&app, session, project).await {
            Ok(()) => (task, None),
            Err(e) => super::moves::move_back(&app, task, was, e).await,
        },
        None => (task, None),
    };
    if budget.is_some() {
        crate::dispatcher::letgo::budget_changed(&app, &task);
    }
    told(&app, if reparented { "" } else { &task.id });   // reparented: its parents' cards, old and new, count it
    let mut answered = card(&app, &task);
    if let Some(n) = note {
        answered["note"] = json!(n);
    }
    Json(answered).into_response()
}

async fn add_comment(State(app): State<Shared>, Path(id): Path<String>, Json(body): Json<Value>) -> Response {
    let (wanted, said, attached) = (id.clone(), text(&body, "text").unwrap_or_default(), crate::config::strings(&body["files"]));
    let done = store(&app, move |tx| {
        let event = ops::comment(tx, HUMAN, &wanted, &said, ops::now())?;
        files::attach(tx, &attached, &wanted, Some(&event))?;
        Ok(json!({"event": event}))
    }).await;
    told_answer(&app, &id, done)
}

async fn edit_comment(State(app): State<Shared>, Path((id, event)): Path<(String, String)>, Json(body): Json<Value>) -> Response {
    let (wanted, said) = (id.clone(), text(&body, "text").unwrap_or_default());
    let done = store(&app, move |tx| ops::change_comment(tx, HUMAN, &wanted, &event, Some(&said), ops::now()).map(|()| json!({"ok": true}))).await;
    told_answer(&app, &id, done)
}

async fn drop_comment(State(app): State<Shared>, Path((id, event)): Path<(String, String)>) -> Response {
    let wanted = id.clone();
    let done = store(&app, move |tx| ops::change_comment(tx, HUMAN, &wanted, &event, None, ops::now()).map(|()| json!({"ok": true}))).await;
    if done.is_ok() {
        scrub(&app).await;
    }
    told_answer(&app, &id, done)
}

/// A purge's text also leaves the store's write-ahead log (AC-109).
async fn scrub(app: &Shared) {
    let a = app.clone();
    let _ = tokio::task::spawn_blocking(move || a.tasks.checkpoint()).await;
}

/// Off the board, or back (AC-118); a run that holds it ends first.
async fn archive(State(app): State<Shared>, Path(id): Path<String>, Json(body): Json<Value>) -> Response {
    let (wanted, on) = (id.clone(), body["on"].as_bool().unwrap_or(true));
    let done = store(&app, move |tx| {
        let key = ops::key_of(tx, &wanted)?;
        if runs::holding(tx, &key)?.is_some() {
            return Err("Its run holds it: give up on the run first.".to_string());
        }
        ops::archive(tx, HUMAN, &wanted, on, ops::now())
    }).await;
    told_answer(&app, &id, done.map(|()| json!({"ok": true})))
}

/// The longest key a close names: a move and a line's words.
const KEY_MOST: usize = 64 * 1024;

/// `{key?}`: the human closed that line of the task, or, with none, saw the line it has now: kept for every browser,
/// every page hears it (AC-117).
async fn seen(State(app): State<Shared>, Path(id): Path<String>, Json(body): Json<Value>) -> Response {
    let key = text(&body, "key");
    if key.as_ref().is_some_and(|k| k.is_empty() || k.len() > KEY_MOST) {
        return refuse(StatusCode::BAD_REQUEST, "Name the line by its key, as terminumd gave it, or none for the line it has now.");
    }
    let done = store(&app, move |tx| super::marks::keep_seen(tx, &id, key.as_deref())).await;
    if done == Ok(true) {
        super::marks::read_later(&app);
    }
    answer(done.map(|_| json!({"ok": true})))
}

async fn purge(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    let wanted = id.clone();
    let done = store(&app, move |tx| ops::purge_text(tx, HUMAN, &wanted, ops::now()).map(|()| json!({"ok": true}))).await;
    if done.is_ok() {
        scrub(&app).await;
    }
    told_answer(&app, &id, done)
}
