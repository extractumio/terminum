// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Where a run starts and its window opens (`terminum-dispatcher.md` AC-140, AC-142): a project's place, what
//! refuses it before any slot is asked, and + Session, with its first prompt (AC-204).

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::press::members;
use super::runs::{self, Phase, Run};
use super::{firstprompt, pool, start, store, Held};
use crate::app::Shared;
use crate::tasks::agents::{self, Agent};
use crate::tasks::api::answer;
use crate::tasks::ops;
pub(super) use crate::tasks::ops::no_folder;
use crate::tasks::store::DEFAULT_PROJECT;
use crate::tmux::{PaneState, Server};

/// Whether terminum's hooks are in place for `kind` on this Mac (AC-121).
pub(super) fn hooks_there(kind: &str) -> bool {
    if crate::config::test_store() {
        return true;
    }
    crate::cli::Ctx::from_env("terminumd").is_ok_and(|ctx| crate::cli::agentsetup::installed(&ctx, kind))
}

/// Whether `folder` stands on this Mac: off the async workers, as a stale mount can hang.
pub(super) async fn folder_there(folder: &str) -> bool {
    let path = std::path::PathBuf::from(crate::tasks::folder::folder_in(&crate::config::home().display().to_string(), folder));
    crate::http::off(move || Ok(path.is_dir())).await.unwrap_or(false)
}

pub(super) fn server_of(app: &Shared, host: Option<&str>) -> Result<Arc<Server>, String> {
    match host {
        None => Ok(app.tmux.local.clone()),
        Some(h) => app.tmux.host(h).filter(|s| s.connected()).ok_or_else(|| format!("The host {h} is out of reach.")),
    }
}

/// The longest a press waits for a linked pane's server to be read again.
const REREAD_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// A linked pane its server's list lacks may be newer than that list (an agent's take_task): that server is
/// read once more before the press judges it closed (AC-113). The press's judge of a linked pane: one whose
/// server was not read again in time stays Unread, never Closed by the older list.
pub(super) async fn linked_panes(app: &Shared, id: &str, also: &[String]) -> impl Fn(&str) -> PaneState + Send + 'static {
    let (ids, more) = (id.to_string(), also.to_vec());
    let keys = store(app, move |tx| Ok(members(tx, &ids, &more)?.into_iter().filter_map(|t| t.session).collect::<Vec<_>>())).await.unwrap_or_default();
    let mut late = HashSet::new();
    for key in keys.into_iter().filter(|k| app.tmux.pane_state(k) == PaneState::Closed) {
        let read = match app.tmux.resolve(&key) {
            Some((server, _)) => tokio::time::timeout(REREAD_WAIT, crate::lists::refresh_server(app, &server)).await.is_ok(),
            None => true,
        };
        if !read {
            late.insert(key);
        }
    }
    let a = app.clone();
    move |k: &str| if late.contains(k) { PaneState::Unread } else { a.tmux.pane_state(k) }
}

/// Where a run's pane is.
pub(super) fn pane_of(app: &Shared, r: &Run) -> Result<(Arc<Server>, String), String> {
    r.place(app).ok_or_else(|| "Its session is gone.".to_string())
}

/// A project's id and name for its sessions' place in the list; Default stands for none.
pub(super) fn project_named(app: &Shared, project: &str) -> Option<(String, String)> {
    if project == DEFAULT_PROJECT {
        return None;
    }
    app.projects.lock().list().iter().find(|p| p.id == project).map(|p| (p.id.clone(), p.name.clone()))
}

/// Where `agent` would start in `project`, and what refuses it before any slot is asked (AC-140): no folder, no such
/// agent, a host's link down, a folder or hooks not there.
pub(super) async fn place(app: &Shared, project: &str, agent: Option<String>) -> Result<(Agent, Option<String>, String, Arc<Server>), String> {
    let project = project.to_string();
    let (a, host, dir) = store(app, move |tx| {
        let (name, host, dir) = ops::project_place(tx, &project)?;
        no_folder(&name, &dir)?;
        let a = agent.as_deref().map(|id| agents::chosen(tx, id, &project)).transpose()?.ok_or("No agents yet: add one first.")?;
        Ok((a, host, dir))
    }).await?;
    let server = server_of(app, host.as_deref())?;
    if let Some(host) = &host {
        let state = crate::hosts::link::agents_state(app, host);
        if state == "off" {
            return Err(format!("Run terminum hooks install on {host} first"));
        }
        if !state.starts_with("up") {
            return Err(format!("The link to {host}'s agents is not up: {state}"));
        }
    } else {
        if !folder_there(&dir).await {
            return Err(format!("The folder {dir} does not exist on this Mac"));
        }
        let kind = a.kind.clone();
        if !crate::http::off(move || Ok(hooks_there(&kind))).await.unwrap_or(false) {
            return Err("Run terminum hooks install first: Dispatcher cannot follow the agent without them.".to_string());
        }
    }
    Ok((a, host, dir, server))
}

