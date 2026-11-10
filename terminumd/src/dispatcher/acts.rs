// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What a walk leaves to do outside its transaction (`terminum-dispatcher.md` AC-131): typing, ending
//! an agent, closing a pane, and what is said of them.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::plan::{self, Act};
use super::runs::{self, Phase, Run};
use super::{gate, procs, say, scale, start, store, BEAT};
use crate::app::Shared;
use crate::tasks::{files, ops};
use crate::tmux::Server;

/// What the transaction left to do outside it; `at`: when the walk read what decided it.
pub(super) struct Later {
    pub(super) run: Run,
    pub(super) act: Act,
    pub(super) at: Instant,
}

/// Closes the pane: its mark checked just before; `keep`: the screen's last 200 rows stay on the task. Last of all,
/// under the pane's typing queue through tmux's kill: no human's input since `at` (what decided it was read), and
/// `still`, of the run as it stands; else it stays, and why comes back. tmux refusing: one more try at the next beat,
/// then it stays and says so (AC-125, AC-131).
pub(super) async fn close(app: &Shared, run: &Run, (server, pane): (&Arc<Server>, &str), words: &str, keep: bool, at: Instant, still: fn(&rusqlite::Transaction, &Run) -> Result<bool, String>) -> Result<Option<String>, String> {
    start::is_marked(server, pane, &run.id).await?;
    if keep {
        let rows = server.run(format!("capture-pane -p -S -200 -t {pane}")).await.unwrap_or_default();
        let text = crate::sent::redact(&rows.join("\n"));
        if !text.trim().is_empty() {
            let (a, task, name, at) = (app.clone(), run.ids.first().cloned().unwrap_or_default(), format!("{}-screen.txt", run.names().replace(", ", "-")), ops::now());
            let kept = crate::http::off(move || {
                let f = files::keep(&a.tasks, &a.cfg.dir, &name, text.as_bytes(), at)?;
                a.tasks.with(|tx| files::attach(tx, std::slice::from_ref(&f.id), &task, None))
            }).await;
            if let Err(e) = kept {
                tracing::warn!(event = "dispatcher.close", task = %run.names(), error = %e);
            }
        }
    }
    // a human's keys, RUN or a human's prompt may have come since the walk decided: read again, last of all (AC-131)
    let Some(_held) = app.inputs.act_queue(server, pane, &server.key(pane), at).await else { return Ok(Some("a human typed in it".into())) };
    let wanted = run.id.clone();
    if !store(app, move |tx| still(tx, &runs::by_id(tx, &wanted)?)).await? {
        return Ok(Some("it is no longer idle".into()));
    }
    let (id, words) = (run.id.clone(), words.to_string());
    match server.run(format!("kill-pane -t {pane}")).await {
        Ok(_) => {
            let closed = store(app, move |tx| {
                let r = runs::change(tx, &id, |r| r.state = Phase::Closed)?;
                say(tx, &r, &words)
            }).await;
            app.dispatcher.soon.notify_one();             // a slot is free: what waits for it starts (AC-158)
            closed.map(|()| None)
        }
        Err(e) => {
            let why = format!("tmux did not close it: {e}");
            store(app, move |tx| {
                let r = runs::change(tx, &id, |r| if r.close_tried { r.state = Phase::Left; } else { r.close_tried = true; })?;
                if r.state == Phase::Left { say(tx, &r, &format!("Session not closed: {e}. It stays open.")) } else { Ok(()) }
            }).await.map(|()| Some(why))
        }
    }
}

