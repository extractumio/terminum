// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A manager's hand on its project's pool of agents (`terminum-agents.md` AC-180), never another project's: the
//! rules are the board's own (tasks/agents.rs): a name once, a command of one line, slots its schedules keep.

use serde_json::{json, Value};

use super::tasks::agent_named;
use super::{arg, number, text, Boss, DATA};
use crate::app::Shared;
use crate::tasks::agents::{self, MOST_AT_ONCE};
use crate::tasks::api::store;
use crate::tasks::ops;

pub async fn list(app: &Shared, boss: &Boss) -> Result<String, String> {
    let project = boss.project.clone();
    let pool = store(app, move |tx| agents::listed(tx, ops::now(), Some(&project))).await?;
    Ok(format!("{DATA}\n{}", json!(pool)))
}

/// A limit as its argument says it: absent leaves `was`, empty clears, else million tokens.
fn limit(args: &Value, key: &str, was: Option<f64>) -> Result<Value, String> {
    match args[key].as_str().map(str::trim) {
        None => Ok(json!(was)),
        Some("") => Ok(Value::Null),
        Some(s) => s.parse::<f64>().map(|n| json!(n)).map_err(|_| format!("{key}: a number of million tokens, or empty")),
    }
}

/// The body the board's own rules judge: `args` over what the agent `was`, in the manager's project.
fn body(boss: &Boss, args: &Value, was: Option<&agents::Agent>) -> Result<Value, String> {
    Ok(json!({
        "project": boss.project,
        "name": text(args, "name", was.map(|a| a.name.as_str())), "kind": text(args, "kind", was.map(|a| a.kind.as_str())),
        "command": text(args, "command", was.map(|a| a.command.as_str())), "effort": text(args, "effort", was.map(|a| a.effort.as_str())),
        "at_once": number(args, "at_once", 1..=MOST_AT_ONCE)?.or(was.map(|a| a.at_once)),
        "week_limit": limit(args, "week_limit", was.and_then(|a| a.week_limit))?, "month_limit": limit(args, "month_limit", was.and_then(|a| a.month_limit))?,
    }))
}

pub async fn create(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let given = agents::checked(&body(boss, args, None)?)?;
    let name = arg(args, "name")?.to_string();
    store(app, move |tx| agents::add(tx, &given)).await?;
    crate::page::projects::tell(app);
    Ok(format!("{name} is in your project's pool: run_task starts tasks on it."))
}

pub async fn update(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let (named, args, boss) = (arg(args, "agent")?.to_string(), args.clone(), boss.clone());
    let name = store(app, move |tx| {
        let was = agent_named(tx, &boss.project, &named)?;
        let given = agents::checked(&body(&boss, &args, Some(&was))?)?;
        agents::change(tx, &was.id, &given)?;
        Ok(args["name"].as_str().map(str::to_string).unwrap_or(was.name))
    }).await?;
    crate::page::projects::tell(app);
    Ok(format!("{name} is changed: its next start takes it."))
}

pub async fn delete(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let (named, project) = (arg(args, "agent")?.to_string(), boss.project.clone());
    let name = store(app, move |tx| {
        let was = agent_named(tx, &project, &named)?;
        agents::remove(tx, &was.id).map(|()| was.name)
    }).await?;
    crate::page::projects::tell(app);
    Ok(format!("{name} is out of your project's pool."))
}
