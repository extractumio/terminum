// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A red Verify told apart (`terminum-integration.md` AC-198): the steps a task's Verify names as failed run again
//! on the target, in the task's verify tree moved there. Red there too, the target is red, not the task: it stays
//! queued behind its project's one open Bug task for the red target and runs Verify again once that task is DONE
//! or FAILED. Green there, the task broke them and goes back to its agent; a run that cannot tell returns it too.
//! A held task that leaves the queue meanwhile is told so, and told again to integrate when that task ends.

use std::path::Path;

use rusqlite::{OptionalExtension, Transaction};

use super::engine::{how_ended, place, target_now, worktree, Place};
use super::git;
use super::retry_notes;
use super::runner::Runner;
use super::stages::{line_of, vars, Run};
use super::store::{self as bs, Left};
use crate::tasks::integration;
use crate::tasks::integration_queue::and_list;
use crate::tasks::ops::{self, sql, NewTask, Task};
use crate::tasks::store::{Store, DISPATCHER};

/// A queued entry held for a red target.
pub const HELD: &str = "red target";
/// The SQL of task alias `$t` still open, neither DONE nor FAILED nor archived; one definition for the red target's readers.
macro_rules! open {
    ($t:literal) => { concat!($t, ".status NOT IN ('DONE', 'FAILED') AND ", $t, ".archived_at IS NULL") };
}
pub(super) use open;

/// A Bug task `b` still open: what its held tasks wait for.
const OPEN: &str = open!("b");

/// Verify alone started in `tree` on `target`, told to run `steps` of `of`'s plan again (land_stages.py verify);
/// the tree moved there first. Its pane, or why it did not start.
pub(super) fn launch(p: &Place, runner: &dyn Runner, tree: &Path, (id, number): (&str, i64), target: &str, steps: &str, of: &str) -> Result<String, String> {
    let line = line_of("", &p.s.verify);
    if line.is_empty() {
        return Err("Verify is empty".to_string());
    }
    worktree(p, tree.to_path_buf(), target).map_err(|e| format!("its tree could not be moved there: {e}"))?;
    retry_notes::clear(tree)?;
    let mut vars = vars(id, target, p, tree);
    vars.extend([("TM_INTEGRATION_RERUN", steps.to_string()), ("TM_INTEGRATION_RERUN_OF", of.to_string())]);
    runner.start(&format!("verify-{number}"), tree, &line, &vars)
}

