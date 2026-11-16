// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Prepare and Verify of a queued task (`terminum-integration.md` AC-198): run by terminumd in a tree of its own at
//! the task's commit, never the agent's, before the task may join a batch. Each run is a window of terminumd's tmux
//! as Check's is, judged by the same rules; a few at once, as their own checks pace themselves (load gate,
//! heavy-build slots). Kept in `integration_verifies`, apart from the queue's entry, so a task taken out meanwhile
//! still has its window ended.

use std::path::Path;

use rusqlite::Transaction;

use super::engine::{place, window, worktree, Window, OVER};
use super::git;
use super::retry_notes;
use super::runner::Runner;
use super::store::{self as bs, Left};
use crate::tasks::integration;
use crate::tasks::ops::{self, sql};
use crate::tasks::store::{Store, DISPATCHER};

/// How many tasks' Prepare and Verify run at once on the machine, over all projects (§9 Limits). Their lines pace
/// their heavy steps themselves (this repository's: a heavy-build slot for each cargo step, one of two live-check
/// slots for each real-program one), so beside two long runs a third lets a short one, a pass reused, through.
pub const VERIFY_MOST: usize = 3;

/// Prepare and Verify as one window that says which failed: Prepare's 2 stays 2 (a conflict), any other failure of
/// either is 1 (a red check), whatever the line's own tool makes of its status (make turns each into 2). Each line
/// stands on lines of its own, so nothing it holds (a `#`, a quote) reaches the frame; lines hold no line ends.
pub(crate) fn line_of(prepare: &str, verify: &str) -> String {
    let mut parts = Vec::new();
    if !prepare.is_empty() {
        parts.push(format!("(\n{prepare}\n) || {{ s=$?; [ \"$s\" -eq 2 ] && exit 2; exit 1; }}"));
    }
    if !verify.is_empty() {
        parts.push(format!("(\n{verify}\n) || exit 1"));
    }
    parts.join("\n")
}

/// A task's Prepare and Verify as kept, with what judging it needs.
struct Run {
    task: String,
    id: String,
    number: i64,
    project: String,
    commit: String,
    pane: Option<String>,
    started: i64,
    /// still queued with that commit and waiting for its Verify
    wanted: bool,
    minutes: i64,
}

