// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What the session list shows of tasks: open ones by session, for their rows' marks (AC-113), and
//! by project, for the dialog of Delete Project (AC-110), and the tasks that need a human with the lines no human
//! closed or saw (AC-117). Told to pages only when they change.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use parking_lot::Mutex;
use serde_json::{json, Value};

use super::api::{answer, store};
use super::ops::{self, rows};
use super::store::Store;
use crate::app::Shared;
use crate::page::Page;

#[derive(Default, PartialEq)]
struct Held {
    counts: HashMap<String, i64>,
    sessions: BTreeMap<String, Vec<String>>,
    /// WAIT FOR HUMAN and FAILED (AC-117)
    attention: Vec<Asks>,
}

/// A task that needs a human, as its last move left it (AC-117).
#[derive(PartialEq)]
struct Asks {
    id: String,
    failed: bool,
    /// an agent or Dispatcher made the move: it has a line
    by_agent: bool,
    /// the first line of the reason that move gave
    why: String,
    /// which move: a move back into the column is a new line, whatever its words
    moved: i64,
    /// the key of the line a human closed or saw, kept in the store for every browser
    seen: Option<String>,
}

impl Asks {
    fn words(&self) -> String {
        if self.failed { format!("{} failed: {}", self.id, self.why) } else { format!("{} waits for you", self.id) }
    }

    /// What a close names: this move with these words, so a close of an older line leaves a newer one up.
    fn key(&self) -> String {
        format!("{}|{}", self.moved, self.words())
    }

    fn due(&self) -> bool {
        self.by_agent && self.seen.as_deref() != Some(self.key().as_str())
    }
}

#[derive(Default)]
pub struct Marks {
    held: Mutex<Held>,
    /// one read at a time: a slow one never stands over a newer; `held` stays free meanwhile
    turn: Mutex<()>,
    /// a read is asked for and has not begun: a burst of changes costs one
    asked: AtomicBool,
}

/// A task changed: the marks are read again on a task of their own, and pages told when they
/// changed; a read that fails is tried again with back-off (D16), else they would stand stale
/// until the next change.
pub fn read_later(app: &Shared) {
    if !app.task_marks.ask() {
        return;                                        // a read yet to begin sees this change too
    }
    let a = app.clone();
    tokio::spawn(async move {
        let mut wait = std::time::Duration::from_secs(1);
        loop {
            let b = a.clone();
            match tokio::task::spawn_blocking(move || b.task_marks.refresh(&b.tasks)).await.map_err(|e| e.to_string()).and_then(|r| r) {
                Ok(true) => return tell(&a),
                Ok(false) => return,
                Err(e) => tracing::warn!(event = "tasks.marks", error = %e),
            }
            if !a.task_marks.ask() {
                return;                                // one a newer change asked for reads instead
            }
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(std::time::Duration::from_secs(60));
        }
    });
}

impl Marks {
    /// A change came: whether its caller must start a `refresh`; else one yet to begin will see it.
    pub fn ask(&self) -> bool {
        !self.asked.swap(true, Ordering::AcqRel)
    }

    /// Reads them from the store: whether they changed, or why they could not be read (the last
    /// numbers stand). Off the async workers.
    pub fn refresh(&self, store: &Store) -> Result<bool, String> {
        let _turn = self.turn.lock();
        self.asked.store(false, Ordering::Release);
        let read = store.with(|tx| {
            let counts = rows(tx, "SELECT project, count(*) FROM tasks WHERE archived_at IS NULL GROUP BY project", [], |r| Ok((r.get(0)?, r.get(1)?)))?.into_iter().collect();
            let mut sessions: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for (key, id) in rows(tx, "SELECT session_key, slug FROM tasks WHERE session_key IS NOT NULL AND archived_at IS NULL AND status <> 'DONE' ORDER BY number", [], |r| Ok((r.get(0)?, r.get(1)?)))? {
                sessions.entry(key).or_default().push(id);
            }
            // the reason is the one its mover gave with that move, never a later note (a human's choice of agent)
            let attention = rows(tx, "SELECT t.slug, t.status = 'FAILED', COALESCE(m.author != 'human', 0),
                    COALESCE((SELECT text FROM events e WHERE e.task = t.id AND e.kind = 'why' AND e.author = m.author AND e.rowid > m.rowid ORDER BY e.rowid DESC LIMIT 1), ''),
                    COALESCE(m.rowid, 0), t.line_seen
                FROM tasks t LEFT JOIN events m ON m.rowid = (SELECT e.rowid FROM events e WHERE e.task = t.id AND e.kind = 'move' ORDER BY e.at DESC, e.rowid DESC LIMIT 1)
                WHERE t.status IN ('WAIT FOR HUMAN', 'FAILED') AND t.archived_at IS NULL ORDER BY t.number DESC", [],
                |r| Ok(Asks { id: r.get(0)?, failed: r.get(1)?, by_agent: r.get(2)?, why: r.get::<_, String>(3)?.lines().next().unwrap_or("").to_string(), moved: r.get(4)?, seen: r.get(5)? }))?;
            Ok(Held { counts, sessions, attention })
        });
        // a passing failure says nothing about the tasks: the last numbers stand
        let read = read?;
        let mut held = self.held.lock();
        let changed = *held != read;
        *held = read;
        Ok(changed)
    }