#[derive(Deserialize)]
pub(super) struct AdHoc {
    agent: String,
    /// the human's first prompt, typed once (AC-204); none: nothing is typed (AC-142)
    #[serde(default)]
    prompt: Option<String>,
    /// files the human added for it (`/api/task-files`, no task's): their paths go into the prompt, on this Mac only
    #[serde(default)]
    files: Vec<String>,
}

/// A project's agent with no task, + Session's (AC-142), the human's from its first box; with a prompt, that typed
/// once first (AC-204). `typed` says whether it went in, `why` what missed: the session stays either way.
pub(super) async fn ad_hoc(State(app): State<Shared>, Path(project): Path<String>, Json(body): Json<AdHoc>) -> Response {
    answer(session(&app, &project, body).await)
}

pub(super) async fn session(app: &Shared, project: &str, body: AdHoc) -> Result<Value, String> {
    let (a, host, dir, server) = place(app, project, Some(body.agent)).await?;
    let prompt = firstprompt::asked(app, body.prompt.as_deref(), &body.files, host.is_some()).await?;
    let (ag, p, f) = (a.clone(), project.to_string(), dir.clone());
    let run = store(app, move |tx| {
        if let Some(full) = pool::check(tx, Some(&ag), &p, None)? {
            return Err(full.words);
        }
        runs::add(tx, &Run::of(&ag, host, f, p, None), &[], runs::Started::of(runs::How::Session), ops::now())
    }).await?;
    let Some(text) = prompt else {
        let key = launch(app, &run, &server, project, &dir, &a, &[]).await?;
        return Ok(json!({"run": run, "key": key}));
    };
    // its start is this request's until the prompt went in or missed: a walk's watch would take an empty box as AC-142's
    // and leave nothing to type into; on a task of its own, so a page that goes away cuts no paste short
    let hold = Held::take(app, |d| &d.starting, &run).ok_or("its start is already watched")?;
    let (app, project) = (app.clone(), project.to_string());
    tokio::spawn(async move {
        let key = open_run(&app, &run, &server, &project, &dir, &a, &[]).await?;
        let typed = firstprompt::type_once(&app, &run, &text, firstprompt::Scope::Human).await;
        drop(hold);
        start::watch(&app, &run);                    // after the prompt: a run it made Working is no more its to type in
        Ok(json!({"run": run, "key": key, "typed": typed.is_ok(), "why": typed.err()}))
    }).await.map_err(|e| format!("+ Session stopped: {e}"))?
}

/// The window of a run just added: opened at its place and watched; one that does not open closes its run. Its key.
pub(super) async fn launch(app: &Shared, run: &str, server: &Arc<Server>, project: &str, dir: &str, a: &Agent, tasks: &[String]) -> Result<String, String> {
    let key = open_run(app, run, server, project, dir, a, tasks).await?;
    start::watch(app, run);
    Ok(key)
}

/// The window of a run just added, opened at its place, not yet watched; one that does not open closes its run.
async fn open_run(app: &Shared, run: &str, server: &Arc<Server>, project: &str, dir: &str, a: &Agent, tasks: &[String]) -> Result<String, String> {
    let named = project_named(app, project);
    let opening = start::Opening { server, project: named.as_ref().map(|(i, n)| (i.as_str(), n.as_str())), folder: dir, command: &a.line, run, tasks, program: &a.kind, agent: &a.id };
    let opened = start::open(app, opening).await;
    if opened.is_err() {
        let r = run.to_string();
        let _ = store(app, move |tx| runs::change(tx, &r, |now| now.state = Phase::Closed).map(drop)).await;
    }
    opened
}
