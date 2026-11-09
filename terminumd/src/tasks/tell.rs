// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What waits for a pane's next hook that carries context: Dispatcher's words its agent could not take at once
//! (`terminum-dispatcher.md` §9): fixed words with a T-number, never a title, text or comment.

use std::collections::{HashMap, HashSet};

use parking_lot::Mutex;
use rusqlite::OptionalExtension;
use serde_json::Value;

use super::ops;
use super::store::DISPATCHER;
use crate::app::Shared;
use crate::tmux::agentstate::State;

/// One pane's lines, with their tasks: waiting for its next context, and those sent with one
/// and not yet heard (the hook says so after printing them, `Heard`).
#[derive(Default)]
struct Waiting {
    lines: Vec<(String, String)>,
    sent: Option<(u64, Vec<(String, String)>)>,
}

/// Lines for panes' next contexts: each goes until the hook says it was heard (D15), so an
/// answer lost on its way, or a hook that printed nothing, sends it again.
#[derive(Default)]
pub struct Lines(Mutex<HashMap<String, Waiting>>);

/// The most lines a pane keeps: a session with no next hook never holds many.
const KEPT: usize = 20;

impl Lines {
    /// A line waits for the pane's next context; true when an older one went to make room.
    pub(crate) fn push(&self, key: &str, task: &str, line: String) -> bool {
        let mut all = self.0.lock();
        let w = all.entry(key.to_string()).or_default();
        w.lines.push((task.to_string(), line));
        let dropped = w.lines.len() > KEPT;
        if dropped {
            w.lines.remove(0);
        }
        dropped
    }

    /// The pane's lines for its next context, those sent before and not heard first, and the
    /// number the hook says it heard them by.
    pub(crate) fn offer(&self, key: &str) -> Option<(u64, Vec<String>)> {
        static SENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let mut all = self.0.lock();
        let w = all.get_mut(key)?;
        let mut lines = w.sent.take().map(|(_, l)| l).unwrap_or_default();
        lines.append(&mut w.lines);
        if lines.is_empty() {
            all.remove(key);
            return None;
        }
        let n = SENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let words = lines.iter().map(|(_, l)| l.clone()).collect();
        w.sent = Some((n, lines));
        Some((n, words))
    }

    /// The hook printed the lines sent as `n`: they go; their tasks.
    pub(crate) fn heard(&self, key: &str, n: u64) -> Vec<String> {
        let mut all = self.0.lock();
        let Some(w) = all.get_mut(key) else { return Vec::new() };
        if w.sent.as_ref().is_none_or(|(m, _)| *m != n) {
            return Vec::new();
        }
        let tasks = w.sent.take().map(|(_, l)| l.into_iter().map(|(t, _)| t).collect()).unwrap_or_default();
        if w.lines.is_empty() {
            all.remove(key);
        }
        tasks
    }

    /// Every line of the pane, sent or not, which never reached it (its run ends).
    pub(crate) fn take(&self, key: &str) -> Vec<(String, String)> {
        self.0.lock().remove(key).map(|w| w.sent.map(|(_, l)| l).unwrap_or_default().into_iter().chain(w.lines).collect()).unwrap_or_default()
    }
}

/// Each line that went is said on its task, in one transaction; a task gone costs only its line.
fn say_told(tx: &rusqlite::Transaction, tasks: &[String], at: i64) -> Result<HashSet<String>, String> {
    let mut said = HashSet::new();
    for id in tasks {
        match ops::key_of(tx, id) {
            Ok(key) => {
                ops::log(tx, &key, DISPATCHER, "told", "Told the session", None, at)?;
                said.insert(id.clone());
            }
            Err(e) => tracing::warn!(event = "tasks.told", task = %id, error = %e),
        }
    }
    Ok(said)
}

/// What a turn's end says of the task its session works on (AC-136), as hookstate read it: a turn
/// that ends in a question or while subagents run is no stop.
fn ended_words(state: State, why: &str) -> Option<String> {
    match state {
        State::Done => Some("The agent stopped with this task open".to_string()),
        State::Failed => Some(format!("The agent stopped: {}", if why.is_empty() { "an error" } else { why })),
        _ => None,
    }
}

/// An agent's event is said on the task in IN PROGRESS of its pane, once in a row: nothing moves
/// by it (terminum-dispatcher.md AC-123 moves only what Dispatcher started); `Ok(None)`: nothing to say.
fn say_stopped(tx: &rusqlite::Transaction, key: &str, words: &str, at: i64) -> Result<Vec<String>, String> {
    let mut said = Vec::new();
    for (row, id) in ops::rows(tx, "SELECT id, slug FROM tasks WHERE session_key = ?1 AND status = 'IN PROGRESS' AND archived_at IS NULL", [key], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let last: Option<String> = tx.query_row("SELECT text FROM events WHERE task = ?1 ORDER BY at DESC, rowid DESC LIMIT 1", [&row], |r| r.get(0)).optional().map_err(ops::sql)?;
        if last.as_deref() != Some(words) {
            ops::log(tx, &row, DISPATCHER, "told", words, None, at)?;
            said.push(id);
        }
    }
    Ok(said)
}

/// Off the async workers; an open window shows it.
fn said_stopped(app: &Shared, key: &str, words: String) {
    let (a, key) = (app.clone(), key.to_string());
    tokio::task::spawn_blocking(move || match a.tasks.with(|tx| say_stopped(tx, &key, &words, ops::now())) {
        Ok(said) => said.iter().for_each(|id| super::api::history_heard(&a, id)),
        Err(e) => tracing::warn!(event = "tasks.told", error = %e),
    });
}