/// The task's failed steps started on the target, its Verify alone with what to run again; whether it runs. One
/// that cannot start, whatever stops it, returns the task, red, as one that cannot tell. `red_attempt`: the task's own run.
pub(super) fn start(store: &Store, runner: &dyn Runner, home: &Path, run: &Run, steps: &[String], red_attempt: &str, now: i64) -> Result<bool, String> {
    let tree = integration::verify_tree_in(home, &run.project, run.number);
    let attempt = crate::auth::crypto::random_hex::<16>();
    let started = place(store, home, &run.project).and_then(|p| {
        let target = if line_of("", &p.s.verify).is_empty() { Err("Verify is empty".to_string()) } else { target_now(&p) }?;
        launch(&p, runner, &tree, (&run.id, run.number), &target, &steps.join(" "), &run.commit).map(|pane| (p.s.target.clone(), target, pane))
    });
    let own = integration::verify_log_in(home, &run.project, run.number, red_attempt);
    let (id, failed) = (&run.id, and_list(&steps.iter().map(String::as_str).collect::<Vec<_>>()));
    store.with(|tx| match &started {
        Ok((name, target, pane)) => {
            tx.execute("UPDATE integration_verifies SET pane = ?2, started = ?3, attempt = ?4, target_sha = ?5, steps = ?6, red_attempt = ?7 WHERE task = ?1",
                       rusqlite::params![run.task, pane, now, attempt, target, steps.join(" "), red_attempt]).map_err(sql)?;
            let log = integration::verify_log_in(home, &run.project, run.number, &attempt);
            ops::comment(tx, DISPATCHER, id, &format!("{id}'s Verify failed {failed}: they run on {} at {} to tell whether the target is red too. Its log: {}; the target's: {}",
                                                      name, git::short(target), own.display(), log.display()), now).map(drop)
        }
        Err(why) => {
            bs::not_again(tx, &run.task, &run.commit, now)?;
            bs::give_back(tx, &run.task, &format!("{id} returned: red check: its Verify failed {failed}; its run on the target did not start: {why}, so whether the target is red too is not known. Its log: {}",
                                                  own.display()), Left::Red, now)
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
    let all = red.as_ref().is_some_and(|red| steps.iter().all(|s| red.iter().any(|r| r == s)));
    // the red target's own task, red still: its fix does not mend them yet, and it never waits behind itself
    if red.as_ref().is_some_and(|failed| !failed.is_empty()) {
        crate::supervisor::misses::target_red(tx, &run.project, &s.target, target, &run.task, Some(&run.red_attempt), now)?;
    }
    let itself = all && open_bug(tx, &run.project)?.as_deref() == Some(id.as_str());
    let words = match &red {
        Some(_) if itself => format!("{id} returned: red check: its Verify failed {failed}, which fail on {at} too, and {id} is the red target's own task: its commit does not mend them yet. {logs}"),
        Some(_) if all => return hold(tx, run, &s.target, target, &failed, &logs, now),
        Some(red) => {
            let passed: Vec<&str> = steps.iter().copied().filter(|s| !red.iter().any(|r| r == s)).collect();
            format!("{id} returned: red check: its Verify failed {failed}; on {at} {} passed, so {id} broke {}. {logs}",
                    and_list(&passed), if passed.len() == 1 { "it" } else { "them" })
        }
        None => {
            let how = if code == 1 { "ended with status 1 naming no failed step".to_string() } else { how_ended(code, run.minutes) };
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
            // an ended Bug task's held tasks go back to Verify before the new one is made, never waiting behind it untold
            release(tx, now)?;
            let text = format!("## Problem or goal\n{name} is red at {short}: the Verify steps {failed} fail there. {id}'s Verify failed the same steps, so it waits in INTEGRATION behind this task, with each task held for the same reason. {logs}\n\n\
                ## Expected result\nThose steps green on {name}. When this task is DONE or FAILED, the Verify of each task held behind it runs again.\n\n\
                ## Context\nMade by Dispatcher: a red Verify's failed steps failed on the target too (terminum-integration.md AC-198).");
            let bug = ops::create(tx, DISPATCHER, NewTask { title: format!("Bug: {name} is red: {failed} fail at {short}"), text, project: Some(run.project.clone()),
                severity: Some("P1".into()), ..Default::default() }, now)?;
            tx.execute("UPDATE project_integration SET red_bug = ?2 WHERE project = ?1", (&run.project, &bug.key)).map_err(sql)?;
            // terminumd runs the steps on the target again by itself, so a red of a busy moment holds no one (red_recheck)
            super::red_recheck::keep(tx, &run.project, &run.steps, &run.commit, now)?;
            bug.id
        }
    };
    tx.execute("UPDATE integration_entries SET stage = ?2 WHERE task = ?1", (&run.task, HELD)).map_err(sql)?;
    ops::notice(tx, DISPATCHER, id, &format!("{id} stays queued: the steps its Verify failed ({failed}) fail on {name} at {short} too, so the target is red, not {id}. \
        It waits until {bug} (the red target) is DONE or FAILED, then its Verify runs again; its wait for integration goes on while it stays in INTEGRATION. {logs}"),
        serde_json::json!({"held": HELD, "bug": bug, "target_red": true}), now).map(drop)
}

/// The project's Bug task for its red target, (key, ID, open), while it is a task.
fn red_bug(tx: &Transaction, project: &str) -> Result<Option<(String, String, bool)>, String> {
    tx.query_row(&format!("SELECT b.id, b.slug, {OPEN} FROM project_integration i JOIN tasks b ON b.id = i.red_bug WHERE i.project = ?1"), [project],
                 |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(sql)
}

/// The project's Bug task for its red target while it is open, by its ID: what its held tasks wait for.
pub fn open_bug(tx: &Transaction, project: &str) -> Result<Option<String>, String> {
    Ok(red_bug(tx, project)?.filter(|b| b.2).map(|b| b.1))
}

/// The open Bug task's row key: the Supervisor's evidence of a red target (`terminum-supervisor.md` AC-212).
pub(crate) fn open_bug_key(tx: &Transaction, project: &str) -> Result<Option<String>, String> {
    Ok(red_bug(tx, project)?.filter(|b| b.2).map(|b| b.0))
}

/// Held task `task` left INTEGRATION for `status`: anything but DONE or FAILED, it is told the Bug task's end runs
/// nothing for it, and is kept to be told to integrate again at that end (`release`); the Bug task hears of it.
pub(crate) fn left(tx: &Transaction, task: &Task, status: &str, at: i64) -> Result<(), String> {
    if matches!(status, "DONE" | "FAILED") {
        return Ok(());
    }
    let (id, out) = (&task.id, Left::Out.word());
    match red_bug(tx, &task.project)? {
        Some((key, bug, true)) => {
            ops::left_queue(tx, DISPATCHER, id, &format!("{id} left the queue while it waited behind {bug} (the red target): it is not queued now, so {bug}'s end runs nothing for it. \
                Integrate again to land it; {id} is told when {bug} ends."), out, at)?;
            ops::comment(tx, DISPATCHER, &bug, &format!("{id} left the queue while it waited on this red target: when this task ends, {id} is told to integrate again."), at)?;
            tx.execute("INSERT OR REPLACE INTO integration_held_left (task, bug) VALUES (?1, ?2)", (&task.key, &key)).map(drop).map_err(sql)
        }
        // the Bug task ended, or is gone, before this beat's release
        bug => ops::left_queue(tx, DISPATCHER, id, &integrate_again(bug.map(|b| b.1).as_deref(), id), out, at).map(drop),
    }
}

/// How a task released from a red target, or told to integrate again, names that target's end.
fn ended(bug: Option<&str>) -> String {
    bug.map_or("The red target's task is gone".to_string(), |b| format!("{b} (the red target) ended"))
}

fn integrate_again(bug: Option<&str>, id: &str) -> String {
    format!("{}: {id} left the queue while it waited behind it, so nothing runs for it. Integrate again to land it.", ended(bug))
}

/// The tasks held for a red target whose Bug task ended (DONE, FAILED, archived or gone) verified again, each told;
/// each that left the queue while held behind it, open still, told once to integrate again.
pub(super) fn release(tx: &Transaction, now: i64) -> Result<(), String> {
    let held: Vec<(String, String, Option<String>)> = ops::rows(tx, &format!("SELECT e.task, t.slug, b.slug FROM integration_entries e JOIN tasks t ON t.id = e.task
            LEFT JOIN project_integration i ON i.project = e.project LEFT JOIN tasks b ON b.id = i.red_bug
            WHERE e.stage = ?1 AND NOT coalesce({OPEN}, 0)"), [HELD], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    for (task, id, bug) in &held {
        tx.execute("UPDATE integration_entries SET stage = 'verifying' WHERE task = ?1", [task]).map_err(sql)?;
        ops::comment(tx, DISPATCHER, id, &format!("{}: {id}'s Prepare and Verify run again.", ended(bug.as_deref())), now)?;
    }
    let to_tell: Vec<(String, String, String, String, bool)> = ops::rows(tx, &format!("SELECT h.task, t.slug, b.id, b.slug, t.status IN ('DONE', 'FAILED') OR t.archived_at IS NOT NULL
            FROM integration_held_left h JOIN tasks t ON t.id = h.task JOIN tasks b ON b.id = h.bug WHERE NOT ({OPEN})"), [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
    for (task, id, key, bug, closed) in &to_tell {
        tx.execute("DELETE FROM integration_held_left WHERE task = ?1", [task]).map_err(sql)?;
        if !closed {
            ops::notice(tx, DISPATCHER, id, &integrate_again(Some(bug), id), serde_json::json!({"requeue": key}), now)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "red_target_tests.rs"]
mod tests;
