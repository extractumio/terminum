// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A run its tmux server's end cut off goes on by itself (`terminum-dispatcher.md` AC-205): the walk saw its pane
//! gone under a server with another identity (`plan::step`), and here its conversation resumes as RUN's resume does,
//! in a new window, with fixed words and its open tasks' IDs; once an hour per server at most, so a server its own
//! work ends is not started again and again.

use rusqlite::OptionalExtension;

use super::api::Press;
use super::runs::{self, Phase, Run};
use super::{fail, say, store};
use crate::app::Shared;
use crate::tasks::ops::{self, sql, Task};
use crate::tasks::agents;

/// After a resume, how long another end of the same server resumes nothing.
const AGAIN_WITHIN: i64 = 3600;
const RESUMES: &str = "Dispatcher resumes it: its tmux server ended and started anew.";
const ENDED_AGAIN: &str = "Its tmux server ended again within an hour of resuming it: run it once the cause is found.";

/// What the resumed agent hears, through the gate: its open tasks' IDs and fixed words, approved by the maintainer.
fn words(ids: &[String]) -> String {
    format!("[terminum Dispatcher] {}: your session ended because terminum's tmux server stopped (a crash or a restart of the machine), and Dispatcher resumed it. \
        Your conversation shows what was done: do not redo finished steps. Programs and servers you ran in the background are gone. \
        Before you go on with the task, check your working tree (git status, files half written, a stale .git/index.lock) and start again what you still need.", ids.join(", "))
}

/// The server's last resume (the identity it took, when) against this end's `identity`: what to keep, or None when
/// another end came within the hour of it. The runs one end cut off resume together, by the identity it took.
fn breaker(last: Option<(String, i64)>, identity: &str, now: i64) -> Option<(String, i64)> {
    match last {
        Some((was, at)) if was == identity => Some((was, at)),
        Some((_, at)) if now - at < AGAIN_WITHIN => None,
        _ => Some((identity.to_string(), now)),
    }
}

/// Why a run its server's end cut off does not go on, on its tasks.
fn not_resumed(why: &str) -> String {
    format!("Not resumed after its tmux server started anew: {why}")
}

/// The store's key for a server's last resume: this Mac's, or a host's.
fn meta_key(r: &Run) -> String {
    format!("resumed:{}", r.host.as_deref().unwrap_or("local"))
}

/// Run `r`, cut off by its server's end and back as `identity`, resumes, or is FAILED with why it does not.
pub(super) async fn after_restart(app: &Shared, r: Run, identity: String) {
    // A refused resume releases its old run's slot before the next recovery checks the pool.
    let _turn = app.dispatcher.restart_recovery.lock().await;
    let (run, now) = (r.clone(), ops::now());
    let ready = store(app, move |tx| {
        let key = meta_key(&run);
        let kept: Option<String> = tx.query_row("SELECT value FROM meta WHERE key = ?1", [&key], |x| x.get(0)).optional().map_err(sql)?;
        let last = kept.and_then(|v| serde_json::from_str::<(String, i64)>(&v).ok());
        let Some(now_kept) = breaker(last, &identity, now) else {
            fail(tx, &run, ENDED_AGAIN)?;
            return Ok(None);
        };
        if agents::in_project(tx, &run.agent_id, &run.project)?.is_none() {
            fail(tx, &run, &not_resumed(&format!("{} is no longer in the pool.", run.agent_name)))?;
            return Ok(None);
        }
        // its tasks in progress, as they stand now: one moved meanwhile stays where it went
        let mut tasks: Vec<Task> = Vec::new();
        for id in &run.open {
            let t = ops::get(tx, id)?;
            if t.status == "IN PROGRESS" {
                tasks.push(t);
            }
        }
        if tasks.is_empty() {
            return Ok(None);
        }
        // the hour counts from this end's first resume, not from a run that could not be resumed
        tx.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)", [&key, &serde_json::to_string(&now_kept).map_err(|e| e.to_string())?]).map_err(sql)?;
        let started = runs::started(tx, &run.id)?;
        say(tx, &run, RESUMES)?;
        Ok(Some((tasks, started)))
    }).await;
    match ready {
        Ok(Some((tasks, (how, by)))) => {
            let ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
            // who started it and how stay its own (AC-183): the server's end was no one's press
            let press = Press::resumed(how, &by);
            // no room is no queue: no human agreed to wait (AC-169); the limit's words say why
            let missed = match super::goes_on::resume(app, r.clone(), &tasks, words(&ids), press).await {
                Ok(v) => v["full"]["words"].as_str().map(str::to_string),
                Err(why) => Some(why),
            };
            let Some(why) = missed else { return super::told_all(app, &r) };   // its resume closed the old run
            let (run, words) = (r.clone(), not_resumed(&why));
            let _ = store(app, move |tx| fail(tx, &run, &words)).await;
        }
        Ok(None) => {}
        Err(e) => return tracing::warn!(event = "dispatcher.resume", run = %r.id, error = %e),
    }
    // tried: the old run is over, however it went (a resume closed it already)
    let id = r.id.clone();
    let _ = store(app, move |tx| runs::change(tx, &id, |was| if !was.is_over() { was.state = Phase::Closed; was.pane_key = None; }).map(drop)).await;
    super::told_all(app, &r);
}

#[cfg(test)]
#[path = "resume_tests.rs"]
mod tests;
