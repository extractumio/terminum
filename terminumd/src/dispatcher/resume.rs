// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A run its tmux server's end cut off goes on by itself (`terminum-dispatcher.md` AC-205): the walk saw its pane
//! gone under a server with another identity (`plan::step`), and here its conversation resumes as RUN's resume does,
//! in a new window, with fixed words and its open tasks' IDs; once an hour per server at most, so a server its own
//! work ends is not started again and again.

use rusqlite::OptionalExtension;

use super::api::Press;
use super::runs::{How, Run};
use super::{fail, say, store};
use crate::app::Shared;
use crate::tasks::ops::{self, sql, Task};
use crate::tasks::{agents, store::HUMAN};

/// After a resume, how long another end of the same server resumes nothing.
pub const AGAIN_WITHIN: i64 = 3600;
pub const RESUMES: &str = "Dispatcher resumes it: its tmux server ended and started anew.";
pub const ENDED_AGAIN: &str = "Its tmux server ended again within an hour of resuming it: run it once the cause is found.";

/// What the resumed agent hears, through the gate: its open tasks' IDs and fixed words, approved by the maintainer.
pub fn words(ids: &[String]) -> String {
    format!("[terminum Dispatcher] {}: your session ended because terminum's tmux server stopped (a crash or a restart of the machine), and Dispatcher resumed it. \
        Your conversation shows what was done: do not redo finished steps. Programs and servers you ran in the background are gone. \
        Before you go on with the task, check your working tree (git status, files half written, a stale .git/index.lock) and start again what you still need.", ids.join(", "))
}

/// The server's last resume (the identity it took, when) against this end's `identity`: what to keep, or None when
/// another end came within the hour of it. The runs one end cut off resume together, by the identity it took.
pub fn breaker(last: Option<(String, i64)>, identity: &str, now: i64) -> Option<(String, i64)> {
    match last {
        Some((was, at)) if was == identity => Some((was, at)),
        Some((_, at)) if now - at < AGAIN_WITHIN => None,
        _ => Some((identity.to_string(), now)),
    }
}

/// The store's key for a server's last resume: this Mac's, or a host's.
fn meta_key(r: &Run) -> String {
    format!("resumed:{}", r.host.as_deref().unwrap_or("local"))
}

/// Run `r`, cut off by its server's end and back as `identity`, resumes, or is FAILED with why it does not.
pub(super) async fn after_restart(app: &Shared, r: Run, identity: String) {
    let (run, now) = (r.clone(), ops::now());
    let ready = store(app, move |tx| {
        let key = meta_key(&run);
        let kept: Option<String> = tx.query_row("SELECT value FROM meta WHERE key = ?1", [&key], |x| x.get(0)).optional().map_err(sql)?;
        let last = kept.and_then(|v| serde_json::from_str::<(String, i64)>(&v).ok());
        let Some(now_kept) = breaker(last, &identity, now) else {
            fail(tx, &run, ENDED_AGAIN)?;
            return Ok(None);
        };
        tx.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)", [&key, &serde_json::json!(now_kept).to_string()]).map_err(sql)?;
        if agents::in_project(tx, &run.agent_id, &run.project)?.is_none() {
            fail(tx, &run, &format!("Not resumed after its tmux server started anew: {} is no longer in the pool.", run.agent_name))?;
            return Ok(None);
        }
        // its tasks in progress, as they stand now: one moved meanwhile stays where it went
        let tasks: Vec<Task> = run.open.iter().map(|id| ops::get(tx, id)).collect::<Result<Vec<_>, _>>()?.into_iter().filter(|t| t.status == "IN PROGRESS").collect();
        if tasks.is_empty() {
            return Ok(None);
        }
        let started: (Option<String>, Option<String>) = tx.query_row("SELECT started_by, started_how FROM runs WHERE id = ?1", [&run.id], |x| Ok((x.get(0)?, x.get(1)?))).map_err(sql)?;
        say(tx, &run, RESUMES)?;
        Ok(Some((tasks, started)))
    }).await;
    match ready {
        Ok(Some((tasks, (by, how)))) => {
            let ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
            // who started it and how stay its own (AC-183): the server's end was no one's press
            let press = Press::resumed(how.as_deref().and_then(How::of_word).unwrap_or_default(), by.as_deref().unwrap_or(HUMAN));
            if let Err(why) = super::goes_on::resume(app, r.clone(), &tasks, words(&ids), press).await {
                let (run, words) = (r.clone(), format!("Not resumed after its tmux server started anew: {why}"));
                let _ = store(app, move |tx| fail(tx, &run, &words)).await;
            }
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(event = "dispatcher.resume", run = %r.id, error = %e),
    }
    super::told_all(app, &r);
}

#[cfg(test)]
#[path = "resume_tests.rs"]
mod tests;
