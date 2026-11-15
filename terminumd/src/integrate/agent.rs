// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A red batch's integration agent (`terminum-integration.md` AC-194): its integration task I_<ID> runs on the
//! project's integration agent, in a worktree of its own at the candidate, told the project's Prompt with the
//! batch's facts filled in; it decides by three tools alone, only for its own batch: return the tasks that caused
//! it, check again, or add a fix of its own. terminumd still alone builds, checks and publishes.

use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, Transaction};
use serde_json::Value;

use super::git::{self, short};
use super::store::{self as bs, Batch};
use crate::tasks::integration::{self, DEFAULT_PROMPT};
use crate::tasks::ops::{self, sql};
use crate::tasks::store::{Store, DISPATCHER};
use crate::tasks::tools::{arg, Who};

/// The analysis' sections an agent's return gives (AC-194).
pub const ANALYSIS: [&str; 4] = ["Failing step", "Evidence", "Commit and lines", "What to fix"];

/// Where the integration task's agent works: its batch's worktree of its own at the candidate (§9), made by the engine.
pub fn tree_of_task(tx: &Transaction, key: &str) -> Result<Option<String>, String> {
    Ok(bs::by_task(tx, key)?.and_then(|b| b.agent_tree))
}

/// The red batches whose agent is to start, taken: each no longer wanted, its integration task's budget the
/// project's. (batch, the task's ID, the agent).
pub fn take_wanted(tx: &Transaction, at: i64) -> Result<Vec<(i64, String, String)>, String> {
    let wanted: Vec<(i64, String, String, i64)> = ops::rows(tx, "SELECT b.id, t.slug, i.agent, i.agent_budget_minutes FROM integration_batches b JOIN tasks t ON t.id = b.task
                   JOIN project_integration i ON i.project = b.project
                   WHERE b.state = 'red' AND b.agent_wanted = 1 AND i.is_on = 1 AND i.agent IS NOT NULL", [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    for (batch, id, _, minutes) in &wanted {
        tx.execute("UPDATE integration_batches SET agent_wanted = 0 WHERE id = ?1", [batch]).map_err(sql)?;
        ops::set_budget(tx, DISPATCHER, id, Some(*minutes), at)?;
    }
    Ok(wanted.into_iter().map(|(b, id, agent, _)| (b, id, agent)).collect())
}

