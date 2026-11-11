// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A manager's runs (`terminum-agents.md` AC-175): it starts its project's tasks on the pool's agents as a human's
//! RUN does, through Dispatcher, answers and goes on, stops; and reads what runs and the project's numbers.

use serde_json::{json, Value};

use super::{arg, in_project, yes, Boss, DATA};
use crate::app::Shared;
use crate::dispatcher::runs;
use crate::tasks::api::store;
use crate::tasks::{agents, ops};

pub async fn list_runs(app: &Shared, boss: &Boss) -> Result<String, String> {
    let project = boss.project.clone();
    let listed = store(app, move |tx| {
        let waits: Vec<Value> = ops::rows(tx, "SELECT t.slug, a.name FROM run_waits w JOIN tasks t ON t.id = w.task LEFT JOIN agents a ON a.id = w.agent_id WHERE w.project = ?1 ORDER BY w.at",
                                          [&project], |r| Ok(json!({"task": r.get::<_, String>(0)?, "waits_for": r.get::<_, Option<String>>(1)?})))?;
        let live: Vec<Value> = runs::live(tx)?.into_iter().filter(|r| r.project == project && r.holds_slot()).map(|r| json!({
            "run": r.id, "kind": if r.ad_hoc() { "session" } else { "task" }, "tasks": r.ids, "open": r.open, "status": r.status, "agent": r.agent_name,
            "state": r.state.label(), "session": r.pane_key, "working": !r.turn_ended, "asked_a_human": r.asked,
        })).collect();
        Ok(json!({"runs": live, "waiting_for_a_slot": waits}))
    }).await?;
    Ok(format!("{DATA}\n{listed}"))
}

/// The pool's agent named `named`: by its name, else its id.
pub fn agent_named(tx: &rusqlite::Transaction, named: &str) -> Result<agents::Agent, String> {
    let all = agents::list(tx)?;
    all.iter().find(|a| a.name == named).or_else(|| all.iter().find(|a| a.id == named)).cloned()
        .ok_or_else(|| format!("The pool has no agent {named:?}: {}.", if all.is_empty() { "it is empty".to_string() } else { all.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ") }))
}

pub async fn run_task(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let id = arg(args, "id")?.to_string();
    let (b, wanted, named) = (boss.clone(), id.clone(), args["agent"].as_str().map(str::trim).filter(|a| !a.is_empty()).map(str::to_string));
    let agent = store(app, move |tx| {
        in_project(tx, &b, &wanted)?;
        named.map(|n| agent_named(tx, &n)).transpose()
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

pub async fn stop_task(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let id = arg(args, "id")?.to_string();
    let (b, wanted) = (boss.clone(), id.clone());
    store(app, move |tx| in_project(tx, &b, &wanted).map(drop)).await?;
    let done = crate::dispatcher::letgo::gave_up(app, &id, "The project's manager").await?;
    let (b, ids) = (boss.clone(), done["ids"].clone());
    store(app, move |tx| {
        for id in ids.as_array().into_iter().flatten().filter_map(Value::as_str) {
            ops::log(tx, &ops::key_of(tx, id)?, &b.user, "why", &format!("{} stopped it.", b.name), None, ops::now())?;
        }
        Ok(())
    }).await?;
    Ok(format!("Stopped: {}. Its window stays; its slot is free.", done["ids"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")))
}

pub async fn dashboard(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let (tab, period, project) = (args["tab"].as_str().unwrap_or("system").to_string(), crate::dashboard::Period::of(args["period"].as_str().unwrap_or("")), boss.project.clone());
    let colors = crate::messages::api::colors(app);
    let numbers = store(app, move |tx| crate::dashboard::numbers(tx, crate::inputs::now_ms(), &tab, period, Some(&project), &colors)).await?;
    Ok(numbers.to_string())
}
