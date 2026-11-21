// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A red target looked at again by terminumd (`terminum-integration.md` AC-198): its Bug task's steps run once more on
//! the target itself, 30 minutes after it was found red, then again only when the target moves. Green there, the red
//! did not hold: Dispatcher ends the Bug task DONE with that run as its evidence, and its held tasks' Verify runs again
//! (`red_target::release`). Red there, the Bug task stays, for an agent or a human. A step that is red, then green on a
//! re-run, twice within a week gets a Bug task of its own, which holds nothing.

use std::path::Path;

use rusqlite::{OptionalExtension, Transaction};

use super::engine::{place, target_now, worktree};
use super::git;
use super::retry_notes;
use super::runner::Runner;
use super::stages::{line_of, vars, Run};
use crate::tasks::integration;
use crate::tasks::integration_queue::and_list;
use crate::tasks::ops::{self, sql, NewTask};
use crate::tasks::store::{Store, DISPATCHER};

/// What marks a Verify row as a red target's re-run, not its Bug task's own Verify (`integration_verifies.red_attempt`).
pub(super) const RECHECK: &str = "recheck";

/// How long after a red is seen it is run again: a red of a busy moment passes by then.
const AFTER: i64 = 1800;
/// How often the target is fetched to see whether it moved, while a red stands.
const LOOK: i64 = 300;
/// Within this long of a green re-run, the same steps red and green again are a step red now and then.
const AGAIN_WITHIN: i64 = 7 * 86_400;

/// The red's facts, kept when its Bug task is made: the steps, the commit whose plan names them, the red target.
pub(super) fn keep(tx: &Transaction, project: &str, steps: &[String], of: &str, red_sha: &str, now: i64) -> Result<(), String> {
    tx.execute("INSERT INTO integration_red (project, steps, of_sha, red_sha, red_at, retried, looked_at) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?5)
                ON CONFLICT (project) DO UPDATE SET steps = excluded.steps, of_sha = excluded.of_sha, red_sha = excluded.red_sha, red_at = excluded.red_at, retried = 0, looked_at = excluded.looked_at",
               rusqlite::params![project, steps.join(" "), of, red_sha, now]).map(drop).map_err(sql)
}

/// A project whose open Bug task for its red target is due a look.
struct Due {
    project: String,
    bug: String,
    id: String,
    number: i64,
    steps: String,
    of: String,
    red_sha: String,
    retried: bool,
}

fn due(tx: &Transaction, now: i64) -> Result<Vec<Due>, String> {
    ops::rows(tx, "SELECT r.project, b.id, b.slug, b.number, r.steps, r.of_sha, r.red_sha, r.retried FROM integration_red r
            JOIN project_integration i ON i.project = r.project JOIN tasks b ON b.id = i.red_bug
            WHERE i.is_on = 1 AND b.status NOT IN ('DONE', 'FAILED') AND b.archived_at IS NULL
              AND ?1 - r.red_at >= ?2 AND ?1 - r.looked_at >= ?3 AND NOT EXISTS (SELECT 1 FROM integration_verifies v WHERE v.task = b.id)",
        rusqlite::params![now, AFTER, LOOK], |r| Ok(Due { project: r.get(0)?, bug: r.get(1)?, id: r.get(2)?, number: r.get(3)?, steps: r.get(4)?, of: r.get(5)?,
                                                       red_sha: r.get(6)?, retried: r.get(7)? }))
}

/// The re-runs due started, at most `room`; how many started. The target is fetched at most every 5 minutes a project,
/// and a target already found red twice runs nothing until it moves.
pub(super) fn start_due(store: &Store, runner: &dyn Runner, home: &Path, room: usize, now: i64) -> Result<usize, String> {
    let mut started = 0;
    for d in store.with(|tx| due(tx, now))? {
        if started >= room {
            break;
        }
        store.with(|tx| tx.execute("UPDATE integration_red SET looked_at = ?2 WHERE project = ?1", rusqlite::params![d.project, now]).map(drop).map_err(sql))?;
        let p = place(store, home, &d.project)?;
        let target = match target_now(&p) {
            Ok(t) => t,
            Err(e) => { tracing::warn!(event = "integration.red_recheck", project = %d.project, error = %e); continue }
        };
        if target == d.red_sha && d.retried {
            continue;
        }
        let tree = integration::verify_tree_in(home, &d.project, d.number);
        let attempt = crate::auth::crypto::random_hex::<16>();
        let line = line_of("", &p.s.verify);
        let run = (!line.is_empty()).then_some(()).ok_or_else(|| "Verify is empty".to_string())
            .and_then(|()| worktree(&p, tree.clone(), &target).map_err(|e| format!("its tree could not be made: {e}")))
            .and_then(|t| {
                retry_notes::clear(&t)?;
                let mut vars = vars(&d.id, &target, &p, &t);
                vars.extend([("TM_INTEGRATION_RERUN", d.steps.clone()), ("TM_INTEGRATION_RERUN_OF", d.of.clone())]);
                runner.start(&format!("verify-{}", d.number), &t, &line, &vars)
            });
        let log = integration::verify_log_in(home, &d.project, d.number, &attempt);
        let failed = and_list(&d.steps.split(' ').collect::<Vec<_>>());
        store.with(|tx| match &run {
            Ok(pane) => {
                tx.execute("INSERT INTO integration_verifies (task, project, commit_sha, pane, tree, started, attempt, target_sha, steps, red_attempt) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?3, ?8, ?9)",
                           rusqlite::params![d.bug, d.project, target, pane, tree.display().to_string(), now, attempt, d.steps, RECHECK]).map_err(sql)?;
                ops::comment(tx, DISPATCHER, &d.id, &format!("terminumd runs {failed} again on {} at {} to tell whether the red holds: green there, {} ends DONE by itself. Its log: {}",
                                                             p.s.target, git::short(&target), d.id, log.display()), now).map(drop)
            }
            Err(why) => ops::comment(tx, DISPATCHER, &d.id, &format!("terminumd could not run {failed} again on {} at {}: {why}. It tries again when the target moves.", p.s.target, git::short(&target)), now)
                .and_then(|_| tx.execute("UPDATE integration_red SET red_sha = ?2, retried = 1 WHERE project = ?1", (&d.project, &target)).map(drop).map_err(sql)),
        })?;
        if run.is_err() {
            let _ = git::run(&p.repo, &["worktree", "remove", "--force", &tree.to_string_lossy()]);
        } else {
            started += 1;
        }
    }
    Ok(started)
}

