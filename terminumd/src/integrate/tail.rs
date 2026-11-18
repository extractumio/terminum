// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A published batch's After and Cleanup (`terminum-integration.md` AC-191, AC-198): one batch's at a time on the
//! machine, in the order they were published, beside the batch in work, so the next batch is built and checked while
//! they run. A project's folder is brought up to the target only when its batch's After's turn comes, never while
//! another After builds in it. A batch whose After failed holds its project's later Afters until a human closes it
//! (AC-194); one whose After said it cannot run now (status 75) is run again after a pause, holding only its
//! project's later Afters, until a cap.

use std::path::Path;

use super::engine::{self, after_after, Place};
use super::git;
use super::runner::Runner;
use super::store as bs;
use crate::tasks::integration;
use crate::tasks::ops::{self, NewTask};
use crate::tasks::store::{Store, DISPATCHER, HUMAN};

/// After's status that says it cannot run now (`EX_TEMPFAIL`): its last rows say why (AC-191).
pub(super) const LATER: i32 = 75;
/// Between an After's tries while it cannot run now, and how long it may wait before it fails (AC-191, §9 Limits):
/// longer than one release run holds the primary checkout (its gate 15 minutes, its checks, the Mac runner's hour).
const PAUSE_S: i64 = 300;
const WAIT_MOST_S: i64 = 6 * 3600;

/// One step of the tail: the running After or Cleanup followed, else the published batch that waited longest takes
/// the turn: its project's folder brought up to the target, then its After or Cleanup, or it ends with neither; a
/// batch whose After waits runs it again once its pause passed.
pub fn step(store: &Store, runner: &dyn Runner, home: &Path, now: i64) -> Result<(), String> {
    let next = store.with(|tx| match bs::in_tail(tx)? {
        Some(b) => Ok(Some((b, true))),
        None => Ok(bs::next_after(tx, now, PAUSE_S)?.map(|b| (b, false))),
    })?;
    let Some((b, running)) = next else { return Ok(()) };
    // turned off before its turn, or its project unreadable: it ends without its After and holds no other project's
    let p = match engine::place(store, home, &b.project) {
        Ok(p) if running || p.s.on => p,
        Err(e) if running => return Err(e),
        found => {
            let why = match found { Ok(p) => format!("integration was turned off for {}", p.name), Err(e) => e };
            return not_run(store, &b, &why, "ended", now);
        }
    };
    if running {
        return engine::follow(store, runner, &p, b, now);
    }
    // its folder moved at its first try: a try again moves nothing under what made it wait; a folder that made it
    // wait, not on the batch's commit yet, is tried again; clean is the move's rule, not After's (AC-191)
    let moved = if b.state == bs::AFTER_WAITS && folder_holds(&p, &b) { Ok(()) } else { folder_follows(&p) };
    let stays = |w: String| format!("{}'s folder stays where it is: {w}", p.name);
    match moved {
        Err(Stays::Dirty(w)) => waits(store, &p, &b, &stays(w), None, now),
        Err(Stays::Off(w)) => not_run(store, &b, &stays(w), after_after(&p.s), now),
        Ok(()) => store.with(|tx| bs::set_state(tx, b.id, if p.s.after.is_empty() { after_after(&p.s) } else { "after" }, None, now)),
    }
}

/// An After lost with its window (closed, or terminum's tmux server ended while it ran) is not run again: what it did
/// is not known (AC-191). Said on the batch's row and its integration task, and by a P1 Bug task on the board, the
/// human's: After works on their machine (an install); they close it once they checked After's work or did it by hand.
/// It holds nothing.
pub(super) fn lost(tx: &rusqlite::Transaction, p: &Place, b: &bs::Batch, now: i64) -> Result<(), String> {
    let (n, name, folder, line) = (b.number, &p.name, p.repo.display(), &p.s.after);
    let tasks = b.tasks.iter().map(|k| bs::slug(tx, k)).collect::<Result<Vec<_>, _>>()?.join(", ");
    let text = format!("## Problem or goal\nBatch {n} of {name} was published ({tasks}) and its After was running in {folder} when its window was lost: \
        closed, or terminum's tmux server ended (a crash, kill-server, a restart of the machine). What After did is not known, and terminum does not \
        run it again.\n\n## Expected result\nAfter's work done once for batch {n}: check it in {folder}; not done or half done, run After's line \
        there by hand, then move this task to DONE. After's line:\n\n```\n{line}\n```\n\n## Context\nMade by Dispatcher: an After lost with its \
        window is not run again (terminum-integration.md AC-191).");
    let bug = ops::create(tx, DISPATCHER, NewTask { title: format!("Bug: After's outcome is not known for batch {n} of {name}"), text,
        project: Some(b.project.clone()), severity: Some("P1".into()), owner: Some(HUMAN.into()), ..Default::default() }, now)?;
    let words = format!("After's outcome is not known for batch {n}: its window was lost. After is not run again. Check its work: {}.", bug.id);
    bs::set_pane(tx, b.id, None, now)?;
    bs::tell_task(tx, b, &words, now)?;
    bs::set_state(tx, b.id, after_after(&p.s), Some(&words), now)
}

/// A published batch's After not run, said on its integration task and its row (AC-191); `next` follows.
fn not_run(store: &Store, b: &bs::Batch, why: &str, next: &str, now: i64) -> Result<(), String> {
    let words = format!("After did not run for batch {}: {why}", b.number);
    store.with(|tx| {
        bs::tell_task(tx, b, &words, now)?;
        bs::set_state(tx, b.id, next, Some(&words), now)
    })
}