    /// Whether a task not done stands linked to session `key`.
    pub fn has_session(&self, key: &str) -> bool {
        self.held.lock().sessions.contains_key(key)
    }

    fn message(&self) -> String {
        let held = self.held.lock();
        let lines: Vec<_> = held.attention.iter().filter(|a| a.due()).map(|a| json!({"id": a.id, "failed": a.failed, "words": a.words(), "key": a.key()})).collect();
        json!({"t": "taskmarks", "marks": held.sessions, "counts": held.counts, "need": held.attention.len(), "lines": lines}).to_string()
    }
}

/// A page that connects learns them.
pub fn hello(app: &Shared, page: &Page) {
    page.send(app.task_marks.message());
}

/// The longest key a close names: an ID, a move and a reason's first line.
const KEY_MOST: usize = 64 * 1024;

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/tasks/{id}/seen", post(seen))
}

/// `{key}`: the human closed or saw that line of the task (AC-117); kept here, every page hears it.
async fn seen(State(app): State<Shared>, Path(id): Path<String>, Json(body): Json<Value>) -> Response {
    let Some(key) = body["key"].as_str().filter(|k| !k.is_empty() && k.len() <= KEY_MOST).map(str::to_owned) else {
        return crate::http::refuse(StatusCode::BAD_REQUEST, "Name the line: its key, as terminumd gave it.");
    };
    let done = store(&app, move |tx| keep_seen(tx, &id, &key).map(|()| json!({"ok": true}))).await;
    if done.is_ok() {
        read_later(&app);
    }
    answer(done)
}

fn keep_seen(tx: &rusqlite::Transaction, id: &str, key: &str) -> Result<(), String> {
    let task = ops::key_of(tx, id)?;
    tx.execute("UPDATE tasks SET line_seen = ?1 WHERE id = ?2", (key, task)).map(drop).map_err(ops::sql)
}

