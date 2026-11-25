// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A manager's hand on its project's setup (`terminum-agents.md` AC-283): what it is and lacks, its letters,
//! language, template, agents at once and memory limits by the project window's own rules, and the Supervisor's
//! charter while the Supervisor is off (`terminum-supervisor.md` AC-284). The human's switches have no tool here.

use rusqlite::Transaction;
use serde_json::{json, Value};

use super::{arg, number, Boss, DATA};
use crate::app::Shared;
use crate::tasks::api::store;
use crate::tasks::{agents, auto, closeout, integration, limits, ops, slugs, template};

/// Where the human closes a gap: the project window's tab.
const TAB: &str = "the human, in the project window's";

/// One gap of `setup`'s: what lacks, who closes it and how.
fn gap(what: &str, how: &str) -> Value {
    json!({"gap": what, "how": how})
}

/// Whether a local folder is a linked git worktree: its `.git` a file naming a checkout's `worktrees/` entry (a
/// submodule's names `modules/`).
fn linked_worktree(dir: &str) -> bool {
    std::fs::read_to_string(std::path::Path::new(dir).join(".git"))
        .is_ok_and(|text| text.strip_prefix("gitdir:").is_some_and(|to| to.trim().rsplit('/').nth(1) == Some("worktrees")))
}

/// The project's setup, the Supervisor's part included, and what it still lacks; the folder's own look is
/// `folder_gap`'s, outside the store's turn.
pub(crate) fn setup(tx: &Transaction, project: &str) -> Result<Value, String> {
    let (name, host, dir) = ops::project_place(tx, project)?;
    let prefix: Option<String> = tx.query_row("SELECT prefix FROM projects WHERE id = ?1", [project], |r| r.get(0)).map_err(ops::sql)?;
    let memory = limits::of_project(tx, project)?.map(|(warn, end)| json!({"warn_gib": warn, "end_gib": end}));
    let pool: Vec<String> = agents::of_project(tx, project)?.into_iter().map(|a| a.name).collect();
    let routines: i64 = tx.query_row("SELECT count(*) FROM schedules WHERE project = ?1 AND deleted_at IS NULL", [project], |r| r.get(0))
        .map_err(ops::sql)?;
    let auto = auto::setting(tx, project)?;
    let (supervisor, supervising) = crate::supervisor::charter::setup(tx, project, TAB)?;
    let mut gaps = Vec::new();
    if pool.is_empty() {
        gaps.push(gap("the pool is empty", "you: create_agent"));
    }
    if routines == 0 {
        gaps.push(gap("no routines", "you: create_schedule"));
    }
    gaps.extend(supervising.iter().map(|(what, how)| gap(what, how)));
    if !auto.on {
        gaps.push(gap("Auto-Dispatcher is off: tasks start only when run", &format!("{TAB} Auto-Dispatch tab, if the user wants it")));
    }
    Ok(json!({
        "project": {"name": name, "folder": dir, "host": host, "prefix": prefix, "language": closeout::language_of(tx, project)?,
            "template": template::of_project(tx, project)?, "agents_at_once": limits::agent_limit(tx, project)?, "memory_limits": memory},
        "pool": pool, "routines": routines,
        "auto_dispatch": auto.shown(), "integration_on": integration::read(tx, project)?.on,
        "supervisor": supervisor,
        "gaps": gaps,
    }))
}

/// `shown`'s gaps with the folder's: a local folder that is a linked worktree. It reads the disk, so never in a store turn.
fn folder_gap(shown: &mut Value) {
    let local = shown["project"]["host"].is_null();
    if local && shown["project"]["folder"].as_str().is_some_and(linked_worktree) && let Some(gaps) = shown["gaps"].as_array_mut() {
        gaps.insert(0, gap("the folder is a linked git worktree, so the Supervisor's git readings follow its branch alone",
            &format!("{TAB} General tab: the primary checkout's folder")));
    }
}

pub async fn get(app: &Shared, boss: &Boss) -> Result<String, String> {
    let project = boss.project.clone();
    let mut shown = store(app, move |tx| setup(tx, &project)).await?;
    let shown = crate::http::off(move || { folder_gap(&mut shown); Ok::<_, String>(shown) }).await?;
    Ok(format!("{DATA}\n{shown}"))
}

/// The fields `update` takes, as update_project_setup names them.
const FIELDS: [&str; 6] = ["prefix", "language", "template", "agents_at_once", "memory_warn_gib", "memory_end_gib"];
const NAME: &str = "Name a field: prefix, language, template, agents_at_once, memory_warn_gib with memory_end_gib.";

/// A GiB figure an argument holds.
fn gib(key: &str, text: &str) -> Result<f64, String> {
    text.parse::<f64>().ok().filter(|n| n.is_finite()).ok_or_else(|| format!("{key}: a number of GiB, 0 for off, or empty"))
}

/// Applies the fields `args` names, each by the project window's own rule, in one transaction: every one, or none.
pub(crate) fn update(tx: &Transaction, project: &str, args: &Value) -> Result<Vec<&'static str>, String> {
    if let Some(other) = args.as_object().and_then(|given| given.keys().find(|key| !FIELDS.contains(&key.as_str()))) {
        return Err(format!("{other}: not a setup field; nothing changed. {NAME}"));
    }
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
    let given = |key| args[key].as_str().map(str::trim);
    match (given("memory_warn_gib"), given("memory_end_gib")) {
        (None, None) => {}
        (Some(warn), Some(end)) if warn.is_empty() == end.is_empty() => {
            let limits = (!warn.is_empty()).then(|| Ok::<_, String>((gib("memory_warn_gib", warn)?, gib("memory_end_gib", end)?))).transpose()?;
            limits::keep_limits(tx, project, limits)?;
            changed.push("memory limits");
        }
        _ => return Err("memory_warn_gib and memory_end_gib go together: both numbers, or both empty for the global ones".into()),
    }
    if changed.is_empty() {
        return Err(NAME.into());
    }
    Ok(changed)
}

pub async fn change(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let (project, args) = (boss.project.clone(), args.clone());
    let changed = store(app, move |tx| update(tx, &project, &args)).await?;
    if changed.iter().any(|c| matches!(*c, "ID letters" | "language" | "task template")) {
        crate::tasks::api::told(app, "");              // every card's ID may have changed with the letters
    }
    crate::page::projects::tell(app);
    Ok(format!("Changed: {}.", changed.join(", ")))
}

pub async fn charter(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let revision = number(args, "revision", 0..=i64::MAX)?.ok_or("revision is required: get_project_setup shows it")?;
    let (project, text) = (boss.project.clone(), arg(args, "charter")?.to_string());
    // the Supervisor is off: nothing to wake, its tab reads the settings when opened
    let revision = store(app, move |tx| crate::supervisor::charter::set_by_manager(tx, &project, revision, &text)).await?;
    Ok(format!("The Supervisor's charter is saved at revision {revision}; the owner reviews it in the Supervisor tab before Enabled."))
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