/// The agent and what runs under it, tmux's word taken just before: its mark, and no exit recorded,
/// as a program a human then starts at the shell it leaves is none of this (AC-126). A `ps` that does
/// not answer is tried again; what could not be ended is said. Last of all, under the pane's typing queue
/// through the SIGTERM: the run still Ending (new work or Give up calls it off), and no human's input since
/// `at` (it waits for the next walk, which decides again). Then the run is Gone.
async fn end_agent(app: &Shared, run: &Run, server: &Arc<Server>, pane: &str, at: Instant) {
    let grace = (Duration::from_secs(10) / scale() as u32).max(Duration::from_millis(500));
    let key = server.key(pane);
    let mut why = String::new();
    for _ in 0..3 {
        let said = server.run(format!("display -p -t {pane} '#{{@terminum-run}}\t#{{@terminum-exit}}\t#{{pane_pid}}'")).await.unwrap_or_default();
        let fields: Vec<&str> = said.first().map_or(Vec::new(), |l| l.split('\t').collect());
        let [mark, exit, root] = fields[..] else { return };
        if mark != run.id || !exit.is_empty() {
            return ended(app, run, None).await;        // it has gone by itself, or the pane is no more this run's
        }
        let targets = match procs::under_root(app, server, root).await {
            Ok(t) => t,
            Err(e) => {
                why = e;
                tokio::time::sleep((BEAT / scale() as u32).max(Duration::from_millis(500))).await;
                continue;
            }
        };
        let Some(held) = app.inputs.act_queue(server, pane, &key, at).await else {
            return tracing::info!(event = "dispatcher.end", task = %run.names(), "put off: a human typed in its session");
        };
        let id = run.id.clone();
        match store(app, move |tx| runs::by_id(tx, &id)).await {
            Ok(now) if now.state == Phase::Ending => {}
            Ok(now) => return called_off(app, &now).await,
            Err(e) => return tracing::warn!(event = "dispatcher.end", task = %run.names(), error = %e),
        }
        procs::term(app, server, &targets).await;
        drop(held);                                    // what a human types now waits for no grace: it reaches the shell
        procs::kill_left(app, server, root, &targets, grace).await;
        tracing::info!(event = "dispatcher.end", task = %run.names(), signalled = targets.len());
        return ended(app, run, None).await;
    }
    tracing::warn!(event = "dispatcher.end", task = %run.names(), error = %why);
    ended(app, run, Some(format!("The agent could not be ended: {why}. It may still run."))).await;
}

/// Its End is done, or could not be: Gone, if it is still Ending, and `words` said.
async fn ended(app: &Shared, run: &Run, words: Option<String>) {
    let id = run.id.clone();
    let done = store(app, move |tx| {
        let r = runs::change(tx, &id, |r| if r.state == Phase::Ending { r.gone(); })?;
        words.map_or(Ok(()), |w| say(tx, &r, &w))
    }).await;
    if let Err(e) = done {
        tracing::warn!(event = "dispatcher.end", task = %run.names(), error = %e);
    }
    super::told_all(app, run);
}

/// New work came, or the human gave up, after its End was decided: the agent stays.
async fn called_off(app: &Shared, now: &Run) {
    tracing::info!(event = "dispatcher.end", task = %now.names(), state = now.state.label(), "called off");
    if now.state == Phase::Working {
        let r = now.clone();
        let _ = store(app, move |tx| say(tx, &r, "Its end at the limit was called off: new work came in its session.")).await;
        super::told_all(app, now);
    }
}

/// What a delivered act's flag says, written once it went (delivered, waiting for a tool call, or refused); `mark`
/// sets it only while it still applies, so new work since (AC-122) is not undone.
async fn went(app: &Shared, run: &Run, mark: fn(&mut Run)) {
    let id = run.id.clone();
    if let Err(e) = store(app, move |tx| runs::change(tx, &id, mark).map(drop)).await {
        tracing::warn!(event = "dispatcher.act", task = %run.names(), error = %e);
    }
}

/// Words for the agent: on its next tool call while it works, through the gate while it idles.
pub(super) async fn tell(app: &Shared, run: &Run, server: &Arc<Server>, pane: &str, text: String, what: &str) {
    let key = server.key(pane);
    let sent = if app.hooks.working(&key) || run.open_tools > 0 { Err(None) } else { gate::deliver(app, run, server, pane, &text, false).await.map_err(Some) };
    let words = match sent {
        Ok(()) => format!("{what} was given."),
        Err(why) => {
            let dropped = app.task_lines.push(&key, run.ids.first().map_or("", String::as_str), text);
            let waits = match why {
                Some(why) => format!("{what} waits for the agent's next tool call: {why}."),
                None => format!("{what} waits for the agent's next tool call."),
            };
            if dropped { format!("{waits} The oldest line waiting went: at most 20 wait.") } else { waits }
        }
    };
    let r = run.clone();
    let _ = store(app, move |tx| say(tx, &r, &words)).await;
}

/// A line every open page shows for a moment.
pub(super) fn note(app: &Shared, msg: String) {
    app.pages.tell(&serde_json::json!({"t": "note", "msg": msg}).to_string());
}

