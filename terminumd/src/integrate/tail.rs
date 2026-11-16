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
    if let Some(b) = store.with(bs::in_tail)? {
        let p = engine::place(store, home, &b.project)?;
        return engine::follow(store, runner, &p, b, now);
    }
    let Some(b) = store.with(bs::next_published)? else { return Ok(()) };
    let p = engine::place(store, home, &b.project)?;
    let moved = folder_follows(&p);
    store.with(|tx| {
        if let Err(w) = &moved {
            bs::tell_task(tx, &b, &format!("After did not run for batch {}: {}'s folder stays where it is: {w}", b.number, p.name), now)?;
        }
        let next = if p.s.after.is_empty() || moved.is_err() { after_after(&p.s) } else { "after" };
        bs::set_state(tx, b.id, next, None, now)
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
