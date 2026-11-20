// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What waits for a pane's next hook that carries context: Dispatcher's words its agent could not take at once
//! (`terminum-dispatcher.md` §9): fixed words with a T-number, never a title, text or comment; and the integration
//! queue's notices' first lines (`terminum-integration.md` AC-192), terminumd's words with IDs, commits and file names.

use std::collections::{HashMap, HashSet};

use parking_lot::Mutex;
use rusqlite::OptionalExtension;
use serde_json::Value;

use super::ops;
use super::store::DISPATCHER;
use crate::app::Shared;
use crate::tmux::agentstate::State;

/// A line for a pane's next context: its task, its words, and the comments it tells of (AC-209), none for
/// Dispatcher's and the integration queue's words.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub task: String,
    pub text: String,
    /// each comment's event and who wrote it, in a notice's fixed words ("a human (alex)")
    pub comments: Vec<(String, String)>,
}

impl Line {
    /// Whether it tells of comments (AC-209), not Dispatcher's or the integration queue's words.
    pub fn is_notice(&self) -> bool {
        !self.comments.is_empty()
    }
}

/// One pane's lines: waiting for its next context, and those sent with one and not yet heard (the hook says
/// so after printing them, `Heard`).
#[derive(Default)]
struct Waiting {
    lines: Vec<Line>,
    sent: Option<(u64, Vec<Line>)>,
}

/// Lines for panes' next contexts: each goes until the hook says it was heard (D15), so an
/// answer lost on its way, or a hook that printed nothing, sends it again.
#[derive(Default)]
pub struct Lines(Mutex<HashMap<String, Waiting>>);

/// The most lines a pane keeps: a session with no next hook never holds many.
const KEPT: usize = 20;

impl Lines {
    /// A line waits for the pane's next context; the older one that went to make room, if any.
    pub(crate) fn push(&self, key: &str, task: &str, line: String) -> Option<Line> {
        self.add(key, Line { task: task.to_string(), text: line, comments: Vec::new() })
    }

    fn add(&self, key: &str, line: Line) -> Option<Line> {
        let mut all = self.0.lock();
        let w = all.entry(key.to_string()).or_default();
        w.lines.push(line);
        (w.lines.len() > KEPT).then(|| w.lines.remove(0))
    }

    /// A new comment on `task` waits for the pane's next context (AC-209): with the comments on it not sent yet, in
    /// one notice that counts them; one sent and not heard yet stays as it went, and an edited comment waiting
    /// already is not counted twice. The line that went to make room, if any.
    pub(crate) fn push_comment(&self, key: &str, task: &str, event: &str, by: String) -> Option<Line> {
        {
            let mut all = self.0.lock();
            if let Some(line) = all.get_mut(key).and_then(|w| w.lines.iter_mut().find(|l| l.task == task && l.is_notice())) {
                if !line.comments.iter().any(|(e, _)| e == event) {
                    line.comments.push((event.to_string(), by));
                    line.text = super::notify::words(task, &line.comments);
                }
                return None;
            }
        }
        let comments = vec![(event.to_string(), by)];
        self.add(key, Line { task: task.to_string(), text: super::notify::words(task, &comments), comments })
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
        let words = lines.iter().map(|l| l.text.clone()).collect();
        w.sent = Some((n, lines));
        Some((n, words))
    }

    /// The hook printed the lines sent as `n`: they go, said.
    pub(crate) fn heard(&self, key: &str, n: u64) -> Vec<Line> {
        let mut all = self.0.lock();
        let Some(w) = all.get_mut(key) else { return Vec::new() };
        if w.sent.as_ref().is_none_or(|(m, _)| *m != n) {
            return Vec::new();
        }
        let lines = w.sent.take().map(|(_, l)| l).unwrap_or_default();
        if w.lines.is_empty() {
            all.remove(key);
        }
        lines
    }

    /// Every line of the pane, sent or not, which never reached it (its run ends).
    pub(crate) fn take(&self, key: &str) -> Vec<Line> {
        self.0.lock().remove(key).map(|w| w.sent.map(|(_, l)| l).unwrap_or_default().into_iter().chain(w.lines).collect()).unwrap_or_default()
    }

    /// The pane's comment notices, sent or not, which never reached it (its agent's session ended, AC-209):
    /// Dispatcher's lines stay for the next agent of its run.
    pub(crate) fn take_comments(&self, key: &str) -> Vec<Line> {
        let mut all = self.0.lock();
        let Some(w) = all.get_mut(key) else { return Vec::new() };
        let mut gone = Vec::new();
        for list in [Some(&mut w.lines), w.sent.as_mut().map(|(_, l)| l)].into_iter().flatten() {
            let (comments, rest) = std::mem::take(list).into_iter().partition(Line::is_notice);
            *list = rest;
            gone.extend(comments);
        }
        gone
    }

