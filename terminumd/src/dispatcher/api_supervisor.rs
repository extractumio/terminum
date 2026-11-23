// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-231: only the task ID and an approved pool agent cross into Dispatcher's automatic press.

use rusqlite::Transaction;
use serde_json::Value;

use super::{How, Kept, Press, Shared, Task, pressed};
use crate::roles::{Boss, supervisor};
use crate::tasks::{agents, api::store, ops};

pub(crate) async fn press_by_supervisor(app: &Shared, id: &str, agent: &str, caller: Boss) -> Result<Value, String> {
    let (wanted, scope) = (id.to_owned(), caller.clone());
    let (refusal, cards) = store(app, move |tx| {
        supervisor::require(tx, &scope.key, &scope.project, "run_task")?;
        let tasks = super::super::press::members(tx, &wanted, &[])?;
        for task in &tasks {
            if task.project != scope.project { return Err("The task is outside this project.".into()); }
            if let Some(why) = crate::supervisor::task_policy::record_override(tx, task, ops::now())? {
                return Ok((Some(why), Vec::new()));
            }
        }
        let cards = crate::supervisor::action_tasks::divert_group(tx, &tasks, Some(&scope.key), ops::now())?;
        Ok((None, cards))
    }).await?;
    if let Some(why) = refusal { return Err(why); }
    if !cards.is_empty() { return Ok(card_answer(app, cards)); }
    pressed(app, id, Press { agent: Some(agent.into()), auto: true, fresh: true, how: How::Auto,
        supervisor: Some(caller), ..Default::default() }).await
}

/// Rechecked when the press is prepared and in the transaction that reserves the slot.
pub(super) fn check(tx: &Transaction, press: &Press, tasks: &[Task]) -> Result<Option<f64>, String> {
    let Some(caller) = &press.supervisor else { return Ok(None) };
    supervisor::require(tx, &caller.key, &caller.project, "run_task")?;
    let agent = press.agent.as_deref().ok_or("The Supervisor must name a pool agent.")?;
    if agents::in_project(tx, agent, &caller.project)?.is_none() {
        return Err("That agent is no longer in the Supervisor's project pool.".into());
    }
    for task in tasks {
        if task.project != caller.project {
            return Err(format!("{} is outside your project.", task.id));
        }
        let blocked = tx.query_row("SELECT EXISTS(
            SELECT 1 FROM task_links l JOIN tasks t ON t.id = l.other
            WHERE l.task = ?1 AND l.kind = 'blocked_by' AND t.status <> 'DONE'
            UNION ALL SELECT 1 FROM auto_rounds WHERE task = ?1
            UNION ALL SELECT 1 FROM schedule_fires WHERE task = ?1
            UNION ALL SELECT 1 FROM integration_batches WHERE task = ?1
            )", [&task.key], |r| r.get::<_, bool>(0)).map_err(ops::sql)?;
        if blocked || super::super::waits::protecting(tx, &task.key)?.is_some() || task.status != "TODO" || task.archived_at.is_some() || task.session.is_some() {
            return Err(format!("{} is not free to start as a Supervisor task.", task.id));
        }
    }
    supervisor::author(tx, &caller.project)?;
    crate::supervisor::fix_tasks::check_start(tx, tasks, ops::now()).map(Some)
}

/// Admission can follow asynchronous work, so it checks the group again.
pub(super) fn check_family(tx: &Transaction, press: &Press, tasks: &[Task]) -> Result<(), String> {
    if press.supervisor.is_none() { return Ok(()) }
    let current = super::super::press::members(tx, &tasks[0].id, &[])?;
    if !current.iter().map(|t| &t.key).eq(tasks.iter().map(|t| &t.key)) {
        return Err("The task's family changed while Run was being prepared.".into());
    }
    Ok(())
}

pub(super) fn record(tx: &Transaction, press: &Press, tasks: &[Task], run: &str, estimate: Option<f64>) -> Result<(), String> {
    if let Some(estimate) = estimate {
        tx.execute("INSERT INTO supervisor_fix_runs VALUES (?1, ?2)", (run, estimate)).map_err(ops::sql)?;
    }
    if press.auto && press.how == How::Auto {
        let pane = press.supervisor.as_ref().map(|caller| caller.key.as_str());
        for task in tasks {
            crate::supervisor::action_tasks::record_run(tx, task, pane, &format!("{run}/{}", task.key), run, ops::now())?;
        }
    }
    if let Some(caller) = &press.supervisor {
        let task = &tasks[0];
        ops::log(tx, &task.key, &caller.user, "why", "The project's Supervisor started it.", None, ops::now())?;
    }
    Ok(())
}

pub(super) fn divert(tx: &Transaction, press: &Press, tasks: &[Task]) -> Result<Option<Kept>, String> {
    if let Some(release) = &press.hold_release {
        return release.divert(tx);
    }
    if !press.auto || press.how != How::Auto { return Ok(None); }
    if let Some(caller) = &press.supervisor {
        supervisor::require(tx, &caller.key, &caller.project, "run_task")?;
    }
    let cards = crate::supervisor::action_tasks::divert_group(tx, tasks,
        press.supervisor.as_ref().map(|caller| caller.key.as_str()), ops::now())?;
    Ok((!cards.is_empty()).then_some(Kept::DecisionCards(cards)))
}

pub(super) fn card_answer(app: &Shared, cards: Vec<String>) -> Value {
    for id in &cards { super::told(app, id); }
    serde_json::json!({"decision_cards": cards})
}
