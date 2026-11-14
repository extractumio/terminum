// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! How a press goes on with a conversation a run left (terminum-dispatcher.md AC-122, AC-157, AC-185): in its live
//! window, in its pane's shell, or resumed in a new window, with every task of the press.

use std::sync::Arc;

use serde_json::{json, Value};

use super::api::{wait_if_full, Next, Press};
use super::press::bring_back;
use super::runs::{self, Phase, Run};
use super::places::{folder_there, project_named, server_of};
use super::{move_one, move_task, pool, procs, say, start, store};
use crate::app::Shared;
use crate::tasks::agents;
use crate::tasks::ops::{self, Task};
use crate::tmux::Server;

/// The conversation goes on in a new window (AC-122), on its agent's slot or waiting for one (AC-158), with every task
/// of the press: a main task's subtasks go on with it (AC-185).
pub(super) async fn resume(app: &Shared, r: Run, tasks: &[Task], again: String, kept: Press) -> Result<Value, String> {
    let t = &tasks[0];
    let Ok(line) = start::resume_command(&r) else {
        return could_not_continue(app, &r, "no conversation is known").await;
    };
    if r.host.is_none() && !folder_there(&r.folder).await {
        return could_not_continue(app, &r, "its first folder is gone").await;
    }
    let server = server_of(app, r.host.as_deref())?;
    let new = match next_run(app, &r, tasks, again, kept).await? {
        Next::Run(new) => new,
        Next::Kept(kept) => return Ok(kept.answer(app, t)),
    };
    let (project, ids) = (project_named(app, &t.project), tasks.iter().map(|t| t.id.clone()).collect::<Vec<_>>());
    start::open(app, start::Opening { server: &server, project: project.as_ref().map(|(i, n)| (i.as_str(), n.as_str())), folder: &r.folder, command: &line, run: &new, tasks: &ids, program: &r.kind, agent: &r.agent_id }).await?;
    start::watch(app, &new);
    Ok(json!({"run": new}))
}

/// What runs in `pane` other than its shell: in front, or stopped or left behind it (AC-122).
async fn what_runs(app: &Shared, server: &Server, pane: &str) -> Result<Option<String>, String> {
    let said = server.run(format!("display -p -t {pane} '#{{pane_current_command}}\t#{{pane_pid}}'")).await.map_err(|e| format!("tmux does not answer: {e}"))?;
    let (name, pid) = said.first().and_then(|l| l.split_once('\t')).unwrap_or(("", ""));
    if !crate::tmux::titles::is_shell(name) {
        return Ok(Some(name.to_string()));
    }
    // the moment after an agent's end its wrapper and a prompt's helpers still run: what stays is a job
    for look in 0..3 {
        let tree = procs::read(app, server).await.ok_or("ps did not answer")?;
        if !procs::under(&tree, pid).any {
            return Ok(None);
        }
        if look < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }
    }
    Ok(Some("a job of its shell".to_string()))
}

/// Only its shell remains: the agent starts there anew, by the resume when a conversation is known;
/// the pane's queue held from the look to the start, so no key starts anything between (AC-122).
pub(super) async fn from_shell(app: &Shared, r: &Run, server: &Arc<Server>, pane: &str, again: String, tasks: &[Task], kept: Press) -> Result<Value, String> {
    let t = &tasks[0];
    let _held = server.typing(pane).lock_owned().await;
    if let Some(program) = what_runs(app, server, pane).await? {
        return Err(format!("{program} runs in its pane."));
    }
    let line = start::resume_command(r).unwrap_or_else(|_| r.command.clone());
    let new = match next_run(app, r, tasks, again, kept).await? {
        Next::Run(new) => new,
        Next::Kept(kept) => return Ok(kept.answer(app, t)),
    };
    let done = start::respawn(app, server, pane, &r.id, &new, &r.folder, &line).await;
    crate::inputs::wrote(app, crate::inputs::Who::dispatcher("dispatcher.start", &new, std::slice::from_ref(&t.id), &r.agent_id), "launch", server, pane, &line, done.as_ref().err().cloned(), None);
    done?;
    let (a, key, n) = (app.clone(), server.key(pane), new.clone());
    app.dispatcher.panes.lock().insert(key.clone());    // its agent's first hooks
    store(app, move |tx| a.dispatcher.pane_opened(tx, &n, &key).map(drop)).await?;
    start::watch(app, &new);
    Ok(json!({"run": new}))
}