pub(super) async fn do_later(app: &Shared, l: Later) {
    let Some((server, pane)) = l.run.place(app) else { return };
    let (task, key) = (l.run.names(), server.key(&pane));
    let done = match l.act {
        Act::Warn(left) => {
            let text = format!("[terminum Dispatcher] {task}: {} minutes of the {}-minute budget are left. Write the result with {} and end your subagents now. Then the agent is ended and unsaved work is lost.", (left.max(0) + 59) / 60, l.run.budget.unwrap_or(0) / 60, super::press::FINISH);
            tell(app, &l.run, &server, &pane, text, "The last warning").await;
            went(app, &l.run, |r| r.warned |= plan::warn_due(r)).await;
            Ok(())
        }
        Act::MemoryWarn(gib) => {
            let text = format!("[terminum Dispatcher] {task}: its session uses {gib:.1} GiB of memory. End what it does not need.");
            tell(app, &l.run, &server, &pane, text, "The memory warning").await;
            note(app, format!("{task}'s session uses {gib:.1} GiB of memory"));
            went(app, &l.run, |r| r.memory_warned = true).await;
            Ok(())
        }
        Act::End => {
            if !app.task_lines.take(&key).is_empty() {
                let r = l.run.clone();
                let _ = store(app, move |tx| say(tx, &r, "The last warning could not reach the agent before the limit.")).await;
            }
            // a walk's next End of it waits for this one
            if let Some(held) = super::Held::take(app, |d| &d.ending, &l.run.id) {
                let (a, r, s, p, at) = (app.clone(), l.run.clone(), server.clone(), pane.clone(), l.at);
                tokio::spawn(async move {
                    end_agent(&a, &r, &s, &p, at).await;
                    drop(held);
                });
            }
            Ok(())
        }
        Act::Nudge => {
            let text = format!("[terminum Dispatcher] {task} is still open and you stopped. Call {} with the result, or ask a human by {} what you need.", super::press::FINISH, super::press::ASK_HUMAN);
            let sent = gate::deliver(app, &l.run, &server, &pane, &text, false).await;
            went(app, &l.run, |r| r.nudged |= r.idle_for >= plan::NUDGE).await;
            match sent {
                Ok(()) => Ok(()),
                Err(why) => {
                    let r = l.run.clone();
                    store(app, move |tx| say(tx, &r, &format!("Not delivered: {why}"))).await
                }
            }
        }
        // a session that waits for a human's answers closes only at an empty box: a draft there is someone's (AC-157)
        Act::Close { .. } if plan::asked_human(&l.run) && !matches!(gate::read(&server, &pane, &l.run.kind).await, Ok(super::screen::Screen::Empty)) => Ok(()),
        Act::Close { words, keep } => close(app, &l.run, (&server, &pane), &words, keep, l.at, |_, r| Ok(plan::still_closes(r) || plan::asked_closes(r))).await.map(drop),
        _ => Ok(()),
    };
    if let Err(e) = done {
        tracing::warn!(event = "dispatcher.act", task = %task, error = %e);
    }
    super::told_all(app, &l.run);
}

/// A subagent silent and idle: one request to the main agent, then, as it cannot be ended alone inside the
/// agent, its task FAILED, its session kept (AC-127, OQ-122). A busy tree runs neither clock.
pub(super) async fn watch_subagents(app: &Shared, run: &Run, server: &Arc<Server>, pane: &str, key: &str, busy: bool) {
    let due: Vec<(String, plan::SubStep)> = {
        let mut watch = app.dispatcher.watch.lock();
        let Some(w) = watch.get_mut(key) else { return };
        if busy || run.open_tools > 0 {
            for sub in w.subs.values_mut() {
                sub.last = Instant::now();
            }
            return;
        }
        w.subs.iter_mut().filter_map(|(id, sub)| {
            let step = plan::subagent_step(sub.last.elapsed().as_secs() as i64 * scale(), sub.told)?;
            sub.told = step;
            Some((id.clone(), step))
        }).collect()
    };
    for (id, step) in due {
        if step == plan::SubStep::Tell {
            let text = format!("[terminum Dispatcher] A subagent of {} has been silent for 10 minutes. End it if it is stuck.", run.names());
            tell(app, run, server, pane, text, "The request about a silent subagent").await;
        } else {
            // it runs inside the agent and cannot be ended alone: the task fails, its session stays, and RUN
            // goes on in it (OQ-122, AC-122)
            let (r, words) = (run.clone(), format!("Subagent {id} has been silent for 20 minutes and cannot be ended alone: it runs inside the agent. RUN goes on in this session."));
            note(app, format!("{}: {words}", run.names()));
            let _ = store(app, move |tx| super::fail(tx, &r, &words)).await;
        }
        super::told_all(app, run);
    }
}

#[cfg(test)]
#[path = "acts_tests.rs"]
mod tests;
