// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A red batch's integration agent (`terminum-integration.md` AC-194): its integration task I_<ID> runs on the
//! project's integration agent, in a worktree of its own at the candidate, told the project's Prompt with the
//! batch's facts filled in; it decides by three tools alone, only for its own batch: return the tasks that caused
//! it, check again, or add a fix of its own. terminumd still alone builds, checks and publishes.

use std::path::Path;

use rusqlite::{OptionalExtension, Transaction};
use serde_json::Value;

use super::git::{self, short};
use super::store::{self as bs, Batch};
use crate::tasks::integration::{self, DEFAULT_PROMPT};
use crate::tasks::ops::{self, sql, INTEGRATION};
use crate::tasks::store::{Store, DISPATCHER};
use crate::tasks::tools::{arg, author, Who};

/// The analysis' sections an agent's return gives (AC-194).
pub const ANALYSIS: [&str; 4] = ["Failing step", "Evidence", "Commit and lines", "What to fix"];

/// The batch whose integration task is `key`, if one.
pub fn batch_of_task(tx: &Transaction, key: &str) -> Result<Option<Batch>, String> {
    let id: Option<i64> = tx.query_row("SELECT id FROM integration_batches WHERE task = ?1", [key], |r| r.get(0)).optional().map_err(sql)?;
    id.map(|i| bs::by_id(tx, i)).transpose()
}

/// Where the integration task's agent works: a worktree of its own at the candidate (§9), made by the engine.
pub fn tree_of_task(tx: &Transaction, key: &str) -> Result<Option<String>, String> {
    Ok(tx.query_row("SELECT agent_tree FROM integration_batches WHERE task = ?1", [key], |r| r.get::<_, Option<String>>(0)).optional().map_err(sql)?.flatten())
}

