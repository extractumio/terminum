// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Prepare and Verify of a queued task (`terminum-integration.md` AC-198): run by terminumd in a tree of its own at
//! the task's commit, never the agent's, before the task may join a batch. Each run is a window of terminumd's tmux
//! as Check's is; a few at once, as their own checks pace themselves (load gate, heavy-build slots). Kept in
//! `integration_verifies`, apart from the queue's entry, so a task taken out meanwhile still has its window ended.

use std::path::Path;

use rusqlite::OptionalExtension;

use super::engine::{ended, place, worktree};
use super::git;
use super::runner::Runner;
use super::store::{self as bs, Left};
use crate::tasks::integration;
use crate::tasks::ops::{self, sql};
use crate::tasks::store::Store;

/// How many tasks' Prepare and Verify run at once.
pub const VERIFY_MOST: usize = 2;

/// A task's Prepare and Verify, as kept.
struct Run {
    task: String,
    project: String,
    commit: String,
    pane: Option<String>,
    tree: Option<String>,
    started: Option<i64>,
}

/// One beat: ended windows judged, windows of tasks gone ended, new ones started.
pub fn verify_step(store: &Store, runner: &dyn Runner, home: &Path, now: i64) -> Result<(), String> {
    let runs: Vec<Run> = store.with(|tx| ops::rows(tx, "SELECT task, project, commit_sha, pane, tree, started FROM integration_verifies ORDER BY started", [],
        |r| Ok(Run { task: r.get(0)?, project: r.get(1)?, commit: r.get(2)?, pane: r.get(3)?, tree: r.get(4)?, started: r.get(5)? })))?;
    let mut running = 0;
    for run in runs {
        // still queued with that commit, waiting for its Verify: else nothing of it goes on
        let wanted: bool = store.with(|tx| tx.query_row("SELECT 1 FROM integration_entries WHERE task = ?1 AND commit_sha = ?2 AND stage = 'verifying'",
            (&run.task, &run.commit), |_| Ok(())).optional().map(|o| o.is_some()).map_err(sql))?;
        if !wanted {
            if let Some(pane) = &run.pane {
                runner.stop(pane);
                runner.close(pane);
            }
            finish(store, home, &run)?;
            continue;
        }
        if follow(store, runner, home, &run, now)? {
            running += 1;
        }
    }
    let next: Vec<(String, String, String)> = store.with(|tx| ops::rows(tx, "SELECT e.task, e.project, e.commit_sha FROM integration_entries e
        WHERE e.stage = 'verifying' AND NOT EXISTS (SELECT 1 FROM integration_verifies v WHERE v.task = e.task) ORDER BY e.queued_at", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))))?;
    for (task, project, commit) in next.into_iter().take(VERIFY_MOST.saturating_sub(running)) {
        start(store, runner, home, &task, &project, &commit, now)?;
    }
    Ok(())
}

/// The task's window followed: whether it still runs. Ended, the task is ready for a batch, or back to its agent.
fn follow(store: &Store, runner: &dyn Runner, home: &Path, run: &Run, now: i64) -> Result<bool, String> {
    let Some(pane) = &run.pane else { return Ok(false) };
    let p = place(store, home, &run.project)?;
    let (id, number) = store.with(|tx| tx.query_row("SELECT slug, number FROM tasks WHERE id = ?1", [&run.task], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).map_err(sql))?;
    let log = integration::tree_in(home, &run.project).with_file_name(format!("verify-{number}.log"));
    let code = match runner.status(pane) {
        // lost with its window: run again
        Err(_) => {
            store.with(|tx| tx.execute("UPDATE integration_verifies SET pane = NULL WHERE task = ?1", [&run.task]).map(drop).map_err(sql))?;
            return Ok(false);
        }
        Ok(None) if now - run.started.unwrap_or(now) > p.s.check_minutes * 60 => {
            runner.stop(pane);
            ended(runner, pane, &log, -1)
        }
        Ok(None) => return Ok(true),
        Ok(Some(code)) => ended(runner, pane, &log, code),
    };
    let short = git::short(&run.commit).to_string();
    store.with(|tx| match code {
        0 => {
            tx.execute("UPDATE integration_entries SET stage = NULL WHERE task = ?1", [&run.task]).map_err(sql)?;
            ops::comment(tx, crate::tasks::store::DISPATCHER, &id, &format!("{id}'s Prepare and Verify passed on {short}: it waits for a batch."), now).map(drop)
        }
        -1 => bs::give_back(tx, &run.task, &format!("{id} returned: red check: its Verify ran over {} minutes. Its log: {}", p.s.check_minutes, log.display()), Left::Red, now),
        // Prepare's 2 is a conflict with the target; any other failure a red check (§9)
        2 => bs::give_back(tx, &run.task, &format!("{id} returned: conflict: its Prepare or Verify ended with status 2. Its log: {}", log.display()), Left::Conflict, now),
        code => {
            // as a red check's: the same commit is not queued again (AC-187)
            tx.execute("INSERT OR IGNORE INTO integration_returns (task, commit_sha, why, at) VALUES (?1, ?2, 'red', ?3)", rusqlite::params![run.task, run.commit, now]).map_err(sql)?;
            bs::give_back(tx, &run.task, &format!("{id} returned: red check: its Prepare or Verify ended with status {code}. Its log: {}", log.display()), Left::Red, now)
        }
    })?;
    finish(store, home, run)?;
    Ok(false)
}

/// A task's tree made at its commit and its Prepare and Verify started there; one that cannot start goes back.
fn start(store: &Store, runner: &dyn Runner, home: &Path, task: &str, project: &str, commit: &str, now: i64) -> Result<(), String> {
    let p = place(store, home, project)?;
    let line = [p.s.prepare.as_str(), p.s.verify.as_str()].iter().filter(|l| !l.is_empty()).copied().collect::<Vec<_>>().join(" && ");
    if !p.s.on {
        return Ok(());   // nothing runs while integration is off; the queue keeps it
    }
    if line.is_empty() {
        return store.with(|tx| tx.execute("UPDATE integration_entries SET stage = NULL WHERE task = ?1", [task]).map(drop).map_err(sql));
    }
    let (id, number) = store.with(|tx| tx.query_row("SELECT slug, number FROM tasks WHERE id = ?1", [task], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).map_err(sql))?;
    let tree = integration::verify_tree_in(home, project, number);
    let made = worktree(&p, tree.clone(), commit).map_err(|e| format!("its tree could not be made: {e}"));
    let vars = vec![("TM_INTEGRATION_TASK", id.clone()), ("TM_INTEGRATION_SHA", commit.to_string()), ("TM_INTEGRATION_TARGET", p.s.target.clone()),
                    ("TM_INTEGRATION_TREE", tree.display().to_string()), ("TM_INTEGRATION_PROJECT_DIR", p.repo.display().to_string())];
    let started = made.and_then(|t| runner.start(&format!("verify-{number}"), &t, &line, &vars));
    store.with(|tx| {
        tx.execute("INSERT INTO integration_verifies (task, project, commit_sha, pane, tree, started) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                   rusqlite::params![task, project, commit, started.as_ref().ok(), tree.display().to_string(), now]).map_err(sql)?;
        ops::comment(tx, crate::tasks::store::DISPATCHER, &id, &format!("Verifying {}: its Prepare and Verify run in {}.", git::short(commit), tree.display()), now)?;
        match &started {
            Ok(_) => Ok(()),
            Err(why) => bs::give_back(tx, task, &format!("{id} returned: red check: its Prepare and Verify did not start: {why}"), Left::Red, now),
        }
    })?;
    if started.is_err() {
        let run = Run { task: task.into(), project: project.into(), commit: commit.into(), pane: None, tree: Some(tree.display().to_string()), started: Some(now) };
        finish(store, home, &run)?;
    }
    Ok(())
}

/// The task's tree removed with git and its run forgotten.
fn finish(store: &Store, home: &Path, run: &Run) -> Result<(), String> {
    if let (Some(tree), Ok(p)) = (&run.tree, place(store, home, &run.project)) {
        let _ = git::run(&p.repo, &["worktree", "remove", "--force", tree]);
    }
    store.with(|tx| tx.execute("DELETE FROM integration_verifies WHERE task = ?1", [&run.task]).map(drop).map_err(sql))
}

#[cfg(test)]
#[path = "stages_tests.rs"]
mod tests;