/// Every page learns them (after a change).
pub fn tell(app: &Shared) {
    app.pages.tell(&app.task_marks.message());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::ops::{create, move_to, NewTask};
    use crate::tasks::store::{DISPATCHER, HUMAN};

    fn lines(marks: &Marks) -> (Value, Value) {
        let told: Value = serde_json::from_str(&marks.message()).unwrap();
        (told["need"].clone(), told["lines"].clone())
    }

    #[test]
    fn ac117_a_line_closed_in_one_browser_is_closed_in_all_until_its_task_moves_back() {
        let store = Store::open(crate::testutil::fresh_dir("marks-seen").join("kanban.sqlite"));
        let marks = Marks::default();
        let t = store.with(|tx| create(tx, HUMAN, NewTask { title: "a".into(), ..Default::default() }, 1)).unwrap();
        store.with(|tx| move_to(tx, DISPATCHER, &t.id, "WAIT FOR HUMAN", 2)).unwrap();
        marks.refresh(&store).unwrap();
        let (need, told) = lines(&marks);
        let key = told[0]["key"].as_str().unwrap().to_string();
        assert_eq!((need, told[0]["words"].clone()), (json!(1), json!(format!("{} waits for you", t.id))));
        store.with(|tx| keep_seen(tx, &t.id, "1|an older line")).unwrap();
        marks.refresh(&store).unwrap();
        assert_eq!(lines(&marks).1[0]["key"], json!(key), "a close of another line leaves this one up");
        store.with(|tx| keep_seen(tx, &t.id, &key)).unwrap();
        assert!(marks.refresh(&store).unwrap(), "a close is news for every page");
        assert_eq!(lines(&marks), (json!(1), json!([])), "the line goes; the task still needs a human");
        store.with(|tx| { move_to(tx, HUMAN, &t.id, "IN PROGRESS", 3)?; move_to(tx, DISPATCHER, &t.id, "WAIT FOR HUMAN", 4) }).unwrap();
        marks.refresh(&store).unwrap();
        let (_, told) = lines(&marks);
        assert_eq!(told[0]["words"], json!(format!("{} waits for you", t.id)), "back in the column: its line again, same words");
        assert_ne!(told[0]["key"], json!(key));
    }

    #[test]
    fn ac117_a_failed_line_says_the_reason_its_move_gave_never_a_later_note() {
        let store = Store::open(crate::testutil::fresh_dir("marks-why").join("kanban.sqlite"));
        let marks = Marks::default();
        let t = store.with(|tx| create(tx, HUMAN, NewTask { title: "a".into(), ..Default::default() }, 1)).unwrap();
        store.with(|tx| {
            move_to(tx, DISPATCHER, &t.id, "FAILED", 2)?;
            ops::log(tx, &t.key, DISPATCHER, "why", "No result.\nmore", None, 2).map(drop)
        }).unwrap();
        marks.refresh(&store).unwrap();
        let (_, told) = lines(&marks);
        assert_eq!(told[0]["words"], json!(format!("{} failed: No result.", t.id)));
        let key = told[0]["key"].as_str().unwrap().to_string();
        store.with(|tx| keep_seen(tx, &t.id, &key)).unwrap();
        store.with(|tx| ops::log(tx, &t.key, HUMAN, "why", "Its agent: codex", None, 3).map(drop)).unwrap();
        marks.refresh(&store).unwrap();
        assert_eq!(lines(&marks), (json!(1), json!([])), "a human's note neither reopens the line nor rewords it");
        store.with(|tx| move_to(tx, HUMAN, &t.id, "WAIT FOR HUMAN", 4)).unwrap();
        marks.refresh(&store).unwrap();
        assert_eq!(lines(&marks), (json!(1), json!([])), "a human's move has no line");
        assert!(store.with(|tx| keep_seen(tx, "TASK-404", "x")).is_err(), "an unknown task is refused");
    }

    #[test]
    fn ac113_a_burst_of_changes_asks_for_one_read_and_one_that_began_asks_anew() {
        let store = Store::open(crate::testutil::fresh_dir("marks-ask").join("kanban.sqlite"));
        let marks = Marks::default();
        assert!(marks.ask(), "the first starts a read");
        assert!(!marks.ask(), "a read yet to begin sees this one too");
        marks.refresh(&store).unwrap();
        assert!(marks.ask(), "what comes after a read began needs another");
    }

    #[test]
    fn ac113_marks_hold_open_tasks_and_say_when_they_changed() {
        let store = Store::open(crate::testutil::fresh_dir("marks").join("kanban.sqlite"));
        let marks = Marks::default();
        assert!(!marks.refresh(&store).unwrap(), "nothing yet");
        let key = "tmux:local:terminum:%7";
        let t = store.with(|tx| create(tx, HUMAN, NewTask { title: "a".into(), session: Some(key.into()), ..Default::default() }, 1)).unwrap();
        assert!(marks.refresh(&store).unwrap());
        assert!(!marks.refresh(&store).unwrap(), "the same again is no change");
        let told: serde_json::Value = serde_json::from_str(&marks.message()).unwrap();
        assert_eq!(told, json!({"t": "taskmarks", "marks": {key: [t.id]}, "counts": {"default": 1}, "need": 0, "lines": []}));
        store.with(|tx| move_to(tx, HUMAN, &t.id, "DONE", 2)).unwrap();
        assert!(marks.refresh(&store).unwrap());
        let told: serde_json::Value = serde_json::from_str(&marks.message()).unwrap();
        assert_eq!(told, json!({"t": "taskmarks", "marks": {}, "counts": {"default": 1}, "need": 0, "lines": []}), "DONE leaves its row, and still counts in its project");
    }

    #[test]
    fn ac113_a_store_that_fails_keeps_the_last_numbers() {
        let marks = Marks::default();
        marks.held.lock().counts.insert("api".into(), 3);
        assert!(marks.refresh(&Store::absent()).is_err(), "said, so its caller tries again (D16)");
        assert_eq!(marks.held.lock().counts["api"], 3);
    }
}