    /// Every comment of `task` whose notice waits for its session (AC-209).
    pub(crate) fn waiting_comments(&self, task: &str) -> std::collections::HashSet<String> {
        self.0.lock().values().flat_map(|w| w.lines.iter().chain(w.sent.iter().flat_map(|(_, l)| l)))
            .filter(|l| l.task == task).flat_map(|l| l.comments.iter().map(|(e, _)| e.clone())).collect()
    }
}

/// Each line that went is said on its task, in one transaction: Dispatcher's as told, a notice with the comments
/// it told of (AC-209); a task gone costs only its line.
fn say_told(tx: &rusqlite::Transaction, lines: &[Line], at: i64) -> Result<HashSet<String>, String> {
    let mut said = HashSet::new();
    for l in lines {
        let done = if !l.is_notice() {
            ops::key_of(tx, &l.task).and_then(|key| ops::log(tx, &key, DISPATCHER, "told", "Told the session", None, at))
        } else {
            super::notify::said(tx, l, None, at)
        };
        match done {
            Ok(_) => { said.insert(l.task.clone()); }
            Err(e) => tracing::warn!(event = "tasks.told", task = %l.task, error = %e),
        }
    }
    Ok(said)
}

/// What a turn's end says of the task its session works on (AC-136), as hookstate read it: a turn
/// that ends in a question or while subagents run is no stop. Done waits for its fresh layout
/// to settle the end and say whether background work still runs (`None`: not settled yet).
fn ended_words(state: State, why: &str, background: Option<bool>) -> Option<String> {
    match state {
        State::Done => background.map(|runs| if runs {
            "The agent's turn ended while its background work runs with this task open"
        } else {
            "The agent stopped with this task open"
        }.to_string()),
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

/// A pane's hook ended its turn (`background`: None), or its fresh layout settled that end:
/// failures are said at once, Done only once the layout says whether background work runs.
pub fn turn_ended(app: &Shared, key: &str, background: Option<bool>) {
    if let Some(words) = app.hooks.ended_as(key).and_then(|(state, why)| ended_words(state, &why, background)) {
        said_stopped(app, key, words);
    }
}

/// A pane's agent session ended; /clear goes on. Its comment notices never reach it (AC-209).
pub fn session_ended(app: &Shared, key: &str, ev: &Value) {
    if ev["event"] == "SessionEnd" && ev["reason"] != "clear" {
        said_stopped(app, key, "The agent's session ended with this task open".to_string());
        super::notify::lost(app, app.task_lines.take_comments(key), super::notify::ENDED);
    }
}

/// The pane's next context, joined to what else is due, and the number its lines are heard by.
pub fn with_lines(app: &Shared, key: &str, remind: Option<String>) -> (Option<String>, Option<u64>) {
    let Some((n, lines)) = app.task_lines.offer(key) else { return (remind, None) };
    let words: Vec<String> = remind.into_iter().chain(lines).collect();
    (Some(words.join("\n")), Some(n))
}

/// A line waits for pane `key`'s next context; a comment's notice that went to make room is said not to have
/// reached it (AC-209). Whether one went.
pub fn push(app: &Shared, key: &str, task: &str, line: String) -> bool {
    let dropped = app.task_lines.push(key, task, line);
    let went = dropped.is_some();
    super::notify::lost(app, dropped.into_iter().collect(), super::notify::CROWDED);
    went
}

/// The hook printed the lines sent as `n` (`Heard`): each is said on its task.
pub fn heard(app: &Shared, key: &str, n: u64) {
    let lines = app.task_lines.heard(key, n);
    if lines.is_empty() {
        return;
    }
    let a = app.clone();
    tokio::task::spawn_blocking(move || match a.tasks.with(|tx| say_told(tx, &lines, ops::now())) {
        Ok(said) => said.iter().for_each(|id| super::api::history_heard(&a, id)),   // an open window shows it
        Err(e) => tracing::warn!(event = "tasks.told", error = %e),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac136_what_a_turn_s_end_says() {
        assert_eq!(ended_words(State::Failed, "rate_limit", None).as_deref(), Some("The agent stopped: rate_limit"));
        assert_eq!(ended_words(State::Failed, "", None).as_deref(), Some("The agent stopped: an error"));
        assert_eq!(ended_words(State::Done, "", None), None, "a hook's Stop waits for its fresh layout");
        assert_eq!(ended_words(State::Done, "", Some(false)).as_deref(), Some("The agent stopped with this task open"));
        assert_eq!(ended_words(State::Done, "", Some(true)).as_deref(), Some("The agent's turn ended while its background work runs with this task open"));
        assert_eq!(ended_words(State::Waiting, "Which one?", Some(false)), None, "a question is no stop");
        assert_eq!(ended_words(State::Working, "", Some(false)), None, "subagents still run");
    }

    #[tokio::test]
    async fn ac136_a_new_prompt_before_settlement_is_no_stop() {
        let (_root, app) = crate::testutil::app_with_tasks("tell-new-turn");
        let key = "tmux:local:t:%7";
        let task = app.tasks.with(|tx| {
            let t = ops::create(tx, super::super::store::HUMAN, ops::NewTask { title: "one".into(), session: Some(key.into()), ..Default::default() }, 1)?;
            ops::move_to(tx, super::super::store::HUMAN, &t.id, "IN PROGRESS", 2)?;
            Ok(t)
        }).unwrap();
        let event = |name| serde_json::json!({"agent": "claude", "event": name, "session": "s1"});
        let now = std::time::Instant::now();
        app.hooks.apply(key, &event("UserPromptSubmit"), now);
        app.hooks.apply(key, &event("Stop"), now);
        turn_ended(&app, key, None);
        app.hooks.apply(key, &event("UserPromptSubmit"), now);
        turn_ended(&app, key, Some(false));
        let stops: i64 = app.tasks.with(|tx| tx.query_row("SELECT count(*) FROM events WHERE task = ?1 AND kind = 'told'", [&task.key], |r| r.get(0)).map_err(ops::sql)).unwrap();
        assert_eq!(stops, 0, "a hook alone or a stale settlement writes no stop");
    }

    #[tokio::test]
    async fn ac136_settled_history_uses_the_fresh_layout_background_mark() {
        use crate::tmux::agentstate::{after_stop, Background, BACKGROUND_LIMIT};
        let (_root, app) = crate::testutil::app_with_tasks("tell-settled");
        for (key, footer) in [("tmux:local:t:%7", Background::Tasks), ("tmux:local:t:%8", Background::None)] {
            let task = app.tasks.with(|tx| {
                let t = ops::create(tx, super::super::store::HUMAN, ops::NewTask { title: "one".into(), session: Some(key.into()), ..Default::default() }, 1)?;
                ops::move_to(tx, super::super::store::HUMAN, &t.id, "IN PROGRESS", 2)?;
                Ok(t)
            }).unwrap();
            let now = std::time::Instant::now();
            let event = |name| serde_json::json!({"agent": "claude", "event": name, "session": "s1"});
            app.hooks.apply(key, &event("UserPromptSubmit"), now);
            app.hooks.apply(key, &event("Stop"), now);
            app.turns.hook_ended(key, now);
            let (keeps, background) = after_stop(true, Some(footer));
            assert!(!keeps);
            *app.layout.write() = serde_json::json!([{"items": [{"id": key, "state": "done", "background": background}]}]);
            let ended = app.turns.settle("tmux:local:t:", |_| Some(false), now + BACKGROUND_LIMIT);
            assert_eq!(ended, [key]);
            crate::manager::remind(&app, ended).await;
            let expected = ended_words(State::Done, "", background).unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let texts = app.tasks.with(|tx| ops::rows(tx, "SELECT text FROM events WHERE task = ?1 AND kind = 'told'", [&task.key], |r| r.get::<_, String>(0))).unwrap();
                    if !texts.is_empty() {
                        assert_eq!(texts, [expected]);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await.expect("settled history was written");
        }
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
        let line = |task: &str| Line { task: task.to_string(), text: "w".into(), comments: Vec::new() };
        let said = s.with(|tx| say_told(tx, &[line(&t.id), line(&t.id), line("T-9999")], 2)).unwrap();
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
        assert_eq!(l.heard("a", n).iter().map(|l| l.task.as_str()).collect::<Vec<_>>(), ["T-1", "T-2"]);
        assert!(l.offer("a").is_none(), "heard: gone");
        assert_eq!(l.take("b")[0].text, "three");
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
        assert!((0..KEPT).all(|i| l.push("b", "T-2", i.to_string()).is_none()) && l.push("b", "T-2", "over".into()).is_some_and(|g| g.text == "0"), "past the most, the oldest goes and that is said");
        assert_eq!(l.take("b").len(), KEPT);
    }
}
