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
    let (name, _, dir) = ops::project_place(tx, &b.project)?;
    let repo = integration::local_dir(&dir);
    let id = bs::slug(tx, key)?;
    let (why, started, ended): (Option<String>, Option<i64>, Option<i64>) = tx.query_row("SELECT why, started, ended FROM integration_rounds WHERE batch = ?1 AND round = ?2",
        (b.id, b.round), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(sql)?.unwrap_or((None, None, None));
    let tasks: Vec<String> = bs::queued_of(tx, &b)?.iter().map(|q| format!("{} at {} ({})", q.id, short(&q.commit), changed(&repo, &b.base, &q.commit))).collect();
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

/// The most files the prompt names of one task.
const MOST_FILES: usize = 30;

/// The files a task's commit changed since it left the target, by name alone (§9 Security), at most MOST_FILES. Read
/// in the press's transaction: once a red round's agent starts, one `git diff` a task of a batch of a few.
fn changed(repo: &Path, base: &str, commit: &str) -> String {
    let Ok(files) = git::changed(repo, base, commit) else { return "its files unknown".into() };
    match files.len() {
        0 => "no files".into(),
        n if n > MOST_FILES => format!("{} and {} more", files[..MOST_FILES].join(", "), n - MOST_FILES),
        _ => files.join(", "),
    }
}

/// What the same session is told when a later round of its batch is red (AC-194): the round's own fixed words, its
/// number, why and its log, what it checked, where the agent's worktree now stands; then what the humans answered,
/// when it had asked.
pub fn red_again(tx: &Transaction, key: &str, home: &Path, answers: Option<&[String]>) -> Result<Option<String>, String> {
    let Some(b) = bs::by_task(tx, key)? else { return Ok(None) };
    let id = bs::slug(tx, key)?;
    let log = integration::log_in(home, &b.project, b.number, b.round, "check");
    let shown = log.display().to_string();
    let mut why = b.why.as_deref().unwrap_or("its check was red").trim_end_matches('.').to_string();
    if log.exists() && !why.contains(&shown) {
        why.push_str(&format!(". Its log: {shown}"));
    }
    let ids: Vec<String> = bs::queued_of(tx, &b)?.into_iter().map(|q| q.id).collect();
    let mut words = format!("[terminum Dispatcher] {id}: batch {}'s round {} ended red: {why}. It checked {} with {}; your worktree is at it now. Decide as before.",
                            b.number, b.round, short(b.candidate.as_deref().unwrap_or_default()), ids.join(", "));
    if let Some(said) = answers.filter(|a| !a.is_empty()) {
        words.push_str(&format!("\n\nThe human answered your questions, in the comments below. Take them into account.\n\n{}", said.join("\n\n---\n\n")));
    }
    Ok(Some(words))
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
    if let Some(commit) = tx.query_row("SELECT commit_sha FROM integration_entries WHERE task = ?1", [&task.key], |r| r.get::<_, String>(0)).optional().map_err(sql)? {
        bs::not_again(tx, &task.key, &commit, at)?;
    }
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

/// integration_add_fix {commit}: the agent's own commit, from its worktree, kept with the batch once its change applies
/// alone to the batch's target (a cherry-pick, outside the store's transaction); each round built after picks it
/// anew onto the target as it is then, so it carries no task of the batch and joins the candidate first.
pub fn add_fix(store: &Store, who: &Who, args: &Value, at: i64) -> Result<String, String> {
    let commit = arg(args, "commit")?.to_string();
    // a commit as git names it, never an option or a range: it reaches git's arguments
    if !(7..=64).contains(&commit.len()) || !commit.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("A commit is its hash, 7 to 64 hex digits, from your worktree.".into());
    }
    let b = store.with(|tx| own_batch(tx, who))?;
    let tree = b.agent_tree.clone().ok_or_else(|| format!("Batch {} has no worktree of its agent.", b.number))?;
    let tree = Path::new(&tree);
    let full = git::run(tree, &["rev-parse", "--verify", "--quiet", &format!("{commit}^{{commit}}")]).map_err(|_| format!("{commit} is no commit in your worktree."))?;
    // its own: made on the candidate it was given, never a task's commit or another branch's
    let candidate = b.candidate.clone().unwrap_or_default();
    if full == candidate || !git::ok(tree, &["merge-base", "--is-ancestor", &candidate, &full])? {
        return Err(format!("{} is no commit of yours on the candidate {}: commit your fix in your worktree.", short(&full), short(&candidate)));
    }
    // a path of this call's own: a rebuild of the engine or another call picks beside it, never in it
    let (_, left_out) = picked_onto(tree, &scratch_of(tree, &format!("fix-{}", short(&full))), std::slice::from_ref(&full), &b.base)?;
    if let Some((_, e)) = left_out.first() {
        return Err(format!("{} does not apply alone to the target at {}: {e}", short(&full), short(&b.base)));
    }
    store.with(|tx| {
        let b = own_batch(tx, who)?;
        let mut fixes = b.fixes.clone();
        if !fixes.contains(&full) {
            fixes.push(full.clone());
        }
        tx.execute("UPDATE integration_batches SET fixes = ?2 WHERE id = ?1", (b.id, serde_json::to_string(&fixes).unwrap_or_default())).map_err(sql)?;
        let i_id = bs::task_of(tx, &b)?.unwrap_or_default();
        ops::comment(tx, DISPATCHER, &i_id, &format!("Its agent added a fix of its own: {}. Its change alone is picked onto the target, first in the candidate.", short(&full)), at)?;
        Ok(format!("{} joins batch {}'s candidate first. Call integration_recheck to build and check it.", short(&full), b.number))
    })
}

/// Where a batch's fixes are picked: beside its agent's worktree, `<tree>-<whose>`.
pub(super) fn scratch_of(agent_tree: &Path, whose: &str) -> PathBuf {
    PathBuf::from(format!("{}-{whose}", agent_tree.display()))
}

/// Each of `commits`, its own change alone, picked in turn onto `onto` in `at`, a worktree made for that and removed
/// after: the agent's own stays as it stood, and no hook of the project runs. `repo` is any worktree of the
/// project's repository. Answers the head the picks made (`onto` when none applied) and each commit left out with
/// git's words, or "its change is already there" when it would change nothing.
pub(super) fn picked_onto(repo: &Path, at: &Path, commits: &[String], onto: &str) -> Result<(String, Vec<(String, String)>), String> {
    let path = at.to_string_lossy().to_string();
    // one left by a stop in the middle is locked: twice forced
    let _ = git::run(repo, &["worktree", "remove", "--force", "--force", &path]);
    let _ = git::run(repo, &["worktree", "prune"]);
    git::run(repo, &["-c", "core.hooksPath=/dev/null", "worktree", "add", "--quiet", "--detach", &path, onto]).map_err(|e| format!("A worktree to pick fixes in could not be made: {e}"))?;
    let mut left_out = Vec::new();
    for c in commits {
        if let Err(e) = git::run(at, &["-c", "core.hooksPath=/dev/null", "cherry-pick", c]) {
            let empty = git::ok(at, &["diff", "--cached", "--quiet"]).unwrap_or(false) && git::run(at, &["ls-files", "-u"]).is_ok_and(|u| u.is_empty());
            let _ = git::run(at, &["cherry-pick", "--abort"]);
            left_out.push((c.clone(), if empty { "its change is already there".into() } else { e }));
        }
    }
    let head = git::run(at, &["rev-parse", "HEAD"]);
    let _ = git::run(repo, &["worktree", "remove", "--force", "--force", &path]);
    Ok((head?, left_out))
}

/// finish_task on a batch's integration task: refused while its batch is not over.
pub fn finish_refused(tx: &Transaction, key: &str) -> Result<Option<String>, String> {
    Ok(bs::live_by_task(tx, key)?.map(|n| format!("{} ends when its batch {n} does: return tasks, check again, or ask a human.", bs::slug(tx, key).unwrap_or_default())))
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
