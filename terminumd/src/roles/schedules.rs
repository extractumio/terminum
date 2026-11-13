// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A manager's hand on its project's schedules (`terminum-agents.md` AC-180, `terminum-schedules.md` AC-182): made,
//! changed, deleted by the board's own rules (tasks/schedules.rs), its author shown on the card; and run now.

use rusqlite::Transaction;
use serde_json::{json, Value};

use super::tasks::agent_named;
use super::{arg, number, opt, text, Boss, DATA};
use crate::app::Shared;
use crate::tasks::api::store;
use crate::tasks::ops;
use crate::tasks::schedules::{self, Given, Schedule};

/// The manager's project's schedule `args` name: by its key (SCH-1), else its id.
fn of_project(tx: &Transaction, boss: &Boss, args: &Value) -> Result<Schedule, String> {
    let named = arg(args, "schedule")?;
    schedules::all(tx)?.into_iter().find(|s| s.project == boss.project && (s.key.eq_ignore_ascii_case(named) || s.id == named))
        .ok_or_else(|| format!("Your project has no schedule {named:?}: list_schedules shows them."))
}

pub async fn list(app: &Shared, boss: &Boss) -> Result<String, String> {
    let project = boss.project.clone();
    let listed: Vec<Value> = store(app, move |tx| Ok(schedules::listed(tx, ops::now())?.into_iter().filter(|s| s["project"] == project.as_str()).collect())).await?;
    Ok(format!("{DATA}\n{}", json!(listed)))
}

/// The body the board's own rules judge: `args` over what the schedule `was`, in the manager's project.
fn body(tx: &Transaction, boss: &Boss, args: &Value, was: Option<&Schedule>) -> Result<Value, String> {
    let agent = match opt(args, "agent") {
        Some(named) => Some(agent_named(tx, &boss.project, named)?.id),
        None => was.map(|s| s.agent.clone()),
    };
    // a new cadence takes the place of the old one, whichever kind it was
    let (cron, once) = (opt(args, "cron").map(str::to_string), number(args, "once_at", 0..=i64::MAX)?);
    let (cron, once) = if cron.is_some() || once.is_some() { (cron, once) } else { (was.and_then(|s| s.cron.clone()), was.and_then(|s| s.once_at)) };
    let paused = match args["paused"].as_str() {
        Some(p) => p == "yes",
        None => was.is_some_and(|s| s.paused),
    };
    Ok(json!({
        "title": text(args, "title", was.map(|s| s.title.as_str())), "body": text(args, "text", was.map(|s| s.body.as_str())), "project": boss.project,
        "severity": text(args, "severity", was.map(|s| s.severity.as_str())), "agent": agent, "cron": cron, "once_at": once,
        "budget_minutes": number(args, "budget_minutes", 1..=i64::MAX)?.or(was.map(|s| s.budget_seconds / 60)), "paused": paused,
    }))
}

pub async fn create(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let (b, args) = (boss.clone(), args.clone());
    let (key, words) = store(app, move |tx| {
        let now = ops::now();
        let given = Given::of(tx, &body(tx, &b, &args, None)?, now)?;
        let (_, key) = schedules::add(tx, &b.user, &given, now)?;
        Ok((key, given.as_schedule().words()))
    }).await?;
    crate::tasks::schedulesapi::heard(app);
    Ok(format!("{key} is made: {words}. Its card names you as its author; run_schedule runs it now."))
}

pub async fn update(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let (b, args) = (boss.clone(), args.clone());
    let key = store(app, move |tx| {
        let now = ops::now();
        let was = of_project(tx, &b, &args)?;
        let given = Given::of(tx, &body(tx, &b, &args, Some(&was))?, now)?;
        if schedules::change(tx, &b.user, &was.id, &given, now)? {
            crate::dispatcher::fire::called_off(tx, &was.id, "Its schedule was paused.")?;
        }
        Ok(was.key)
    }).await?;
    crate::tasks::api::told(app, "");
    crate::tasks::schedulesapi::heard(app);
    Ok(format!("{key} is changed: its next firing takes it."))
}

pub async fn delete(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let (b, args) = (boss.clone(), args.clone());
    let key = store(app, move |tx| {
        let was = of_project(tx, &b, &args)?;
        schedules::remove(tx, &was.id, ops::now())?;
        crate::dispatcher::fire::called_off(tx, &was.id, "Its schedule was deleted.")?;
        Ok(was.key)
    }).await?;
    crate::tasks::api::told(app, "");
    crate::tasks::schedulesapi::heard(app);
    Ok(format!("{key} is deleted; its tasks and firings stay."))
}

pub async fn run_now(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let (b, args) = (boss.clone(), args.clone());
    let s = store(app, move |tx| of_project(tx, &b, &args)).await?;
    let task = crate::dispatcher::fire::now(app, &s.id, &boss.name).await?;
    let wanted = task.clone();
    let status = store(app, move |tx| Ok(ops::get(tx, &wanted)?.status)).await?;
    Ok(format!("{} ran now: its task {task} is {status}. list_runs shows its run; get_task its result.", s.key))
}
