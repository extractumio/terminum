// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Where a run starts and its window opens (`terminum-dispatcher.md` AC-140, AC-142): a project's place, what
//! refuses it before any slot is asked, and + Session.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::runs::{self, Phase, Run};
use super::{pool, start, store};
use crate::app::Shared;
use crate::tasks::agents::{self, Agent};
use crate::tasks::api::answer;
use crate::tasks::ops;
pub(super) use crate::tasks::ops::no_folder;
use crate::tasks::store::DEFAULT_PROJECT;
use crate::tmux::Server;

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
}

/// + Session (AC-142): a project's agent with no task, nothing typed: the human's from its first box.
pub(super) async fn ad_hoc(State(app): State<Shared>, Path(project): Path<String>, Json(body): Json<AdHoc>) -> Response {
    answer(session(&app, &project, body.agent).await)
}

pub(super) async fn session(app: &Shared, project: &str, agent: String) -> Result<Value, String> {
    let (a, host, dir, server) = place(app, project, Some(agent)).await?;
    let (ag, p, f) = (a.clone(), project.to_string(), dir.clone());
    let run = store(app, move |tx| {
        if let Some(full) = pool::check(tx, Some(&ag), &p, None)? {
            return Err(full.words);
        }
        runs::add(tx, &Run::of(&ag, host, f, p, None), &[], runs::Started::of(runs::How::Session), ops::now())
    }).await?;
    let key = launch(app, &run, &server, project, &dir, &a, &[]).await?;
    Ok(json!({"run": run, "key": key}))
}

/// The window of a run just added: opened at its place and watched; one that does not open closes its run. Its key.
pub(super) async fn launch(app: &Shared, run: &str, server: &Arc<Server>, project: &str, dir: &str, a: &Agent, tasks: &[String]) -> Result<String, String> {
    let named = project_named(app, project);
    let opening = start::Opening { server, project: named.as_ref().map(|(i, n)| (i.as_str(), n.as_str())), folder: dir, command: &a.line, run, tasks, program: &a.kind, agent: &a.id };
    match start::open(app, opening).await {
        Ok(key) => {
            start::watch(app, run);
            Ok(key)
        }
        Err(e) => {
            let r = run.to_string();
            let _ = store(app, move |tx| runs::change(tx, &r, |now| now.state = Phase::Closed).map(drop)).await;
            Err(e)
        }
    }
}
