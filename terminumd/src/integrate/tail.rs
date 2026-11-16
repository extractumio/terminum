// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A published batch's After and Cleanup (`terminum-integration.md` AC-191, AC-198): one batch's at a time on the
//! machine, in the order they were published, beside the batch in work, so the next batch is built and checked while
//! they run. A project's folder is brought up to the target only when its batch's After's turn comes, never while
//! another After builds in it. A batch whose After failed holds its project's later Afters until a human closes it
//! (AC-194).

use std::path::Path;

use super::engine::{self, after_after, Place};
use super::git;
use super::runner::Runner;
use super::store as bs;
use crate::tasks::integration;
use crate::tasks::store::Store;

/// One step of the tail: the running After or Cleanup followed, else the published batch that waited longest takes
/// the turn: its project's folder brought up to the target, then its After or Cleanup, or it ends with neither.
pub fn step(store: &Store, runner: &dyn Runner, home: &Path, now: i64) -> Result<(), String> {
    let next = store.with(|tx| match bs::in_tail(tx)? {
        Some(b) => Ok(Some((b, true))),
        None => Ok(bs::next_published(tx)?.map(|b| (b, false))),
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
    match folder_follows(&p) {
        Err(w) => not_run(store, &b, &format!("{}'s folder stays where it is: {w}", p.name), after_after(&p.s), now),
        Ok(()) => store.with(|tx| bs::set_state(tx, b.id, if p.s.after.is_empty() { after_after(&p.s) } else { "after" }, None, now)),
    }
}

/// A published batch's After not run, said on its integration task and its row (AC-191); `next` follows.
fn not_run(store: &Store, b: &bs::Batch, why: &str, next: &str, now: i64) -> Result<(), String> {
    let words = format!("After did not run for batch {}: {why}", b.number);
    store.with(|tx| {
        bs::tell_task(tx, b, &words, now)?;
        bs::set_state(tx, b.id, next, Some(&words), now)
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