/// After's last rows, from the last 4 KiB of its log: cleaned and redacted, for its row and its task (AC-191).
fn last_rows(log: &Path) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let mut tail = Vec::new();
    if let Ok(mut f) = std::fs::File::open(log) {
        let from = f.metadata().map(|m| m.len().saturating_sub(4096)).unwrap_or(0);
        let _ = f.seek(SeekFrom::Start(from)).and_then(|_| f.read_to_end(&mut tail));
    }
    let text = crate::plans::rules::clean(&String::from_utf8_lossy(&tail));
    let rows: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    rows[rows.len().saturating_sub(3)..].join(" / ").chars().take(400).collect()
}

/// A published batch's After ended with `code` (AC-191): 0 its Cleanup follows; LATER it waits; else it failed.
pub(super) fn ended(store: &Store, p: &Place, b: &bs::Batch, code: i32, log: &Path, now: i64) -> Result<(), String> {
    match code {
        0 => store.with(|tx| {
            bs::set_pane(tx, b.id, None, now)?;
            bs::set_state(tx, b.id, after_after(&p.s), None, now)?;
            bs::after_passed(tx, b, now)
        }),
        LATER => {
            let rows = last_rows(log);
            waits(store, p, b, &if rows.is_empty() { format!("status {LATER}") } else { rows }, Some(log), now)
        }
        engine::OVER => failed(store, p, b, Some(&format!("it ran over {} minutes", p.s.check_minutes)), &last_rows(log), Some(log), now),
        code => failed(store, p, b, Some(&format!("status {code}")), &last_rows(log), Some(log), now),
    }
}

/// After cannot run now, `why`: it waits without a human and runs again after its pause; waited past its cap, it
/// failed (AC-191).
fn waits(store: &Store, p: &Place, b: &bs::Batch, why: &str, log: Option<&Path>, now: i64) -> Result<(), String> {
    if b.waits_since.is_some_and(|since| now - since >= WAIT_MOST_S) {
        return failed(store, p, b, None, why, log, now);
    }
    store.with(|tx| {
        if b.waits_since.is_none() {
            bs::tell_task(tx, b, &format!("After waits for batch {}: {why}. It runs again by itself; nothing waits for you.", b.number), now)?;
        }
        bs::after_waits(tx, b, why, now)
    })
}

/// After failed `how`, or (None) waited past its cap: the batch stands "after failed" with its cause and `rows`, its
/// log's last or what it waited for, its integration task waits for a human told what is held and what Close does
/// (AC-191, AC-194).
fn failed(store: &Store, p: &Place, b: &bs::Batch, how: Option<&str>, rows: &str, log: Option<&Path>, now: i64) -> Result<(), String> {
    let waited = format!("waited {} hours", WAIT_MOST_S / 3600);
    let head = match how {
        Some(how) => format!("After failed for batch {} with {how}", b.number),
        None => format!("After {waited} for batch {}", b.number),
    };
    let said = if rows.is_empty() { String::new() } else { format!(": {rows}") };
    let why = format!("{}{said}", how.unwrap_or(&waited));
    let name = &p.name;
    let words = format!("{head}{said}. The batch stays published and its tasks DONE. Held until this task ends: {name}'s next batches are not built, \
                         and the After of every batch published after it waits. Mend the cause, then press After again to run After once more, or do \
                         its work by hand and press Close…: {name}'s batches and later Afters go on.{}",
                         log.map(|l| format!(" Its log: {}", l.display())).unwrap_or_default());
    store.with(|tx| {
        bs::set_pane(tx, b.id, None, now)?;
        bs::set_state(tx, b.id, bs::AFTER_FAILED, Some(&why), now)?;
        bs::wait_for_human(tx, b, &words, now)
    })
}

/// Why the project's folder stays where it is: an uncommitted change, which its batch waits out; or another cause
/// (off its branch, commits of its own, git's refusal), for which After does not run (AC-191).
enum Stays {
    Dirty(String),
    Off(String),
}

/// The project's folder fast-forwarded to the target when it stands clean on its branch (AC-191): any uncommitted
/// change keeps it where it is, untracked files and changes the move would not touch included (CLAUDE.md §9).
fn folder_follows(p: &Place) -> Result<(), Stays> {
    let (remote, branch) = integration::target_of(&p.s.target).map_err(Stays::Off)?;
    let on = git::run(&p.repo, &["branch", "--show-current"]).map_err(Stays::Off)?;
    if on != branch {
        return Err(Stays::Off(format!("it is on {}, not {branch}", if on.is_empty() { "no branch" } else { &on })));
    }
    let dirty = git::uncommitted(&p.repo).map_err(Stays::Off)?;
    if dirty > 0 {
        return Err(Stays::Dirty(format!("{dirty} uncommitted files in it; it moves only when git status --porcelain is empty")));
    }
    git::run(&p.repo, &["merge", "--ff-only", "--quiet", &format!("{remote}/{branch}")]).map(drop).map_err(Stays::Off)
}

/// Whether the project's folder holds the batch's published commit: moved by its first try, or by a later one.
fn folder_holds(p: &Place, b: &bs::Batch) -> bool {
    b.candidate.as_deref().is_some_and(|c| git::ok(&p.repo, &["merge-base", "--is-ancestor", c, "HEAD"]).unwrap_or(false))
}

#[cfg(test)]
#[path = "tail_tests.rs"]
mod tests;
