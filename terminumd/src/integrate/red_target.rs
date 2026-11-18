// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A red Verify told apart (`terminum-integration.md` AC-198): the steps a task's Verify names as failed run again
//! on the target, in the task's verify tree moved there. Red there too, the target is red, not the task: it stays
//! queued behind its project's one open Bug task for the red target and runs Verify again once that task is DONE
//! or FAILED. Green there, the task broke them and goes back to its agent; a run that cannot tell returns it too.

use std::path::Path;

use rusqlite::{OptionalExtension, Transaction};

use super::engine::{place, target_now, worktree};
use super::git;
use super::retry_notes;
use super::runner::Runner;
use super::stages::{how, line_of, vars, Run};
use super::store::{self as bs, Left};
use crate::tasks::integration;
use crate::tasks::integration_queue::and_list;
use crate::tasks::ops::{self, sql, NewTask};
use crate::tasks::store::{Store, DISPATCHER};

/// A queued entry held for a red target.
pub const HELD: &str = "red target";
/// A Bug task `b` still open: what its held tasks wait for.
const OPEN: &str = "b.status NOT IN ('DONE', 'FAILED') AND b.archived_at IS NULL";

/// The task's failed steps started on the target, its Verify alone with what to run again; whether it runs. One
/// that cannot start returns the task, red, as one that cannot tell. `red_attempt`: the task's own run.
pub(super) fn start(store: &Store, runner: &dyn Runner, home: &Path, run: &Run, steps: &[String], red_attempt: &str, now: i64) -> Result<bool, String> {
    let p = place(store, home, &run.project)?;
    let tree = integration::verify_tree_in(home, &run.project, run.number);
    let attempt = crate::auth::crypto::random_hex::<16>();
    let line = line_of("", &p.s.verify);
    let started = if line.is_empty() { Err("Verify is empty".to_string()) } else { target_now(&p) }.and_then(|target| {
        worktree(&p, tree.clone(), &target).map_err(|e| format!("its tree could not be moved there: {e}"))?;
        retry_notes::clear(&tree)?;
        let mut vars = vars(&run.id, &target, &p, &tree);
        vars.extend([("TM_INTEGRATION_RERUN", steps.join(" ")), ("TM_INTEGRATION_RERUN_OF", run.commit.clone())]);
        runner.start(&format!("verify-{}", run.number), &tree, &line, &vars).map(|pane| (target, pane))
    });
    let own = integration::verify_log_in(home, &run.project, run.number, red_attempt);
    let (id, failed) = (&run.id, and_list(&steps.iter().map(String::as_str).collect::<Vec<_>>()));
    store.with(|tx| match &started {
        Ok((target, pane)) => {
            tx.execute("UPDATE integration_verifies SET pane = ?2, started = ?3, attempt = ?4, target_sha = ?5, steps = ?6, red_attempt = ?7 WHERE task = ?1",
                       rusqlite::params![run.task, pane, now, attempt, target, steps.join(" "), red_attempt]).map_err(sql)?;
            let log = integration::verify_log_in(home, &run.project, run.number, &attempt);
            ops::comment(tx, DISPATCHER, id, &format!("{id}'s Verify failed {failed}: they run on {} at {} to tell whether the target is red too. Its log: {}; the target's: {}",
                                                      p.s.target, git::short(target), own.display(), log.display()), now).map(drop)
        }
        Err(why) => {
            bs::not_again(tx, &run.task, &run.commit, now)?;
            bs::give_back(tx, &run.task, &format!("{id} returned: red check: its Verify failed {failed}; its run on {} did not start: {why}, so whether the target is red too is not known. Its log: {}",
                                                  p.s.target, own.display()), Left::Red, now)
        }
    })?;
    Ok(started.is_ok())
}

