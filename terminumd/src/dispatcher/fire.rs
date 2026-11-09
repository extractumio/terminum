// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A schedule's firings (`terminum-schedules.md` AC-162): on each walk, a schedule due makes a task from itself as
//! it is then and runs it on its agent, as RUN's route would; its last task still open skips it; missed firings come
//! as one; a start that fails pauses it; a task made and never run, terminumd having ended between, runs once.

use serde_json::json;

use super::press::{budget_words, ASK};
use super::{pool, runs, store, DISPATCHER};
use crate::app::Shared;
use crate::tasks::cron;
use crate::tasks::ops::{self, NewTask};
use crate::tasks::schedules::{self, Schedule};

/// Over this, a firing is said to have come late: the Mac slept or terminumd was not running.
const LATE: i64 = 120;

/// What a scheduled run's agent is told (AC-162): only the schedule's own words, as its task holds them.
pub fn prompt(s: &Schedule, task: &str, title: &str, body: &str) -> String {
    let words = s.words();
    let cadence = words.chars().next().map_or(String::new(), |c| c.to_lowercase().chain(words.chars().skip(1)).collect());
    let mut parts = vec![format!("Task {task}: {title}. A scheduled run of terminum's schedule {} \"{}\" ({cadence}), set by a human; no human started this run or watches it now.", s.key, s.title)];
    if !body.trim().is_empty() {
        parts.push(body.to_string());
    }
    parts.push(budget_words(s.budget_seconds));
    parts.push(["This is a scheduled run: its result is read later on the board. When it is done, call finish_task with the result in this form, each on its own line:",
                "Outcome: ok, attention or failed", "Summary: one or two sentences", "Details: what was checked or done", "For the human: what they should do, or nothing"].join("\n"));
    parts.push(ASK.to_string());
    parts.join("\n\n")
}

/// What a due schedule comes to now.
enum Next {
    /// a task made: its ID, its firing
    Run(String, i64),
    /// its last run's idle session holds its slot: closed first, then decided again
    Close(runs::Run),
    Nothing,
}

/// Decides one schedule due by `now`, in one transaction.
fn decide(tx: &rusqlite::Transaction, s: &Schedule, now: i64, closed: bool) -> Result<Next, String> {
    let Some((due, missed)) = schedules::by_id(tx, &s.id)?.due(now) else { return Ok(Next::Nothing) };
    let advance = |tx: &rusqlite::Transaction| tx.execute("UPDATE schedules SET last_due = ?2 WHERE id = ?1", (&s.id, due)).map(drop).map_err(ops::sql);
    if let Some(last) = schedules::last_task(tx, s)? {
        if !super::plan::finished(&last.status) && last.archived_at.is_none() {
            advance(tx)?;
            schedules::fired(tx, s, due, missed, None, "skipped", Some(&format!("Skipped: {} is still open.", last.id)), now)?;
            return Ok(Next::Nothing);
        }
        if let Some(r) = runs::last_of(tx, &last.key)?.filter(runs::Run::holds_slot) {
            if !closed && pool::closable(tx, &r)? {
                return Ok(Next::Close(r));
            }
            advance(tx)?;
            schedules::fired(tx, s, due, missed, None, "skipped", Some(&format!("Skipped: {}'s session is still busy.", last.id)), now)?;
            return Ok(Next::Nothing);
        }
    }
    advance(tx)?;
    let t = ops::create(tx, DISPATCHER, NewTask { title: s.title.clone(), text: s.body.clone(), project: Some(s.project.clone()), severity: Some(s.severity.clone()), ..Default::default() }, now)?;
    ops::set_budget(tx, DISPATCHER, &t.id, Some(s.budget_seconds / 60), now)?;
    let late = if now - due > LATE { format!(", run {}: the Mac was asleep or terminumd was not running", cron::when_words(now).rsplit(' ').next().unwrap_or_default()) } else { String::new() };
    ops::log(tx, &t.key, DISPATCHER, "why", &format!("Dispatcher made it from schedule {}, due {}{late}.", s.key, cron::when_words(due)), None, now)?;
    let fire = schedules::fired(tx, s, due, missed, Some(&t.key), "made", None, now)?;
    if s.once_at.is_some() {
        schedules::pause(tx, &s.id, "ran once", now)?;
    }
    Ok(Next::Run(t.id, fire))
}

/// Runs a schedule's task on its agent as RUN's route would; a refusal fails the task and pauses the schedule.
async fn start(app: &Shared, s: &Schedule, task: &str, fire: i64) {
    let started = super::api::press_again(app, task, json!({"agent": s.agent})).await;
    let (id, sid, now) = (task.to_string(), s.id.clone(), ops::now());
    let _ = store(app, move |tx| match started {
        Ok(_) => {
            let t = ops::get(tx, &id)?;
            schedules::fire_prompt(tx, fire, &super::press::prompt_of(tx, std::slice::from_ref(&t), None)?)?;
            schedules::fire_is(tx, fire, "started", None)
        }
        Err(why) => {
            let words = format!("Paused: {why}");
            if !super::plan::finished(&ops::get(tx, &id)?.status) {
                super::move_one(tx, &id, "FAILED", &format!("It did not start: {why}"))?;
            }
            schedules::pause(tx, &sid, &words, now)?;
            schedules::fire_is(tx, fire, "failed", Some(&words))
        }
    }).await;
    crate::tasks::api::told(app, task);
}

/// Each schedule due, and each task made but never run (AC-162). Called by the walk, one at a time.
pub async fn due(app: &Shared) {
    let Ok((all, unstarted)) = store(app, |tx| Ok((schedules::all(tx)?, schedules::unstarted(tx)?))).await else { return };
    let mut heard = !unstarted.is_empty();
    for (fire, task) in unstarted {
        let found = store(app, move |tx| { let t = ops::get(tx, &task)?; let owner = schedules::of_task(tx, &t.key)?; Ok((t, owner)) }).await;
        match found {
            Ok((t, Some(owner))) if t.status == "TODO" && t.archived_at.is_none() => start(app, &owner, &t.id, fire).await,
            _ => { let _ = store(app, move |tx| schedules::fire_is(tx, fire, "called off", Some("Its task was moved or archived before it ran."))).await; }
        }
    }
    let now = ops::now();
    for s in all.into_iter().filter(|s| s.due(now).is_some()) {
        heard = true;
        let mut closed = false;
        loop {
            let (one, at) = (s.clone(), ops::now());
            match store(app, move |tx| decide(tx, &one, at, closed)).await {
                Ok(Next::Run(task, fire)) => start(app, &s, &task, fire).await,
                Ok(Next::Close(r)) if !closed => {
                    closed = true;
                    if super::api::close_idle(app, &r.id, &s.key).await.is_ok() {
                        continue;
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(event = "dispatcher.schedule", schedule = %s.key, error = %e),
            }
            break;
        }
    }
    if heard {
        crate::tasks::schedulesapi::heard(app);
    }
}

/// A task of schedule `id` waiting for room is called off, `words` saying why (AC-160).
pub fn called_off(tx: &rusqlite::Transaction, id: &str, words: &str) -> Result<(), String> {
    let keys: Vec<String> = ops::rows(tx, "SELECT f.task FROM schedule_fires f JOIN run_waits w ON w.task = f.task WHERE f.schedule = ?1", [id], |r| r.get(0))?;
    keys.iter().try_for_each(|k| super::waits::call_off(tx, k, words))
}

#[cfg(test)]
#[path = "fire_tests.rs"]
mod tests;
