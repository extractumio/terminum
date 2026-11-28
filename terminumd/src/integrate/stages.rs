// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Prepare and Verify of a queued task (`terminum-integration.md` AC-198): run by terminumd in a tree of its own at
//! the task's commit, never the agent's, before the task may join a batch. Each run is a window of terminumd's tmux
//! as Check's is, judged by the same rules; a few at once, as their own checks pace themselves (load gate,
//! heavy-build slots). Kept in `integration_verifies`, apart from the queue's entry, so a task taken out meanwhile
//! still has its window ended. A red Verify that names its failed steps runs them on the target before the task
//! goes back (`red_target`).

use std::path::Path;

use rusqlite::Transaction;

use super::engine::{how_ended, place, window, worktree, Place, Window};
use super::git;
use super::{red_recheck, red_target};
use super::retry_notes;
use super::runner::Runner;
use super::store::{self as bs, Left};
use crate::supervisor::verify_review;
use crate::tasks::integration;
use crate::tasks::ops::{self, sql};
use crate::tasks::store::{Store, DISPATCHER};

/// How many tasks' Prepare and Verify run at once on the machine, over all projects (§9 Limits). Their lines pace
/// their heavy steps themselves (this repository's: a heavy-build slot for each cargo step, one of two live-check
/// slots for each real-program one), so beside two long runs a third lets a short one, a pass reused, through.
pub const VERIFY_MOST: usize = 3;

/// Prepare and Verify as one window that says which failed: Prepare's 2 stays 2 (a conflict), any other failure of
/// either is 1 (a red check). Verify preserves the design-review status for a matching typed failure.
/// Each line stands on lines of its own, so nothing it holds (a `#`, a quote) reaches the frame; lines hold no line ends.
pub(crate) fn line_of(prepare: &str, verify: &str) -> String {
    stages_line(prepare, verify, false)
}

const OWN_VERIFY_GREEN: i32 = 78;
const OWN_VERIFY_RED: i32 = 79;

fn stages_line(prepare: &str, verify: &str, own: bool) -> String {
    let mut parts = Vec::new();
    if !prepare.is_empty() {
        parts.push(format!("(\n{prepare}\n) || {{ s=$?; [ \"$s\" -eq 2 ] && exit 2; exit 1; }}"));
    }
    if !verify.is_empty() {
        let code = verify_review::MISSING_EXIT;
        let red = if own { OWN_VERIFY_RED } else { 1 };
        parts.push(format!("(\n{verify}\n) || {{ s=$?; [ \"$s\" -eq {code} ] && exit {code}; exit {red}; }}"));
        if own { parts.push(format!("exit {OWN_VERIFY_GREEN}")); }
    }
    parts.join("\n")
}

/// A task's Prepare and Verify as kept, with what judging it needs.
pub(super) struct Run {
    pub(super) task: String,
    pub(super) id: String,
    pub(super) number: i64,
    pub(super) project: String,
    pub(super) commit: String,
    pane: Option<String>,
    started: i64,
    pub(super) attempt: String,
    /// still queued with that commit and waiting for its Verify
    wanted: bool,
    pub(super) minutes: i64,
    /// the run of the task's failed steps on the target, at this commit (`red_target`)
    target: Option<String>,
    pub(super) steps: Vec<String>,
    /// the task's own run, whose log holds its red
    pub(super) red_attempt: String,
}