/// The target's run ended with `code`, naming `red` as its failed steps: the task held, or given back.
pub(super) fn judge(tx: &Transaction, home: &Path, run: &Run, target: &str, code: i32, red: Option<Vec<String>>, now: i64) -> Result<(), String> {
    let s = integration::read(tx, &run.project)?;
    let own = integration::verify_log_in(home, &run.project, run.number, &run.red_attempt);
    let theirs = integration::verify_log_in(home, &run.project, run.number, &run.attempt);
    let logs = format!("Its log: {}; the target's: {}", own.display(), theirs.display());
    let steps: Vec<&str> = run.steps.iter().map(String::as_str).collect();
    let (id, failed, at) = (&run.id, and_list(&steps), format!("{} at {}", s.target, git::short(target)));
    let red = match code { 0 => Some(Vec::new()), 1 => red, _ => None };
    let words = match &red {
        Some(red) if steps.iter().all(|s| red.iter().any(|r| r == s)) => return hold(tx, run, &s.target, target, &failed, &logs, now),
        Some(red) => {
            let passed: Vec<&str> = steps.iter().copied().filter(|s| !red.iter().any(|r| r == s)).collect();
            format!("{id} returned: red check: its Verify failed {failed}; on {at} {} passed, so {id} broke {}. {logs}",
                    and_list(&passed), if passed.len() == 1 { "it" } else { "them" })
        }
        None => {
            let how = if code == 1 { "ended with status 1 naming no failed step".to_string() } else { how(code, run.minutes) };
            format!("{id} returned: red check: its Verify failed {failed}; its run on {at} {how}, so whether the target is red too is not known. {logs}")
        }
    };
    bs::not_again(tx, &run.task, &run.commit, now)?;
    bs::give_back(tx, &run.task, &words, Left::Red, now)
}

/// The task stays queued behind its project's open Bug task for the red target: made, or told of one more task.
fn hold(tx: &Transaction, run: &Run, name: &str, target: &str, failed: &str, logs: &str, now: i64) -> Result<(), String> {
    let (id, short) = (&run.id, git::short(target));
    let bug = match open_bug(tx, &run.project)? {
        Some(bug) => {
            ops::comment(tx, DISPATCHER, &bug, &format!("{id} also waits on this red target: its Verify failed {failed}, which fail on {name} at {short} too. {logs}"), now)?;
            bug
        }
        None => {
            let text = format!("## Problem or goal\n{name} is red at {short}: the Verify steps {failed} fail there. {id}'s Verify failed the same steps, so it waits in INTEGRATION behind this task, with each task held for the same reason. {logs}\n\n\
                ## Expected result\nThose steps green on {name}. When this task is DONE or FAILED, the Verify of each task held behind it runs again.\n\n\
                ## Context\nMade by Dispatcher: a red Verify's failed steps failed on the target too (terminum-integration.md AC-198).");
            let bug = ops::create(tx, DISPATCHER, NewTask { title: format!("Bug: {name} is red: {failed} fail at {short}"), text, project: Some(run.project.clone()),
                severity: Some("P1".into()), ..Default::default() }, now)?;
            tx.execute("UPDATE project_integration SET red_bug = ?2 WHERE project = ?1", (&run.project, &bug.key)).map_err(sql)?;
            bug.id
        }
    };
    tx.execute("UPDATE integration_entries SET stage = ?2 WHERE task = ?1", (&run.task, HELD)).map_err(sql)?;
    ops::notice(tx, DISPATCHER, id, &format!("{id} stays queued: the steps its Verify failed ({failed}) fail on {name} at {short} too, so the target is red, not {id}. \
        It waits until {bug} (the red target) is DONE or FAILED, then its Verify runs again; its wait for integration goes on. {logs}"),
        serde_json::json!({"held": HELD, "bug": bug}), now).map(drop)
}

/// The project's Bug task for its red target while it is open, by its ID: what its held tasks wait for.
pub fn open_bug(tx: &Transaction, project: &str) -> Result<Option<String>, String> {
    tx.query_row(&format!("SELECT b.slug FROM project_integration i JOIN tasks b ON b.id = i.red_bug WHERE i.project = ?1 AND {OPEN}"), [project],
                 |r| r.get(0)).optional().map_err(sql)
}

/// The tasks held for a red target whose Bug task ended (DONE, FAILED, archived or gone) verified again, each told.
pub(super) fn release(tx: &Transaction, now: i64) -> Result<(), String> {
    let held: Vec<(String, String, Option<String>)> = ops::rows(tx, &format!("SELECT e.task, t.slug, b.slug FROM integration_entries e JOIN tasks t ON t.id = e.task
            LEFT JOIN project_integration i ON i.project = e.project LEFT JOIN tasks b ON b.id = i.red_bug
            WHERE e.stage = ?1 AND NOT coalesce({OPEN}, 0)"), [HELD], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    for (task, id, bug) in &held {
        tx.execute("UPDATE integration_entries SET stage = 'verifying' WHERE task = ?1", [task]).map_err(sql)?;
        let ended = bug.as_deref().map_or("The red target's task is gone".to_string(), |b| format!("{b} (the red target) ended"));
        ops::comment(tx, DISPATCHER, id, &format!("{ended}: {id}'s Prepare and Verify run again."), now)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "red_target_tests.rs"]
mod tests;