/// The conversation cannot go on: FAILED, its comment says why, and Run from scratch stays (AC-122).
pub(super) async fn could_not_continue(app: &Shared, r: &Run, why: &str) -> Result<Value, String> {
    let (run, words) = (r.clone(), format!("Conversation {} could not be continued: {why}", r.agent_session.as_deref().unwrap_or("unknown")));
    store(app, move |tx| super::fail(tx, &run, &words)).await?;
    super::told_all(app, r);
    Err(format!("{why}: use Run from scratch."))
}

/// RUN goes on in its live session: every clock starts anew; each finished task of the press reopens (a main task's
/// subtasks with it, AC-185), an open one with the others waiting with it.
pub(super) async fn went_on(app: &Shared, r: &Run, tasks: &[Task], manager: bool) -> Result<Value, String> {
    let (id, all, words) = (r.id.clone(), tasks.to_vec(), if manager { "The project's manager ran it again." } else { "The human ran it again." });
    let done = store(app, move |tx| {
        let now = runs::change(tx, &id, |now| { now.state = Phase::Working; now.new_work(); })?;
        let mut open = false;
        for t in &all {
            bring_back(tx, &t.id)?;
            if super::plan::finished(&t.status) {
                move_one(tx, &t.id, "IN PROGRESS", words)?;
                runs::reopened(tx, &t.key)?;
            } else {
                open = true;
            }
        }
        if open {
            move_task(tx, &now, "IN PROGRESS", words)?;
        }
        Ok(now.id)
    }).await?;
    super::told_all(app, r);
    Ok(json!({"run": done}))
}

/// A new run of `t` going on from `r`, which is over: its agent, unless another run took it meanwhile. What
/// else was in `r` lets go of its session: it stays open to its own Run, and no agent there holds it.
async fn next_run(app: &Shared, r: &Run, tasks: &[Task], prompt: String, kept: Press) -> Result<Next, String> {
    let (old, all) = (r.clone(), tasks.to_vec());
    store(app, move |tx| {
        // an agent deleted since has no slot to take: its run's own settings start, or nothing waits for it
        let agent = agents::in_project(tx, &old.agent_id, &old.project)?;
        match &agent {
            Some(a) => if let Some(kept) = wait_if_full(tx, a, &old.project, (&all[0], all.len() == 1), Some(&old.id), (&kept, None, &[]))? {
                return Ok(Next::Kept(kept));
            },
            None => if let Some(full) = pool::check(tx, None, &old.project, Some(&old.id))? {
                return Err(full.words);
            },
        }
        if agent.is_none() {
            say(tx, &old, &format!("{} is no longer in the pool: this goes on with the settings it started with.", old.agent_name))?;
        }
        runs::change(tx, &old.id, |was| { was.state = Phase::Closed; was.pane_key = None; })?;
        for other in old.ids.iter().filter(|other| !all.iter().any(|t| t.id == **other)) {
            ops::link(tx, super::DISPATCHER, other, None, ops::now())?;
        }
        for t in &all {
            bring_back(tx, &t.id)?;
            super::waits::remove(tx, &t.key)?;
        }
        let keys: Vec<String> = all.iter().map(|t| t.key.clone()).collect();
        let (from, id) = (old.id.clone(), runs::add(tx, &Run { prompt: Some(prompt), ..old }, &keys, kept.started(), ops::now())?);
        super::usage::carry(tx, &from, &id)?;
        Ok(Next::Run(id))
    }).await
}
