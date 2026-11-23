// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-245: a held run takes the same slot and resume path as RUN.

use rusqlite::Transaction;
use serde_json::Value;

use super::{How, Kept, Press, Shared, Task, pressed};
use crate::dispatcher::{goes_on, runs::{self, Started}};
use crate::roles::supervisor as authority;
use crate::supervisor::holds;
use crate::tasks::{api::store, ops, store::HUMAN};

impl Press {
    pub(in crate::dispatcher) fn started_as<'a>(&'a self, actor: Option<&'a str>) -> Started<'a> {
        actor.map_or_else(|| self.started(), |by| Started { by, how: self.how })
    }
}

#[derive(Clone)]
pub(in crate::dispatcher) struct Release {
    project: String,
    hold: i64,
    pane: Option<String>,
    run: String,
}

impl Release {
    pub(in crate::dispatcher) fn divert(&self, tx: &Transaction) -> Result<Option<Kept>, String> {
        let Some(pane) = &self.pane else { return Ok(None) };
        authority::require(tx, pane, &self.project, "release_hold")?;
        Ok(crate::supervisor::action_tasks::diverted_hold_run(tx, &self.project, self.hold, ops::now())?
            .map(|card| Kept::DecisionCards(vec![card.task])))
    }

    pub(in crate::dispatcher) fn check(&self, tx: &Transaction, tasks: &[Task]) -> Result<String, String> {
        if let Some(pane) = &self.pane {
            authority::require(tx, pane, &self.project, "release_hold")?;
        }
        holds::check_run_release(tx, &self.project, self.hold, &self.run, tasks, self.pane.is_some())?;
        self.pane.as_ref().map_or_else(|| Ok(HUMAN.into()), |_| authority::author(tx, &self.project))
    }
}

pub(crate) async fn resume(app: &Shared, project: String, hold: i64, pane: Option<String>, task: String) -> Result<Value, String> {
    let id = task.clone();
    let (release, run, tasks) = store(app, move |tx| {
        let (original, pending) = holds::run_undo(tx, &project, hold)?;
        let lead = ops::get(tx, &id)?;
        let latest = pending.as_deref().unwrap_or(&original.run);
        let mut run = runs::last_of(tx, &lead.key)?.filter(|r| r.id == latest).ok_or("The held run has changed.")?;
        let mut tasks = vec![lead];
        for other in original.tasks.iter().filter(|other| **other != id) {
            tasks.push(ops::get(tx, other)?);
        }
        let release = Release { project, hold, pane, run: run.id.clone() };
        release.check(tx, &tasks)?;
        if run.agent_session.is_none() && run.id != original.run {
            run.agent_session = runs::by_id(tx, &original.run)?.agent_session;
        }
        Ok((release, run, tasks))
    }).await?;
    let supervisor = release.pane.is_some();
    let press = Press { hold_release: Some(release), ..Default::default() };
    if run.agent_session.is_some() {
        let words = format!("[terminum Dispatcher] {}: the hold on this run was released. Go on with the task.",
            tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(", "));
        goes_on::resume(app, run, &tasks, words.into(), press).await
    } else if supervisor {
        Err("No retained conversation is available. The owner can release this hold with RUN.".into())
    } else {
        let also = tasks.iter().skip(1).map(|t| t.id.clone()).collect();
        pressed(app, &task, Press { agent: Some(run.agent_id), also, fresh: true, how: How::Run, ..press }).await
    }
}

pub(in crate::dispatcher) fn admitted(tx: &Transaction, press: &Press, tasks: &[Task], actor: Option<&str>, run: &str) -> Result<(), String> {
    if let Some(release) = &press.hold_release {
        return holds::reserve_run_release(tx, release.hold, actor.ok_or("The hold release was not checked.")?, run);
    }
    if press.started().by == HUMAN {
        for task in tasks {
            holds::reserve_board_run(tx, &task.project, &task.key, run)?;
        }
    }
    Ok(())
}
