// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The Page's view of plans and notes (AC-97, AC-98): the rows' summaries, the Plan pane's
//! `plan` answer, the user's `note` actions, and `plannew` to every page on a change.

use std::sync::Arc;

use serde_json::{json, Value};

use super::Page;
use crate::app::Shared;
use crate::notes::{Note, Scope};
use crate::plans::{Details, Row};

/// Each row's plan summary and open notes count (absent without one).
pub fn mark(app: &Shared, groups: &mut [Value]) {
    let (now, projects, open) = (crate::auth::store::unix_now(), crate::notes::projects_of(app), app.notes.open_counts());
    for it in groups.iter_mut().flat_map(|g| g["items"].as_array_mut().into_iter().flatten()) {
        let Some(key) = it["id"].as_str().map(str::to_string) else { continue };
        let row = app.plans.row(&key, app.hooks.working(&key), now);
        let scope = Scope { project: projects.get(&key).cloned(), key };
        put(it, row, open.of(&scope));
    }
}

/// Row `it`'s `plan` and `notes`, each absent without one.
pub fn put(it: &mut Value, plan: Option<Row>, open: usize) {
    let Some(o) = it.as_object_mut() else { return };
    match plan {
        Some(r) => { o.insert("plan".into(), json!(r)); }
        None => { o.remove("plan"); }
    }
    match open {
        0 => { o.remove("notes"); }
        n => { o.insert("notes".into(), json!(n)); }
    }
}

/// The pane keys `scope` shows in: its project's sessions, else its pane.
pub fn keys_of(app: &Shared, scope: &Scope) -> Vec<String> {
    match &scope.project {
        Some(id) => app.projects.lock().list().iter().find(|p| p.id == *id).map(|p| p.members.clone()).unwrap_or_default(),
        None => vec![scope.key.clone()],
    }
}

/// Every page hears that the plans or notes of `keys` changed.
pub fn tell(pages: &super::Pages, keys: &[String]) {
    for key in keys {
        pages.tell(&json!({"t": "plannew", "key": key}).to_string());
    }
}

/// The `plan` answer for pane `key`: its plan (null without one) and its scope's notes.
/// Without a plan, the pane's newest archived one, marked `finished` (its agent exited, AC-97).
pub fn answer(app: &Shared, key: &str) -> Value {
    let now = crate::auth::store::unix_now();
    let (plan, finished) = match app.plans.details(key, app.hooks.working(key), now) {
        Some(d) => (Some(d), false),
        None => (app.plans.last_archived(key, now), true),
    };
    let mut v = reply(key, plan, &app.notes.list(&crate::notes::scope_of(app, key)));
    if finished && !v["plan"].is_null() {
        v["plan"]["finished"] = json!(true);
    } else {
        hooks(&mut v, app.hooks.session(key).is_some());
    }
    v
}

/// A plan of a pane terminumd has no hook event from carries `hooks: false` (its agent's
/// hooks are not installed or not trusted, F8).
pub fn hooks(v: &mut Value, hooked: bool) {
    if !hooked && !v["plan"].is_null() {
        v["plan"]["hooks"] = json!(false);
    }
}

/// The `plan` message of pane `key`.
pub fn reply(key: &str, plan: Option<Details>, notes: &[Note]) -> Value {
    let plan = plan.map(|d| json!({
        "title": d.title, "doing": d.doing, "items": d.items.iter().map(|i| json!({"text": i.text, "status": i.status})).collect::<Vec<_>>(),
        "done": d.row.done, "total": d.row.total, "pct": d.row.pct, "eta": d.row.eta, "eta_given": d.eta_given,
        "updated": d.row.updated, "source": d.source, "partial": d.row.partial, "agent": d.agent,
    }));
    let notes: Vec<Value> = notes.iter().map(note).collect();
    json!({"t": "plan", "key": key, "plan": plan, "notes": notes})
}

fn note(n: &Note) -> Value {
    json!({"id": n.id, "kind": n.kind, "text": n.text, "file": n.file, "status": n.status, "at": n.at, "done_at": n.done_at})
}

/// The user's `act` on note `id`: its scope's panes hear it, else why not.
pub fn act(app: &Shared, id: u64, act: &str) -> Result<Vec<String>, String> {
    let scope = app.notes.scope(id).ok_or_else(|| format!("No note {id}."))?;
    app.notes.user(id, act, crate::auth::store::unix_now())?;
    Ok(keys_of(app, &scope))
}

/// `plan` and `note` from a page.
pub async fn handle(app: &Shared, page: &Arc<Page>, t: &str, msg: &Value) {
    let key = msg["key"].as_str().unwrap_or("").to_string();
    if t == "plan" {
        page.send_json(&answer(app, &key));
        return;
    }
    let (a, id, what) = (app.clone(), msg["id"].as_u64().unwrap_or(0), msg["act"].as_str().unwrap_or("").to_string());
    match tokio::task::spawn_blocking(move || act(&a, id, &what)).await.map_err(|e| e.to_string()).and_then(|r| r) {
        Ok(keys) => tell(&app.pages, &keys),
        Err(e) => super::client::error(page, e, Some(&key)),
    }
}

#[cfg(test)]
#[path = "plans_tests.rs"]
mod tests;