/// The project's Prompt with the batch's facts filled in, never a task's title, text or comment (§9 Security).
pub fn prompt_of(tx: &Transaction, key: &str) -> Result<Option<String>, String> {
    let Some(b) = bs::by_task(tx, key)? else { return Ok(None) };
    let s = integration::read(tx, &b.project)?;
    let (name, _, _) = ops::project_place(tx, &b.project)?;
    let id = bs::slug(tx, key)?;
    let (why, started, ended): (Option<String>, Option<i64>, Option<i64>) = tx.query_row("SELECT why, started, ended FROM integration_rounds WHERE batch = ?1 AND round = ?2",
        (b.id, b.round), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(sql)?.unwrap_or((None, None, None));
    let tasks: Vec<String> = bs::queued_of(tx, &b)?.iter().map(|q| format!("{} at {}", q.id, short(&q.commit))).collect();
    let log = integration::log_in(&crate::config::tm_home(), &b.project, b.number, b.round, "check");
    let minutes = match (started, ended) { (Some(a), Some(z)) => format!("{} min", (z - a).max(0) / 60), _ => "an unknown time".into() };
    let what = why.unwrap_or_else(|| "its check was red".into());
    let facts = [("{task}", id.clone()), ("{batch}", b.number.to_string()), ("{project}", name), ("{round}", b.round.to_string()),
        ("{what failed}", what.clone()), ("{how it ended}", what), ("{candidate}", short(b.candidate.as_deref().unwrap_or_default()).to_string()),
        ("{target}", s.target.clone()), ("{base}", short(&b.base).to_string()), ("{tasks, each with its commit and files}", tasks.join(", ")),
        ("{check}", s.check.clone()), ("{time}", minutes), ("{log}", log.display().to_string()), ("{tree}", b.agent_tree.clone().unwrap_or_default())];
    let template = if s.prompt.trim().is_empty() { DEFAULT_PROMPT } else { s.prompt.as_str() };
    let filled = facts.iter().fold(template.to_string(), |t, (k, v)| t.replace(k, v));
    Ok(Some(format!("{filled}\n\nYou have {} minutes for this. Your task {id} ends with its batch: never call finish_task on it; decide by the tools above, then end your turn.",
                    s.agent_budget_minutes)))
}

/// The batch the calling session's run decides for: its run holds the batch's integration task, the batch red.
fn own_batch(tx: &Transaction, who: &Who) -> Result<Batch, String> {
    let run = crate::dispatcher::runs::in_pane(tx, &who.key)?;
    for id in run.map(|r| r.ids).unwrap_or_default() {
        if let Ok(key) = ops::key_of(tx, &id) && let Some(b) = bs::by_task(tx, &key)? {
            if b.state != bs::RED {
                return Err(format!("Batch {} is {}: nothing to decide now.", b.number, b.state));
            }
            return Ok(b);
        }
    }
    Err("Only a batch's integration task decides for it.".into())
}

/// integration_return {task, analysis}: a task of the batch back to its agent with the analysis, redacted; the
/// batch stays red until integration_recheck, so several may go back for one more round.
pub fn give_back(tx: &Transaction, who: &Who, args: &Value, at: i64) -> Result<String, String> {
    let b = own_batch(tx, who)?;
    let task = ops::get(tx, arg(args, "task")?)?;
    if !b.tasks.contains(&task.key) {
        return Err(format!("{} is not a task of batch {}.", task.id, b.number));
    }
    let analysis = crate::sent::redact(&crate::plans::rules::clean(arg(args, "analysis")?));
    let lacks: Vec<&str> = ANALYSIS.iter().copied().filter(|s| !analysis.contains(&format!("{s}:")) && !analysis.contains(&format!("## {s}"))).collect();
    if !lacks.is_empty() {
        return Err(format!("The analysis lacks {}: each section as \"Name:\" or a heading.", lacks.join(", ")));
    }
    let i_id = bs::task_of(tx, &b)?.unwrap_or_default();
    tx.execute("INSERT OR IGNORE INTO integration_returns (task, commit_sha, why, at) SELECT task, commit_sha, 'red', ?2 FROM integration_entries WHERE task = ?1", (&task.key, at)).map_err(sql)?;
    bs::give_back(tx, &task.key, &format!("{} returned: red check. Batch {}'s check was red and {i_id} found its cause in {}. Its analysis:\n\n{analysis}", task.id, b.number, task.id), bs::Left::Red, at)?;
    Ok(format!("{} went back to its agent. Call integration_recheck when you have returned every task that caused it.", task.id))
}

/// integration_recheck {why?}: the batch built anew without what was returned and with the fixes added, and
/// checked; with neither, the same candidate checked again once, why given, for a flaky test or a cause mended outside.
pub fn recheck(tx: &Transaction, who: &Who, args: &Value, at: i64) -> Result<String, String> {
    let b = own_batch(tx, who)?;
    let why = args["why"].as_str().map(str::trim).filter(|w| !w.is_empty());
    if !bs::changed_since_round(tx, &b)? {
        if b.unchanged_rechecks >= 1 {
            return Err(format!("Batch {} was checked again unchanged once: return a task, add a fix, or ask a human.", b.number));
        }
        why.ok_or("Say why it is checked again unchanged: the flaky test, or what you mended.")?;
    }
    bs::check_again(tx, &b, why, at)
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
    let b = store.with(|tx| own_batch(tx, who))?;
    let tree = b.agent_tree.clone().ok_or_else(|| format!("Batch {} has no worktree of its agent.", b.number))?;
    let fix = alone_on(Path::new(&tree), &commit, &b.base)?;
    store.with(|tx| {
        let b = own_batch(tx, who)?;
        let mut fixes = b.fixes.clone();
        if !fixes.contains(&fix) {
            fixes.push(fix.clone());
        }
        tx.execute("UPDATE integration_batches SET fixes = ?2 WHERE id = ?1", (b.id, serde_json::to_string(&fixes).unwrap_or_default())).map_err(sql)?;
        let i_id = bs::task_of(tx, &b)?.unwrap_or_default();
        ops::comment(tx, DISPATCHER, &i_id, &format!("Its agent added a fix of its own: {}, on the target as {}. It joins the candidate first.", short(&commit), short(&fix)), at)?;
        Ok(format!("{} joins batch {}'s candidate first. Call integration_recheck to build and check it.", short(&commit), b.number))
    })
}

/// Commit `commit`'s own change picked onto `base`, in a worktree made for that beside the agent's and removed
/// after: the agent's own stays as it stood, and no hook of the project runs.
fn alone_on(tree: &Path, commit: &str, base: &str) -> Result<String, String> {
    let full = git::run(tree, &["rev-parse", "--verify", "--quiet", &format!("{commit}^{{commit}}")]).map_err(|_| format!("{commit} is no commit in your worktree."))?;
    let pick = PathBuf::from(format!("{}-fix", tree.display()));
    let at = pick.to_string_lossy().to_string();
    let _ = git::run(tree, &["worktree", "remove", "--force", &at]);
    let picked = git::run(tree, &["-c", "core.hooksPath=/dev/null", "worktree", "add", "--quiet", "--detach", &at, base])
        .and_then(|_| git::run(&pick, &["-c", "core.hooksPath=/dev/null", "cherry-pick", &full]))
        .and_then(|_| git::run(&pick, &["rev-parse", "HEAD"]));
    let _ = git::run(tree, &["worktree", "remove", "--force", &at]);
    picked.map_err(|e| format!("{} does not apply alone to the target at {}: {e}", short(&full), short(base)))
}

/// finish_task on a batch's integration task: refused while its batch is not over.
pub fn finish_refused(tx: &Transaction, key: &str) -> Result<Option<String>, String> {
    Ok(bs::live_by_task(tx, key)?.map(|n| format!("{} ends when its batch {n} does: return tasks, check again, or ask a human.", bs::slug(tx, key).unwrap_or_default())))
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
