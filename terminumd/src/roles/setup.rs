// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A manager's hand on its project's setup (`terminum-agents.md` AC-283): what it is and lacks, its letters,
//! language, template, agents at once and memory limits by the project window's own rules, and the Supervisor's
//! charter while the Supervisor is off (`terminum-supervisor.md` AC-284). The human's switches have no tool here.

use rusqlite::Transaction;
use serde_json::{json, Value};

use super::{arg, number, Boss, DATA};
use crate::app::Shared;
use crate::tasks::api::store;
use crate::tasks::{agents, auto, closeout, limits, ops, schedules, slugs, template};

/// Where the human closes a gap: the project window's tab.
const TAB: &str = "the human, in the project window's";

/// One gap of `setup`'s: what lacks, who closes it and how.
fn gap(what: &str, how: &str) -> Value {
    json!({"gap": what, "how": how})
}

/// Whether a local folder is a linked git worktree: its `.git` a file naming another checkout's.
fn linked_worktree(dir: &str) -> bool {
    std::path::Path::new(dir).join(".git").is_file()
}

/// The project's setup, the Supervisor's part included, and what it still lacks.
pub(crate) fn setup(tx: &Transaction, project: &str) -> Result<Value, String> {
    let (name, dir, host, prefix, agent_limit): (String, String, Option<String>, Option<String>, Option<i64>) = tx.query_row(
        "SELECT name, dir, host, prefix, agent_limit FROM projects WHERE id = ?1", [project],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).map_err(ops::sql)?;
    let memory = limits::limits(tx)?.into_iter().find(|(p, ..)| p == project).map(|(_, warn, end)| json!({"warn_gib": warn, "end_gib": end}));
    let pool: Vec<String> = agents::of_project(tx, project)?.into_iter().map(|a| a.name).collect();
    let routines = schedules::all(tx)?.into_iter().filter(|s| s.project == project).count();
    let auto = auto::setting(tx, project)?;
    let integration: bool = tx.query_row("SELECT coalesce((SELECT is_on FROM project_integration WHERE project = ?1), 0)",
        [project], |r| r.get(0)).map_err(ops::sql)?;
    let supervisor = crate::supervisor::charter::setup(tx, project)?;
    let on = supervisor["enabled"] == true;
    let mut gaps = Vec::new();
    if host.is_none() && linked_worktree(&dir) {
        gaps.push(gap("the folder is a linked git worktree, so the Supervisor's git readings follow its branch alone",
            &format!("{TAB} General tab: the primary checkout's folder")));
    }
    if pool.is_empty() {
        gaps.push(gap("the pool is empty", "you: create_agent"));
    }
    if routines == 0 {
        gaps.push(gap("no routines", "you: create_schedule"));
    }
    if supervisor["charter"]["mission"].as_str().is_none_or(str::is_empty) {
        gaps.push(gap("the Supervisor's charter is empty", if on { "the human, in the project window's Supervisor tab (it is on)" } else { "you: set_supervisor_charter" }));
    }
    let unset: Vec<&str> = ["small", "medium", "strong", "independent"].into_iter().filter(|t| supervisor["tiers"][t].is_null()).collect();
    if !unset.is_empty() {
        gaps.push(gap(&format!("Supervisor tiers unset: {}", unset.join(", ")), &format!("{TAB} Supervisor tab, from the pool's agents")));
    }
    if !on {
        gaps.push(gap("the Supervisor is off", &format!("{TAB} Supervisor tab: Level, then Enabled")));
    }
    if !auto.on {
        gaps.push(gap("Auto-Dispatcher is off: tasks start only when run", &format!("{TAB} Auto-Dispatch tab, if the user wants it")));
    }
    Ok(json!({
        "project": {"name": name, "folder": dir, "host": host, "prefix": prefix, "language": closeout::language_of(tx, project)?,
            "template": template::of_project(tx, project)?, "agents_at_once": agent_limit, "memory_limits": memory},
        "pool": pool, "routines": routines,
        "auto_dispatch": auto.shown(), "integration_on": integration,
        "supervisor": supervisor,
        "gaps": gaps,
    }))
}

pub async fn get(app: &Shared, boss: &Boss) -> Result<String, String> {
    let project = boss.project.clone();
    let shown = store(app, move |tx| setup(tx, &project)).await?;
    Ok(format!("{DATA}\n{shown}"))
}

/// A GiB figure as an argument says it: absent, empty or a number.
fn gib(args: &Value, key: &str) -> Result<Option<Option<f64>>, String> {
    match args[key].as_str().map(str::trim) {
        None => Ok(None),
        Some("") => Ok(Some(None)),
        Some(s) => s.parse::<f64>().ok().filter(|n| n.is_finite()).map(|n| Some(Some(n))).ok_or_else(|| format!("{key}: a number of GiB, 0 for off, or empty")),
    }
}

/// Applies the fields `args` names, each by the project window's own rule, in one transaction: every one, or none.
pub(crate) fn update(tx: &Transaction, project: &str, args: &Value) -> Result<Vec<&'static str>, String> {
    let mut changed = Vec::new();
    if let Some(prefix) = args["prefix"].as_str() {
        slugs::set_prefix(tx, project, prefix)?;
        changed.push("ID letters");
    }
    if let Some(language) = args["language"].as_str() {
        closeout::set_language(tx, project, language)?;
        changed.push("language");
    }
    if let Some(text) = args["template"].as_str() {
        template::set(tx, project, text)?;
        changed.push("task template");
    }
    if args["agents_at_once"].is_string() {
        limits::keep_agent_limit(tx, project, number(args, "agents_at_once", 1..=agents::MOST_AT_ONCE)?)?;
        changed.push("agents at once");
    }
    match (gib(args, "memory_warn_gib")?, gib(args, "memory_end_gib")?) {
        (None, None) => {}
        (Some(None), Some(None)) => { limits::keep_limits(tx, project, None)?; changed.push("memory limits"); }
        (Some(Some(warn)), Some(Some(end))) => { limits::keep_limits(tx, project, Some((warn, end)))?; changed.push("memory limits"); }
        _ => return Err("memory_warn_gib and memory_end_gib go together: both numbers, or both empty for the global ones".into()),
    }
    if changed.is_empty() {
        return Err("Name a field: prefix, language, template, agents_at_once, memory_warn_gib with memory_end_gib.".into());
    }
    Ok(changed)
}

pub async fn change(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let (project, args) = (boss.project.clone(), args.clone());
    let changed = store(app, move |tx| update(tx, &project, &args)).await?;
    crate::tasks::api::told(app, "");                  // every card's ID may have changed with the letters
    crate::page::projects::tell(app);
    Ok(format!("Changed: {}.", changed.join(", ")))
}

pub async fn charter(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let revision = number(args, "revision", 0..=i64::MAX)?.ok_or("revision is required: get_project_setup shows it")?;
    let (project, text) = (boss.project.clone(), arg(args, "charter")?.to_string());
    let revision = store(app, move |tx| crate::supervisor::charter::set_by_manager(tx, &project, revision, &text)).await?;
    app.supervisor.wake();
    Ok(format!("The Supervisor's charter is saved at revision {revision}; the owner reviews it in the Supervisor tab before Enabled."))
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
