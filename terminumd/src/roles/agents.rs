// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A manager's hand on the pool of agents (`terminum-agents.md` AC-176): the pool is every project's, and the
//! rules are the board's own (tasks/agents.rs): a name once, a command of one line, slots its schedules keep.

use serde_json::{json, Value};

use super::tasks::agent_named;
use super::{arg, number, DATA};
use crate::app::Shared;
use crate::tasks::agents::{self, MOST_AT_ONCE};
use crate::tasks::api::store;
use crate::tasks::ops;

pub async fn list(app: &Shared) -> Result<String, String> {
    let pool = store(app, |tx| agents::listed(tx, ops::now())).await?;
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

/// The body the board's own rules judge: `args` over what the agent `was`.
fn body(args: &Value, was: Option<&agents::Agent>) -> Result<Value, String> {
    let text = |key: &str, was: Option<&str>| args[key].as_str().map(str::to_string).or_else(|| was.map(str::to_string)).unwrap_or_default();
    Ok(json!({
        "name": text("name", was.map(|a| a.name.as_str())), "kind": text("kind", was.map(|a| a.kind.as_str())),
        "command": text("command", was.map(|a| a.command.as_str())), "effort": text("effort", was.map(|a| a.effort.as_str())),
        "at_once": number(args, "at_once", 1..=MOST_AT_ONCE)?.or(was.map(|a| a.at_once)),
        "week_limit": limit(args, "week_limit", was.and_then(|a| a.week_limit))?, "month_limit": limit(args, "month_limit", was.and_then(|a| a.month_limit))?,
    }))
}

pub async fn create(app: &Shared, args: &Value) -> Result<String, String> {
    let given = agents::checked(&body(args, None)?)?;
    let name = arg(args, "name")?.to_string();
    store(app, move |tx| agents::add(tx, &given)).await?;
    crate::page::projects::tell(app);
    Ok(format!("{name} is in the pool: run_task starts tasks on it."))
}

pub async fn update(app: &Shared, args: &Value) -> Result<String, String> {
    let (named, args) = (arg(args, "agent")?.to_string(), args.clone());
    let name = store(app, move |tx| {
        let was = agent_named(tx, &named)?;
        let given = agents::checked(&body(&args, Some(&was))?)?;
        agents::change(tx, &was.id, &given)?;
        Ok(args["name"].as_str().map(str::to_string).unwrap_or(was.name))
    }).await?;
    crate::page::projects::tell(app);
    Ok(format!("{name} is changed: its next start takes it."))
}

pub async fn delete(app: &Shared, args: &Value) -> Result<String, String> {
    let named = arg(args, "agent")?.to_string();
    let name = store(app, move |tx| {
        let was = agent_named(tx, &named)?;
        agents::remove(tx, &was.id).map(|()| was.name)
    }).await?;
    crate::page::projects::tell(app);
    Ok(format!("{name} is out of the pool."))
}
