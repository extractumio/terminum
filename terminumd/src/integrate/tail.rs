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
    // its folder moved at its first try: a try again moves nothing under what made it wait
    let moved = if b.state == bs::AFTER_WAITS { Ok(()) } else { folder_follows(&p) };
    match moved {
        Err(w) => not_run(store, &b, &format!("{}'s folder stays where it is: {w}", p.name), after_after(&p.s), now),
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
        LATER if b.waits_since.is_some_and(|since| now - since >= WAIT_MOST_S) => failed(store, p, b, None, log, now),
        LATER => {
            let rows = last_rows(log);
            let why = if rows.is_empty() { format!("status {LATER}") } else { rows };
            store.with(|tx| {
                if b.waits_since.is_none() {
                    bs::tell_task(tx, b, &format!("After waits for batch {}: {why}. It runs again by itself; nothing waits for you.", b.number), now)?;
                }
                bs::after_waits(tx, b, &why, now)
            })
        }
        engine::OVER => failed(store, p, b, Some(&format!("it ran over {} minutes", p.s.check_minutes)), log, now),
        code => failed(store, p, b, Some(&format!("status {code}")), log, now),
    }
}

/// After failed `how`, or (None) waited past its cap: the batch stands "after failed" with its cause and last rows,
/// its integration task waits for a human told what is held and what Close does (AC-191, AC-194).
fn failed(store: &Store, p: &Place, b: &bs::Batch, how: Option<&str>, log: &Path, now: i64) -> Result<(), String> {
    let waited = format!("waited {} hours", WAIT_MOST_S / 3600);
    let head = match how {
        Some(how) => format!("After failed for batch {} with {how}", b.number),
        None => format!("After {waited} for batch {}", b.number),
    };
    let rows = last_rows(log);
    let said = if rows.is_empty() { String::new() } else { format!(": {rows}") };
    let why = format!("{}{said}", how.unwrap_or(&waited));
    let name = &p.name;
    let words = format!("{head}{said}. The batch stays published and its tasks DONE. Held until this task ends: {name}'s next batches are not built, \
                         and the After of every batch published after it waits. Mend the cause, then press After again to run After once more, or do \
                         its work by hand and press Close…: {name}'s batches and later Afters go on. Its log: {}", log.display());
    store.with(|tx| {
        bs::set_pane(tx, b.id, None, now)?;
        bs::set_state(tx, b.id, bs::AFTER_FAILED, Some(&why), now)?;
        bs::wait_for_human(tx, b, &words, now)
    })
}

/// The project's folder fast-forwarded to the target when it stands clean on its branch (AC-191).
fn folder_follows(p: &Place) -> Result<(), String> {
    let (remote, branch) = integration::target_of(&p.s.target)?;
    let on = git::run(&p.repo, &["branch", "--show-current"])?;
    if on != branch {
        return Err(format!("it is on {}, not {branch}", if on.is_empty() { "no branch" } else { &on }));
    }
    git::run(&p.repo, &["merge", "--ff-only", "--quiet", &format!("{remote}/{branch}")]).map(drop)
}

#[cfg(test)]
#[path = "tail_tests.rs"]
mod tests;