/// One beat: ended windows judged, windows of tasks gone ended, new ones started; one read of the store.
pub fn verify_step(store: &Store, runner: &dyn Runner, home: &Path, now: i64) -> Result<(), String> {
    let (runs, next) = store.with(|tx| {
        let runs = ops::rows(tx, "SELECT v.task, t.slug, t.number, v.project, v.commit_sha, v.pane, v.started,
                EXISTS (SELECT 1 FROM integration_entries e WHERE e.task = v.task AND e.commit_sha = v.commit_sha AND e.stage = 'verifying'),
                coalesce(i.check_minutes, 90)
            FROM integration_verifies v JOIN tasks t ON t.id = v.task LEFT JOIN project_integration i ON i.project = v.project ORDER BY v.started", [],
            |r| Ok(Run { task: r.get(0)?, id: r.get(1)?, number: r.get(2)?, project: r.get(3)?, commit: r.get(4)?, pane: r.get(5)?, started: r.get(6)?, wanted: r.get(7)?, minutes: r.get(8)? }))?;
        let next: Vec<(String, String, String)> = ops::rows(tx, "SELECT e.task, e.project, e.commit_sha FROM integration_entries e JOIN project_integration i ON i.project = e.project
            WHERE e.stage = 'verifying' AND i.is_on = 1 AND NOT EXISTS (SELECT 1 FROM integration_verifies v WHERE v.task = e.task) ORDER BY e.queued_at LIMIT ?1",
            [VERIFY_MOST as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok((runs, next))
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
    let log = integration::verify_log_in(home, &run.project, run.number);
    let code = match window(runner, pane, &log, now - run.started > run.minutes * 60) {
        Window::Running => return Ok(true),
        // lost with its window: forgotten, so the next beat starts it again
        Window::Lost => {
            finish(store, home, run)?;
            return Ok(false);
        }
        Window::Ended(code) => code,
    };
    let tree = integration::verify_tree_in(home, &run.project, run.number);
    let notes = retry_notes::read(&tree);
    let skipped = retry_notes::skipped(&tree).filter(|_| code == 0);
    let (id, log) = (&run.id, log.display());
    store.with(|tx| {
        if let Some(notes) = &notes {
            ops::notice(tx, DISPATCHER, id, &format!("{id}'s Verify has retry diagnostics (AC-197).\nCheck output (data):\n{notes}"), serde_json::json!({"retry": "verify"}), now)?;
        }
        if let Some(skipped) = &skipped {
            ops::notice(tx, DISPATCHER, id, &format!("{id}'s Verify passed without the timed checks a busy machine skipped (AC-146): they did not run before its batch.\nCheck output (data):\n{skipped}"), serde_json::json!({"skipped": "verify"}), now)?;
        }
        match code {
            0 => {
                ready(tx, &run.task)?;
                ops::comment(tx, DISPATCHER, id, &format!("{id}'s Prepare and Verify passed on {}: it waits for a batch.", git::short(&run.commit)), now).map(drop)
            }
            // Prepare's 2 is a conflict with the target; any other failure a red check (§9)
            2 => bs::give_back(tx, &run.task, &format!("{id} returned: conflict: its Prepare or Verify ended with status 2. Its log: {log}"), Left::Conflict, now),
            code => {
                bs::not_again(tx, &run.task, &run.commit, now)?;
                let how = if code == OVER { format!("ran over {} minutes", run.minutes) } else { format!("ended with status {code}") };
                bs::give_back(tx, &run.task, &format!("{id} returned: red check: its Prepare or Verify {how}. Its log: {log}"), Left::Red, now)
            }
        }
    })?;
    finish(store, home, run)?;
    Ok(false)
}

/// The queued task ready for a batch.
fn ready(tx: &Transaction, task: &str) -> Result<(), String> {
    tx.execute("UPDATE integration_entries SET stage = NULL WHERE task = ?1", [task]).map(drop).map_err(sql)
}

/// A task's tree made at its commit and its Prepare and Verify started there; one that cannot start goes back.
fn start(store: &Store, runner: &dyn Runner, home: &Path, task: &str, project: &str, commit: &str, now: i64) -> Result<(), String> {
    let p = place(store, home, project)?;
    let line = line_of(&p.s.prepare, &p.s.verify);
    if line.is_empty() {
        return store.with(|tx| ready(tx, task));   // the settings changed since it was queued
    }
    let (id, number) = store.with(|tx| Ok((bs::slug(tx, task)?, ops::number_of(tx, &bs::slug(tx, task)?)?)))?;
    let tree = integration::verify_tree_in(home, project, number);
    let vars = vec![("TM_INTEGRATION_TASK", id.clone()), ("TM_INTEGRATION_SHA", commit.to_string()), ("TM_INTEGRATION_TARGET", p.s.target.clone()),
                    ("TM_INTEGRATION_TREE", tree.display().to_string()), ("TM_INTEGRATION_PROJECT_DIR", p.repo.display().to_string())];
    let started = worktree(&p, tree.clone(), commit).map_err(|e| format!("its tree could not be made: {e}"))
        .and_then(|t| {
            retry_notes::clear(&t)?;
            runner.start(&format!("verify-{number}"), &t, &line, &vars)
        });
    store.with(|tx| match &started {
        Ok(pane) => {
            tx.execute("INSERT INTO integration_verifies (task, project, commit_sha, pane, tree, started) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                       rusqlite::params![task, project, commit, pane, tree.display().to_string(), now]).map_err(sql)?;
            ops::comment(tx, DISPATCHER, &id, &format!("Verifying {}: its Prepare and Verify run in {}.", git::short(commit), tree.display()), now).map(drop)
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
    store.with(|tx| tx.execute("DELETE FROM integration_verifies WHERE task = ?1", [&run.task]).map(drop).map_err(sql))
}

#[cfg(test)]
#[path = "stages_tests.rs"]
mod tests;