/// Whether `run` is a re-run of its project's red target, in its Bug task's row.
pub(super) fn is_recheck(run: &Run) -> bool {
    run.red_attempt == RECHECK
}

/// The re-run on `target` ended with `code`: green, the Bug task is DONE by Dispatcher; red, or not known, it stays.
pub(super) fn judge(tx: &Transaction, home: &Path, run: &Run, target: &str, code: i32, now: i64) -> Result<(), String> {
    let s = integration::read(tx, &run.project)?;
    let (id, at) = (&run.id, format!("{} at {}", s.target, git::short(target)));
    let failed = and_list(&run.steps.iter().map(String::as_str).collect::<Vec<_>>());
    let log = integration::verify_log_in(home, &run.project, run.number, &run.attempt);
    if code != 0 {
        tx.execute("UPDATE integration_red SET red_sha = ?2, red_at = ?3, retried = 1 WHERE project = ?1", rusqlite::params![run.project, target, now]).map_err(sql)?;
        let how = if code == 1 { "fail there too" } else { "could not be told apart there" };
        return ops::comment(tx, DISPATCHER, id, &format!("{failed} {how} on {at}: {id} stays open; terminumd runs them again when the target moves. Its log: {}", log.display()), now).map(drop);
    }
    let (steps, green): (String, Option<(String, i64)>) = tx.query_row("SELECT steps, green_steps, green_at FROM integration_red WHERE project = ?1", [&run.project],
        |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.zip(r.get::<_, Option<i64>>(2)?)))).optional().map_err(sql)?.unwrap_or_default();
    ops::comment(tx, DISPATCHER, id, &format!("{failed} pass on {at} on terminumd's run: the target is green for them, the red did not hold. {id} is DONE, and each task held behind it runs its Verify again. Its log: {}", log.display()), now)?;
    ops::move_to(tx, DISPATCHER, id, "DONE", now)?;
    tx.execute("UPDATE integration_red SET green_steps = ?2, green_at = ?3 WHERE project = ?1", rusqlite::params![run.project, steps, now]).map_err(sql)?;
    if green.is_some_and(|(was, at)| was == steps && now - at < AGAIN_WITHIN) {
        flaky(tx, &run.project, &s.target, &failed, id, now)?;
    }
    Ok(())
}

/// The same steps red on the target, then green on a re-run, twice within a week: a Bug task names them, holding nothing.
fn flaky(tx: &Transaction, project: &str, target: &str, failed: &str, bug: &str, now: i64) -> Result<(), String> {
    let title = format!("Bug: {failed} fail now and then on {target}");
    if crate::tasks::query::repeat_of(tx, project, &title)?.is_some() {
        return Ok(());
    }
    let text = format!("## Problem or goal\nThe Verify steps {failed} failed on {target}, held the integration queue behind a red-target task, then passed on terminumd's re-run of the same target, twice within a week (the latest: {bug}). Each time the queue waited half an hour or more.\n\n\
        ## Expected result\nThe steps pass every time on an unchanged target, or their flaky check is named with its cause.\n\n\
        ## Context\nMade by Dispatcher (terminum-integration.md AC-198). Nothing waits behind this task.");
    ops::create(tx, DISPATCHER, NewTask { title, text, project: Some(project.into()), severity: Some("P1".into()), ..Default::default() }, now).map(drop)
}

#[cfg(test)]
#[path = "red_recheck_tests.rs"]
mod tests;