/// A pane's turn ended: what became of it is said on its task.
pub fn turn_ended(app: &Shared, key: &str) {
    if let Some(words) = app.hooks.ended_as(key).and_then(|(state, why)| ended_words(state, &why)) {
        said_stopped(app, key, words);
    }
}

/// A pane's agent session ended; /clear goes on.
pub fn session_ended(app: &Shared, key: &str, ev: &Value) {
    if ev["event"] == "SessionEnd" && ev["reason"] != "clear" {
        said_stopped(app, key, "The agent's session ended with this task open".to_string());
    }
}

/// The pane's next context, joined to what else is due, and the number its lines are heard by.
pub fn with_lines(app: &Shared, key: &str, remind: Option<String>) -> (Option<String>, Option<u64>) {
    let Some((n, lines)) = app.task_lines.offer(key) else { return (remind, None) };
    let words: Vec<String> = remind.into_iter().chain(lines).collect();
    (Some(words.join("\n")), Some(n))
}

/// The hook printed the lines sent as `n` (`Heard`): each is said on its task.
pub fn heard(app: &Shared, key: &str, n: u64) {
    let tasks = app.task_lines.heard(key, n);
    if tasks.is_empty() {
        return;
    }
    let a = app.clone();
    tokio::task::spawn_blocking(move || match a.tasks.with(|tx| say_told(tx, &tasks, ops::now())) {
        Ok(said) => said.iter().for_each(|id| super::api::history_heard(&a, id)),   // an open window shows it
        Err(e) => tracing::warn!(event = "tasks.told", error = %e),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac132_what_a_turn_s_end_says() {
        assert_eq!(ended_words(State::Failed, "rate_limit").as_deref(), Some("The agent stopped: rate_limit"));
        assert_eq!(ended_words(State::Failed, "").as_deref(), Some("The agent stopped: an error"));
        assert_eq!(ended_words(State::Done, "").as_deref(), Some("The agent stopped with this task open"));
        assert_eq!(ended_words(State::Waiting, "Which one?"), None, "a question is no stop");
        assert_eq!(ended_words(State::Working, ""), None, "subagents still run");
    }

    #[test]
    fn ac132_it_is_said_on_the_task_in_progress_once_and_moves_nothing() {
        use crate::tasks::ops::{create, move_to, NewTask};
        use crate::tasks::store::{Store, HUMAN};
        let s = Store::open(crate::testutil::fresh_dir("tell-stopped").join("kanban.sqlite"));
        let key = "tmux:local:t:%7";
        let t = s.with(|tx| create(tx, HUMAN, NewTask { title: "a".into(), session: Some(key.into()), ..Default::default() }, 1)).unwrap();
        let said = |w: &str| s.with(|tx| say_stopped(tx, key, w, 2)).unwrap();
        assert!(said("The agent stopped with this task open").is_empty(), "in TODO: nothing to say");
        s.with(|tx| move_to(tx, HUMAN, &t.id, "IN PROGRESS", 2)).unwrap();
        assert_eq!(said("The agent stopped with this task open"), std::slice::from_ref(&t.id));
        assert!(said("The agent stopped with this task open").is_empty(), "once in a row");
        assert_eq!(said("The agent stopped: rate_limit"), std::slice::from_ref(&t.id), "another word is said");
        assert_eq!(s.with(|tx| ops::get(tx, &t.id)).unwrap().status, "IN PROGRESS", "nothing moved");
    }

    #[test]
    fn ac114_each_line_is_said_on_its_task_and_a_task_gone_costs_only_its_line() {
        use crate::tasks::ops::{create, NewTask};
        use crate::tasks::store::{Store, HUMAN};
        let s = Store::open(crate::testutil::fresh_dir("tell-lines").join("kanban.sqlite"));
        let t = s.with(|tx| create(tx, HUMAN, NewTask { title: "a".into(), ..Default::default() }, 1)).unwrap();
        let said = s.with(|tx| say_told(tx, &[t.id.clone(), t.id.clone(), "T-9999".to_string()], 2)).unwrap();
        assert_eq!(said, HashSet::from([t.id.clone()]), "each task once, the one gone not");
        let told: i64 = s.with(|tx| tx.query_row("SELECT count(*) FROM events WHERE text = 'Told the session'", [], |r| r.get(0)).map_err(ops::sql)).unwrap();
        assert_eq!(told, 2, "one event per line");
    }

    #[test]
    fn ac114_lines_wait_per_pane_and_go_once_heard() {
        let l = Lines::default();
        l.push("a", "T-1", "one".into());
        l.push("a", "T-2", "two".into());
        l.push("b", "T-3", "three".into());
        let (n, words) = l.offer("a").unwrap();
        assert_eq!(words, ["one", "two"]);
        assert_eq!(l.heard("a", n), ["T-1", "T-2"]);
        assert!(l.offer("a").is_none(), "heard: gone");
        assert_eq!(l.take("b")[0].1, "three");
    }

    /// AC-114 failure (D15): lines whose answer never reached the agent (not heard) go again
    /// with its next context, before newer ones; a late Heard of an older sending takes nothing.
    #[test]
    fn ac114_lines_not_heard_go_again() {
        let l = Lines::default();
        l.push("a", "T-1", "one".into());
        let (first, _) = l.offer("a").unwrap();
        l.push("a", "T-1", "two".into());
        let (second, words) = l.offer("a").unwrap();
        assert_eq!(words, ["one", "two"], "the lost one again, then the new one");
        assert!(l.heard("a", first).is_empty(), "an older sending heard late takes nothing");
        assert_eq!(l.heard("a", second).len(), 2);
        assert!((0..KEPT).all(|i| !l.push("b", "T-2", i.to_string())) && l.push("b", "T-2", "over".into()), "past the most, the oldest goes and that is said");
        assert_eq!(l.take("b").len(), KEPT);
    }
}