/// The red batches whose agent is to start: (batch, integration task's ID, agent, budget minutes).
pub fn wanted(tx: &Transaction) -> Result<Vec<(i64, String, String, i64)>, String> {
    ops::rows(tx, "SELECT b.id, t.slug, i.agent, i.agent_budget_minutes FROM integration_batches b JOIN tasks t ON t.id = b.task
                   JOIN project_integration i ON i.project = b.project
                   WHERE b.state = 'red' AND b.agent_state = 'wanted' AND i.agent IS NOT NULL", [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
}

pub fn set_agent_state(tx: &Transaction, batch: i64, state: Option<&str>) -> Result<(), String> {
    tx.execute("UPDATE integration_batches SET agent_state = ?2 WHERE id = ?1", rusqlite::params![batch, state]).map(drop).map_err(sql)
}

/// The project's Prompt with the batch's facts filled in, never a task's title, text or comment (§9 Security).
pub fn prompt_of(tx: &Transaction, key: &str) -> Result<Option<String>, String> {
    let Some(b) = batch_of_task(tx, key)? else { return Ok(None) };
    let s = integration::read(tx, &b.project)?;
    let (name, _, _) = ops::project_place(tx, &b.project)?;
    let (why, started, ended): (Option<String>, Option<i64>, Option<i64>) = tx.query_row("SELECT why, started, ended FROM integration_rounds WHERE batch = ?1 AND round = ?2",
        (b.id, b.round), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(sql)?.unwrap_or((None, None, None));
    let mut tasks = Vec::new();
    for k in &b.tasks {
        let commit: Option<String> = tx.query_row("SELECT commit_sha FROM integration_entries WHERE task = ?1", [k], |r| r.get(0)).optional().map_err(sql)?;
        if let (Ok(id), Some(c)) = (bs::slug(tx, k), commit) {
            tasks.push(format!("{id} at {}", short(&c)));
        }
    }
    let home = crate::config::tm_home();
    let tree = tree_of_task(tx, key)?.unwrap_or_default();
    let log = integration::tree_in(&home, &b.project).with_file_name(format!("batch-{}.{}.check.log", b.number, b.round));
    let minutes = match (started, ended) { (Some(a), Some(z)) => format!("{} min", (z - a).max(0) / 60), _ => "an unknown time".into() };
    let what = why.clone().unwrap_or_else(|| "its check was red".into());
    let template = if s.prompt.trim().is_empty() { DEFAULT_PROMPT } else { s.prompt.as_str() };
    let filled = template
        .replace("{task}", &bs::slug(tx, key)?).replace("{batch}", &b.number.to_string()).replace("{project}", &name)
        .replace("{round}", &b.round.to_string()).replace("{what failed}", &what)
        .replace("{candidate}", short(b.candidate.as_deref().unwrap_or_default())).replace("{target}", &s.target).replace("{base}", short(&b.base))
        .replace("{tasks, each with its commit and files}", &tasks.join(", ")).replace("{check}", &s.check)
        .replace("{how it ended}", &what).replace("{time}", &minutes).replace("{log}", &log.display().to_string()).replace("{tree}", &tree);
    Ok(Some(format!("{filled}\n\nYou have {} minutes for this. Your task {} ends with its batch: never call finish_task on it; decide by the tools above, then end your turn.",
                    s.agent_budget_minutes, bs::slug(tx, key)?)))
}

/// The batch the calling session's run decides for: its run holds the batch's integration task.
fn own_batch(tx: &Transaction, who: &Who) -> Result<Batch, String> {
    let run = crate::dispatcher::runs::in_pane(tx, &who.key)?;
    for id in run.map(|r| r.ids).unwrap_or_default() {
        if let Ok(key) = ops::key_of(tx, &id) && let Some(b) = batch_of_task(tx, &key)? {
            if b.state != "red" {
                return Err(format!("Batch {} is {}: nothing to decide now.", b.number, b.state));
            }
            return Ok(b);
        }
    }
    Err("Only a batch's integration task decides for it.".into())
}

/// Its turn's decision made: the integration task back in INTEGRATION, the batch worked on again.
fn back_to_work(tx: &Transaction, b: &Batch, me: &str, at: i64) -> Result<(), String> {
    set_agent_state(tx, b.id, None)?;
    let id = bs::slug(tx, b.task.as_deref().unwrap_or_default())?;
    let t = ops::get(tx, &id)?;
    if t.status != INTEGRATION {
        ops::place(tx, me, &t, INTEGRATION, at)?;
    }
    Ok(())
}

/// integration_return {task, analysis}: a task of the batch back to its agent with the analysis, redacted; the
/// batch stays red until integration_recheck, so several may go back for one more round.
pub fn give_back(tx: &Transaction, who: &Who, args: &Value, at: i64) -> Result<String, String> {
    let b = own_batch(tx, who)?;
    let task = ops::get(tx, arg(args, "task")?)?;
    if !b.tasks.contains(&task.key) || b.task.as_deref() == Some(task.key.as_str()) {
        return Err(format!("{} is not a task of batch {}.", task.id, b.number));
    }
    let analysis = crate::sent::redact(&crate::plans::rules::clean(arg(args, "analysis")?));
    let lacks: Vec<&str> = ANALYSIS.iter().copied().filter(|s| !analysis.contains(&format!("{s}:")) && !analysis.contains(&format!("## {s}"))).collect();
    if !lacks.is_empty() {
        return Err(format!("The analysis lacks {}: each section as \"Name:\" or a heading.", lacks.join(", ")));
    }
    let i_id = bs::slug(tx, b.task.as_deref().unwrap_or_default())?;
    tx.execute("INSERT OR IGNORE INTO integration_returns (task, commit_sha, why, at) SELECT task, commit_sha, 'red', ?2 FROM integration_entries WHERE task = ?1", (&task.key, at)).map_err(sql)?;
    bs::give_back(tx, &task.key, &format!("{} returned: red check. Batch {}'s check was red and {i_id} found its cause in {}. Its analysis:\n\n{analysis}", task.id, b.number, task.id), at)?;
    Ok(format!("{} went back to its agent. Call integration_recheck when you have returned every task that caused it.", task.id))
}

/// integration_recheck {why?}: the batch built anew without what was returned and with a fix added, and checked;
/// with neither, the same candidate checked again once, why given, for a flaky test or a cause mended outside.
pub fn recheck(tx: &Transaction, who: &Who, args: &Value, at: i64) -> Result<String, String> {
    let b = own_batch(tx, who)?;
    let me = author(tx, who)?;
    let checked: Vec<String> = tx.query_row("SELECT tasks FROM integration_rounds WHERE batch = ?1 AND round = ?2", (b.id, b.round), |r| r.get::<_, String>(0))
        .optional().map_err(sql)?.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if b.tasks != checked {
        tx.execute("UPDATE integration_batches SET state = 'rebuild', pane = NULL, unchanged_rechecks = 0 WHERE id = ?1", [b.id]).map_err(sql)?;
        back_to_work(tx, &b, &me, at)?;
        return Ok(format!("Batch {} is built anew from what is left in it and checked: round {}.", b.number, b.round + 1));
    }
    let unchanged: i64 = tx.query_row("SELECT unchanged_rechecks FROM integration_batches WHERE id = ?1", [b.id], |r| r.get(0)).map_err(sql)?;
    if unchanged >= 1 {
        return Err(format!("Batch {} was checked again unchanged once: return a task, add a fix, or ask a human.", b.number));
    }
    let why = args["why"].as_str().map(str::trim).filter(|w| !w.is_empty()).ok_or("Say why it is checked again unchanged: the flaky test, or what you mended.")?;
    tx.execute("UPDATE integration_batches SET unchanged_rechecks = unchanged_rechecks + 1, state = 'checking', pane = NULL, round = round + 1 WHERE id = ?1", [b.id]).map_err(sql)?;
    tx.execute("INSERT INTO integration_rounds (batch, round, tasks, base_sha, candidate_sha, result, why) VALUES (?1, ?2, ?3, ?4, ?5, 'built', ?6)",
               rusqlite::params![b.id, b.round + 1, serde_json::to_string(&b.tasks).unwrap_or_default(), b.base, b.candidate, format!("checked again unchanged: {why}")]).map_err(sql)?;
    back_to_work(tx, &b, &me, at)?;
    Ok(format!("Batch {} is checked again unchanged: round {}.", b.number, b.round + 1))
}

/// integration_add_fix {commit}: the agent's own commit, from its worktree, put alone on the batch's target (a
/// cherry-pick, outside the store's transaction), so it carries no task of the batch and joins the candidate first;
/// then integration_recheck builds it.
pub fn add_fix(store: &Store, who: &Who, args: &Value, at: i64) -> Result<String, String> {
    let commit = arg(args, "commit")?.to_string();
    // a commit as git names it, never an option or a range: it reaches git's arguments
    if !(7..=64).contains(&commit.len()) || !commit.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("A commit is its hash, 7 to 64 hex digits, from your worktree.".into());
    }
    let (b, tree) = store.with(|tx| {
        let b = own_batch(tx, who)?;
        let tree = tree_of_task(tx, b.task.as_deref().unwrap_or_default())?.ok_or_else(|| format!("Batch {} has no worktree of its agent.", b.number))?;
        Ok((b, tree))
    })?;
    let fix = alone_on(Path::new(&tree), &commit, &b.base)?;
    store.with(|tx| {
        let b = own_batch(tx, who)?;
        let key = b.task.clone().unwrap_or_default();
        let me = author(tx, who)?;
        tx.execute("INSERT INTO integration_entries (task, project, commit_sha, result, author, queued_at, batch) VALUES (?1, ?2, ?3, 'fix', ?4, 0, ?5)
                    ON CONFLICT (task) DO UPDATE SET commit_sha = ?3", rusqlite::params![key, b.project, fix, me, b.id]).map_err(sql)?;
        let mut tasks = vec![key.clone()];
        tasks.extend(b.tasks.iter().filter(|k| **k != key).cloned());
        tx.execute("UPDATE integration_batches SET tasks = ?2 WHERE id = ?1", (b.id, serde_json::to_string(&tasks).unwrap_or_default())).map_err(sql)?;
        ops::comment(tx, DISPATCHER, &bs::slug(tx, &key)?, &format!("Its agent added a fix of its own: {}, on the target as {}. It joins the candidate first.", short(&commit), short(&fix)), at)?;
        Ok(format!("{} joins batch {}'s candidate first. Call integration_recheck to build and check it.", short(&commit), b.number))
    })
}

/// Commit `commit`'s own change picked onto `base` in the agent's worktree, which then stands where it stood.
fn alone_on(tree: &Path, commit: &str, base: &str) -> Result<String, String> {
    let full = git::run(tree, &["rev-parse", "--verify", "--quiet", &format!("{commit}^{{commit}}")]).map_err(|_| format!("{commit} is no commit in your worktree."))?;
    let back = git::run(tree, &["rev-parse", "HEAD"])?;
    git::run(tree, &["checkout", "--quiet", "--detach", base]).map_err(|e| format!("Your worktree cannot leave {}: {e}. Commit or drop your changes first.", short(&back)))?;
    let picked = git::run(tree, &["-c", "core.hooksPath=/dev/null", "cherry-pick", &full]).and_then(|_| git::run(tree, &["rev-parse", "HEAD"]));
    if picked.is_err() {
        let _ = git::run(tree, &["cherry-pick", "--abort"]);
    }
    let restored = git::run(tree, &["checkout", "--quiet", "--detach", &back]);
    let picked = picked.map_err(|e| format!("{} does not apply alone to the target at {}: {e}", short(&full), short(base)))?;
    restored.map(|_| picked)
}

/// finish_task on a batch's integration task: refused while its batch has a red round undecided.
pub fn finish_refused(tx: &Transaction, key: &str) -> Result<Option<String>, String> {
    Ok(batch_of_task(tx, key)?.filter(|b| !["ended", "given up"].contains(&b.state.as_str()))
        .map(|b| format!("{} ends when its batch does: return tasks, check again, or ask a human.", bs::slug(tx, key).unwrap_or_default()).replace("batch does", &format!("batch {} does", b.number))))
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
