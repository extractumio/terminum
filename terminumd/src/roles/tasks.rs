// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A manager's runs (`terminum-agents.md` AC-179): it starts its project's tasks on the pool's agents as a human's
//! RUN does, through Dispatcher, answers and goes on, stops; and reads what runs and the project's numbers.

use serde_json::{json, Value};

use super::{arg, in_project, opt, supervisor, yes, Boss, DATA};
use crate::app::Shared;
use crate::dispatcher::{runs, waits};
use crate::tasks::api::store;
use crate::tasks::{agents, ops};

pub async fn list_runs(app: &Shared, boss: &Boss) -> Result<String, String> {
    let project = boss.project.clone();
    let listed = store(app, move |tx| {
        let waits: Vec<Value> = ops::rows(tx, &format!("SELECT t.slug, a.name FROM run_waits w JOIN tasks t ON t.id = w.task LEFT JOIN agents a ON a.id = w.agent_id WHERE w.project = ?1 ORDER BY {}", waits::ORDER),
                                          [&project], |r| Ok(json!({"task": r.get::<_, String>(0)?, "waits_for": r.get::<_, Option<String>>(1)?})))?;
        let live: Vec<Value> = runs::live(tx)?.into_iter().filter(|r| r.project == project && r.holds_slot()).map(|r| json!({
            "run": r.id, "kind": if r.ad_hoc() { "session" } else { "task" }, "tasks": r.ids, "open": r.open, "status": r.status, "agent": r.agent_name,
            "state": r.state.label(), "session": r.pane_key, "working": !r.turn_ended, "asked_a_human": r.asked,
        })).collect();
        Ok(json!({"runs": live, "waiting_for_a_slot": waits}))
    }).await?;
    Ok(format!("{DATA}\n{listed}"))
}

/// The agent of `project`'s pool named `named`: by its name, else its id.
pub fn agent_named(tx: &rusqlite::Transaction, project: &str, named: &str) -> Result<agents::Agent, String> {
    let all = agents::of_project(tx, project)?;
    all.iter().find(|a| a.name == named).or_else(|| all.iter().find(|a| a.id == named)).cloned()
        .ok_or_else(|| format!("Your project's pool has no agent {named:?}: {}.", if all.is_empty() { "it is empty".to_string() } else { all.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ") }))
}

pub async fn run_task(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let id = arg(args, "id")?.to_string();
    let (b, wanted, named) = (boss.clone(), id.clone(), opt(args, "agent").map(str::to_string));
    let agent = store(app, move |tx| {
        in_project(tx, &b, &wanted)?;
        named.map(|n| agent_named(tx, &b.project, &n)).transpose()
    }).await?;
    let comment = args["comment"].as_str().map(crate::plans::rules::clean).filter(|c| !c.trim().is_empty());
    let name = agent.as_ref().map(|a| a.name.clone());
    let done = crate::dispatcher::api::press_by_manager(app, &id, &boss.user, agent.map(|a| a.id), comment, (yes(args, "fresh"), yes(args, "queue"))).await?;
    let on = name.map(|n| format!(" on {n}")).unwrap_or_default();
    Ok(if let Some(words) = done["waits"].as_str() {
        format!("{id} waits for a slot: {words}")
    } else if let Some(words) = done["full"]["words"].as_str() {
        format!("Not started: {words} Nothing waits: call run_task again with queue \"yes\" to wait for a slot, or choose another agent.")
    } else {
        format!("{id} runs{on}: a worker's session has it. list_runs shows its state; get_task its result or its questions, which run_task answers with comment.")
    })
}

/// Moves an existing press, never starts one: scope, order and history share admission's store transaction (AC-169).
pub async fn promote_queued_task(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    promote_queued(app, boss, args, false).await
}

pub(super) async fn promote_queued(app: &Shared, boss: &Boss, args: &Value, supervised: bool) -> Result<String, String> {
    let (b, id) = (boss.clone(), arg(args, "id")?.to_string());
    let (id, name, moved, behind) = store(app, move |tx| {
        let t = in_project(tx, &b, &id)?;
        let gone = || format!("{} waits no more: it started, or its wait was called off.", t.id);
        let w = waits::of_task(tx, &t.key)?.ok_or_else(|| format!("{} has no waiting run of its own: use a queued primary task from list_runs.", t.id))?;
        if supervised {
            supervisor::require(tx, &b.key, &b.project, "promote_queued_task")?;
            crate::supervisor::task_policy::check(tx, &t, ops::now())?;
            if w.scheduled {
                return Err("The Supervisor cannot promote a schedule's task.".into());
            }
            supervisor::author(tx, &b.project)?;
        }
        if w.project != b.project {
            return Err(format!("{}'s wait is outside your project.", t.id));
        }
        if t.archived_at.is_some() || t.status != w.status { return Err(gone()); }
        let a = agents::in_project(tx, &w.agent, &b.project)?.ok_or_else(gone)?;
        let line = waits::of_agent(tx, &w.agent)?;
        // the front of its own kind: a wait that is no schedule's task stops behind those that are (terminum-schedules.md AC-161)
        let front = if w.scheduled { 0 } else { line.iter().filter(|x| x.scheduled).count() };
        let mut order: Vec<String> = line.into_iter().map(|x| x.task).collect();
        let moved = order.get(front) != Some(&t.key);
        if moved {
            order.retain(|key| key != &t.key);
            order.insert(front, t.key.clone());
            waits::reorder(tx, &w.agent, &order)?;
            ops::log(tx, &t.key, &b.user, "why", &format!("{} moved it to the front of {}'s queue.", b.name, a.name), None, ops::now())?;
        }
        Ok((t.id, a.name, moved, front > 0))
    }).await?;
    if moved { crate::tasks::api::told(app, &id); }
    Ok(format!("{id} {} at the front of {name}'s queue{}. It starts when there is room.", if moved { "is now" } else { "is already" }, if behind { ", behind its schedules' tasks" } else { "" }))
}

pub async fn stop_task(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let id = arg(args, "id")?.to_string();
    let (scope, actor) = (boss.clone(), boss.clone());
    let stopped = crate::dispatcher::letgo::gave_up_checked(app, &id, "The project's manager", move |tx, wanted| {
        in_project(tx, &scope, wanted).map(drop)
    }, move |tx, ids| {
        for id in ids {
            let key = ops::key_of(tx, id)?;
            ops::log(tx, &key, &actor.user, "why", &format!("{} stopped it.", actor.name), None, ops::now())?;
        }
        Ok(())
    }).await?;
    Ok(format!("Stopped: {}. Its window stays; its slot is free.", stopped.join(", ")))
}

pub async fn dashboard(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let (tab, period, project) = (args["tab"].as_str().unwrap_or("system").to_string(), crate::dashboard::Period::of(args["period"].as_str().unwrap_or("")), boss.project.clone());
    let colors = crate::messages::api::colors(app);
    let numbers = store(app, move |tx| crate::dashboard::numbers(tx, crate::inputs::now_ms(), &tab, period, Some(&project), &colors)).await?;
    Ok(numbers.to_string())
}