/// One beat: ended windows judged, windows of tasks gone ended, new ones started; one read of the store.
pub fn verify_step(store: &Store, runner: &dyn Runner, home: &Path, now: i64) -> Result<(), String> {
    let (runs, next, looks) = store.with(|tx| {
        red_target::release(tx, now)?;
        let runs = ops::rows(tx, &format!("SELECT v.task, t.slug, t.number, v.project, v.commit_sha, v.pane, v.started,
                EXISTS (SELECT 1 FROM integration_entries e WHERE e.task = v.task AND e.commit_sha = v.commit_sha AND e.stage = 'verifying') OR {},
                coalesce(i.check_minutes, 90), v.attempt, v.target_sha, v.steps, v.red_attempt
            FROM integration_verifies v JOIN tasks t ON t.id = v.task LEFT JOIN project_integration i ON i.project = v.project ORDER BY v.started", red_recheck::WANTED), [],
            |r| Ok(Run { task: r.get(0)?, id: r.get(1)?, number: r.get(2)?, project: r.get(3)?, commit: r.get(4)?, pane: r.get(5)?, started: r.get(6)?, wanted: r.get(7)?,
                         minutes: r.get(8)?, attempt: r.get(9)?, target: r.get(10)?, steps: r.get::<_, String>(11)?.split_whitespace().map(String::from).collect(), red_attempt: r.get(12)? }))?;
        let next: Vec<(String, String, String)> = ops::rows(tx, "SELECT e.task, e.project, e.commit_sha FROM integration_entries e JOIN project_integration i ON i.project = e.project
            WHERE e.stage = 'verifying' AND i.is_on = 1 AND NOT EXISTS (SELECT 1 FROM integration_verifies v WHERE v.task = e.task) ORDER BY e.queued_at LIMIT ?1",
            [VERIFY_MOST as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok((runs, next, red_recheck::due(tx, now)?))
    })?;
    let mut running = 0;
    for run in &runs {
        if !run.wanted {
            if let Some(pane) = &run.pane {
                runner.stop(pane);
                runner.close(pane);
            }
            finish(store, home, run)?;
        } else if follow(store, runner, home, run, now)? {
            running += 1;
        }
    }
    // a red target run again by itself frees the tasks held behind it (AC-198): before new tasks' Verifies
    running += red_recheck::start_due(store, runner, home, looks, VERIFY_MOST.saturating_sub(running), now)?;
    for (task, project, commit) in next.into_iter().take(VERIFY_MOST.saturating_sub(running)) {
        start(store, runner, home, &task, &project, &commit, now)?;
    }
    Ok(())
}

/// The task's window followed: whether it still runs. Ended, the task is ready for a batch, or back to its agent.
fn follow(store: &Store, runner: &dyn Runner, home: &Path, run: &Run, now: i64) -> Result<bool, String> {
    // its window gone and its tree not yet removed: removed again, then started again
    let Some(pane) = &run.pane else {
        finish(store, home, run)?;
        return Ok(false);
    };
    let log = integration::verify_log_in(home, &run.project, run.number, &run.attempt);
    let code = match window(runner, pane, &log, now - run.started > run.minutes * 60) {
        Window::Running => return Ok(true),
        // lost with its window: the run on the target started again, the task's own forgotten, so the next beat starts it
        // again; a red target's own run is forgotten, and its next look starts it again
        Window::Lost if run.target.is_some() && run.red_attempt != red_recheck::RECHECK && red_target::start(store, runner, home, run, &run.steps, &run.red_attempt, now)? => return Ok(true),
        Window::Lost => {
            finish(store, home, run)?;
            return Ok(false);
        }
        Window::Ended(code) => code,
    };
    let tree = integration::verify_tree_in(home, &run.project, run.number);
    // the run on the target: its notes are the target's, its failed steps what is red there
    if let Some(target) = &run.target {
        let red = retry_notes::failed(&tree);
        store.with(|tx| if run.red_attempt == red_recheck::RECHECK { red_recheck::judge(tx, home, run, target, code, now) } else { red_target::judge(tx, home, run, target, code, red, now) })?;
        finish(store, home, run)?;
        return Ok(false);
    }
    // Keep each own Verify outcome even when its failing steps will be checked on the target.
    let verified = match code { OWN_VERIFY_GREEN => Some(true), OWN_VERIFY_RED => Some(false), _ => None };
    let code = match verified { Some(true) => 0, Some(false) => 1, None => code };
    // read before the tree moves to the target
    let failed = (code == 1).then(|| retry_notes::failed(&tree)).flatten();
    let notes = retry_notes::read(&tree);
    let skipped = retry_notes::skipped(&tree).filter(|_| code == 0);
    let (id, log) = (&run.id, log.display());
    store.with(|tx| {
        if let Some(passed) = verified {
            tx.execute("INSERT OR IGNORE INTO integration_verify_results (attempt, project, task, commit_sha, passed, at, agent)
                SELECT ?1, e.project, e.task, e.commit_sha, ?5, ?6,
                    CASE WHEN e.author GLOB 'agent:pool:*' THEN substr(e.author, length('agent:pool:') + 1) END
                FROM integration_entries e WHERE e.project = ?2 AND e.task = ?3 AND e.commit_sha = ?4",
                (&run.attempt, &run.project, &run.task, &run.commit, passed, now)).map_err(sql)?;
        }
        if let Some(notes) = &notes {
            ops::notice(tx, DISPATCHER, id, &format!("{id}'s Verify has retry diagnostics (AC-197).\nCheck output (data):\n{notes}"), serde_json::json!({"retry": "verify"}), now)?;
        }
        if let Some(skipped) = &skipped {
            ops::notice(tx, DISPATCHER, id, &format!("{id}'s Verify passed without the timed checks a busy machine skipped (AC-146): they did not run before its batch.\nCheck output (data):\n{skipped}"), serde_json::json!({"skipped": "verify"}), now)?;
        }
        Ok(())
    })?;
    // its failed steps named: judged by their run on the target
    if let Some(steps) = &failed {
        if red_target::start(store, runner, home, run, steps, &run.attempt, now)? {
            return Ok(true);
        }
        finish(store, home, run)?;
        return Ok(false);
    }
    store.with(|tx| {
        match code {
            0 => {
                ready(tx, &run.task)?;
                ops::comment(tx, DISPATCHER, id, &format!("{id}'s Prepare and Verify passed on {}: it waits for a batch. Its log: {log}", git::short(&run.commit)), now).map(drop)
            }
            // Prepare's 2 is a conflict with the target; any other failure a red check (§9)
            2 => bs::give_back(tx, &run.task, &format!("{id} returned: conflict: its Prepare or Verify ended with status 2. Its log: {log}"), Left::Conflict, now),
            code => {
                let missing = verify_review::missed(tx, &run.task, &run.attempt)? && code == verify_review::MISSING_EXIT;
                if !missing { bs::not_again(tx, &run.task, &run.commit, now)?; }
                bs::give_back(tx, &run.task, &format!("{id} returned: red check: its Prepare or Verify {}. Its log: {log}", how_ended(code, run.minutes)), Left::Red, now)
            }
        }
    })?;
    finish(store, home, run)?;
    Ok(false)
}

/// What Prepare and Verify get (§9): the task, the commit they run on, the target and the tree.
pub(super) fn vars(id: &str, sha: &str, p: &Place, tree: &Path) -> Vec<(&'static str, String)> {
    vec![("TM_INTEGRATION_TASK", id.to_string()), ("TM_INTEGRATION_SHA", sha.to_string()), ("TM_INTEGRATION_TARGET", p.s.target.clone()),
         ("TM_INTEGRATION_TREE", tree.display().to_string()), ("TM_INTEGRATION_PROJECT_DIR", p.repo.display().to_string())]
}

/// The queued task ready for a batch.
fn ready(tx: &Transaction, task: &str) -> Result<(), String> {
    tx.execute("UPDATE integration_entries SET stage = NULL WHERE task = ?1", [task]).map(drop).map_err(sql)
}

/// A task's tree made at its commit and its Prepare and Verify started there; one that cannot start goes back.
fn start(store: &Store, runner: &dyn Runner, home: &Path, task: &str, project: &str, commit: &str, now: i64) -> Result<(), String> {
    let p = place(store, home, project)?;
    let line = stages_line(&p.s.prepare, &p.s.verify, true);
    if line.is_empty() {
        return store.with(|tx| ready(tx, task));   // the settings changed since it was queued
    }
    let (id, number) = store.with(|tx| Ok((bs::slug(tx, task)?, ops::number_of(tx, &bs::slug(tx, task)?)?)))?;
    let tree = integration::verify_tree_in(home, project, number);
    let attempt = crate::auth::crypto::random_hex::<16>();
    let log = integration::verify_log_in(home, project, number, &attempt);
    let vars = vars(&id, commit, &p, &tree);
    let started = worktree(&p, tree.clone(), commit).map_err(|e| format!("its tree could not be made: {e}"))
        .and_then(|t| {
            retry_notes::clear(&t)?;
            runner.start(&format!("verify-{number}"), &t, &line, &vars)
        });
    store.with(|tx| match &started {
        Ok(pane) => {
            tx.execute("INSERT INTO integration_verifies (task, project, commit_sha, pane, tree, started, attempt) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                       rusqlite::params![task, project, commit, pane, tree.display().to_string(), now, attempt]).map_err(sql)?;
            ops::comment(tx, DISPATCHER, &id, &format!("Verifying {}: its Prepare and Verify run in {}. Its log: {}", git::short(commit), tree.display(), log.display()), now).map(drop)
        }
        Err(why) => bs::give_back(tx, task, &format!("{id} returned: red check: its Prepare and Verify did not start: {why}"), Left::Red, now),
    })?;
    if started.is_err() {
        let _ = git::run(&p.repo, &["worktree", "remove", "--force", &tree.to_string_lossy()]);
    }
    Ok(())
}

/// The task's tree removed with git, and its run forgotten once the tree is gone. A folder git will not remove
/// (no longer its worktree) is terminumd's own: removed, links not followed, and git told; one that still cannot
/// go is tried again at the next beat.
fn finish(store: &Store, home: &Path, run: &Run) -> Result<(), String> {
    let tree = integration::verify_tree_in(home, &run.project, run.number);
    if tree.exists() && let Ok(p) = place(store, home, &run.project)
        && git::run(&p.repo, &["worktree", "remove", "--force", &tree.to_string_lossy()]).is_err() {
        if let Err(e) = std::fs::remove_dir_all(&tree) {
            tracing::warn!(event = "integration.verify_tree", path = %tree.display(), error = %e);
        }
        let _ = git::run(&p.repo, &["worktree", "prune"]);
    }
    if tree.exists() {
        return store.with(|tx| tx.execute("UPDATE integration_verifies SET pane = NULL WHERE task = ?1", [&run.task]).map(drop).map_err(sql));
    }
    store.with(|tx| {
        verify_review::missed(tx, &run.task, &run.attempt)?;
        tx.execute("DELETE FROM integration_verifies WHERE task = ?1", [&run.task]).map(drop).map_err(sql)
    })
}

#[cfg(test)]
#[path = "stages_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "stages_log_tests.rs"]
mod log_tests;
