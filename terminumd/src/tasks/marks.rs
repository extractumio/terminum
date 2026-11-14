// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What pages show of tasks: open ones by session, for the rows' marks (AC-113); counts by project, for the
//! dialog of Delete Project (AC-110); the tasks that need a human, with the lines no human closed or saw
//! (AC-117). Told to pages only when they change.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use rusqlite::{OptionalExtension, Transaction};
use serde_json::json;

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
    words: String,
    /// what a close names: the move (back into the column is a new line, whatever its words) and its words, so a
    /// close of an older line leaves a newer one up
    key: String,
    /// an agent or Dispatcher made the move and no human closed or saw its line
    due: bool,
    /// a card of its own in its column: no main task of the same column holds it (AC-112)
    card: bool,
}

/// WAIT FOR HUMAN and FAILED, or only task `one` (its store key) of them; the reason is the one its mover gave with
/// that move, else the move's own words (who moved it), never a later note (a human's choice of agent).
fn asks(tx: &Transaction, one: Option<&str>) -> Result<Vec<Asks>, String> {
    rows(tx, "SELECT t.slug, t.status = 'FAILED', COALESCE(m.author != 'human', 0),
            COALESCE((SELECT text FROM events e WHERE e.task = t.id AND e.kind = 'why' AND e.author = m.author AND e.rowid > m.rowid ORDER BY e.at DESC, e.rowid DESC LIMIT 1), m.text, ''),
            COALESCE(m.rowid, 0), t.line_seen,
            NOT EXISTS (SELECT 1 FROM tasks p WHERE p.id = t.parent AND p.status = t.status AND p.archived_at IS NULL)
        FROM tasks t LEFT JOIN events m ON m.rowid = (SELECT e.rowid FROM events e WHERE e.task = t.id AND e.kind = 'move' ORDER BY e.at DESC, e.rowid DESC LIMIT 1)
        WHERE t.status IN ('WAIT FOR HUMAN', 'FAILED') AND t.archived_at IS NULL AND (?1 IS NULL OR t.id = ?1) ORDER BY t.number DESC", [one], |r| {
        let (id, failed): (String, bool) = (r.get(0)?, r.get(1)?);
        let why = r.get::<_, String>(3)?;
        let words = match why.lines().next().unwrap_or("") {
            _ if !failed => format!("{id} waits for you"),
            "" => format!("{id} failed"),
            first => format!("{id} failed: {first}"),
        };
        let key = format!("{}|{words}", r.get::<_, i64>(4)?);
        let due = r.get::<_, bool>(2)? && r.get::<_, Option<String>>(5)?.as_deref() != Some(key.as_str());
        Ok(Asks { id, failed, words, key, due, card: r.get(6)? })
    })
}

/// The human closed or saw task `id`'s line of `key`, or its line as it stands now (its window opened): whether
/// that changed what the store keeps.
pub fn keep_seen(tx: &Transaction, id: &str, key: Option<&str>) -> Result<bool, String> {
    let task = ops::key_of(tx, id)?;
    let key = match key {
        Some(k) => k.to_string(),
        None => match asks(tx, Some(&task))?.pop() {
            Some(a) => a.key,
            None => return Ok(false),                  // it needs no human: no line to see
        },
    };
    let had: Option<Option<String>> = tx.query_row("SELECT line_seen FROM tasks WHERE id = ?1", [&task], |r| r.get(0)).optional().map_err(ops::sql)?;
    if had.flatten().as_deref() == Some(key.as_str()) {
        return Ok(false);
    }
    tx.execute("UPDATE tasks SET line_seen = ?1 WHERE id = ?2", (key, task)).map(|_| true).map_err(ops::sql)
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
            Ok(Held { counts, sessions, attention: asks(tx, None)? })
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
        let lines: Vec<_> = held.attention.iter().filter(|a| a.due).map(|a| json!({"id": a.id, "failed": a.failed, "words": a.words, "key": a.key})).collect();
        // the Kanban key's counts, as the board's cards: what waits for a human, what failed
        let cards = |failed: bool| held.attention.iter().filter(|a| a.card && a.failed == failed).count();
        json!({"t": "taskmarks", "marks": held.sessions, "counts": held.counts, "waiting": cards(false), "failed": cards(true), "lines": lines}).to_string()
    }
}

/// A page that connects learns them.
pub fn hello(app: &Shared, page: &Page) {
    page.send(app.task_marks.message());
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
    use serde_json::Value;

    fn lines(marks: &Marks) -> (Value, Value) {
        let told: Value = serde_json::from_str(&marks.message()).unwrap();
        (json!(told["waiting"].as_u64().unwrap() + told["failed"].as_u64().unwrap()), told["lines"].clone())
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
        store.with(|tx| keep_seen(tx, &t.id, Some("1|an older line"))).unwrap();
        marks.refresh(&store).unwrap();
        assert_eq!(lines(&marks).1[0]["key"], json!(key), "a close of another line leaves this one up");
        assert!(store.with(|tx| keep_seen(tx, &t.id, Some(&key))).unwrap(), "a close is kept");
        assert!(!store.with(|tx| keep_seen(tx, &t.id, Some(&key))).unwrap(), "the same close from another browser changes nothing");
        assert!(marks.refresh(&store).unwrap(), "a close is news for every page");
        assert_eq!(lines(&marks), (json!(1), json!([])), "the line goes; the task still needs a human");
        store.with(|tx| { move_to(tx, HUMAN, &t.id, "IN PROGRESS", 3)?; move_to(tx, DISPATCHER, &t.id, "WAIT FOR HUMAN", 4) }).unwrap();
        marks.refresh(&store).unwrap();
        let (_, told) = lines(&marks);
        assert_eq!(told[0]["words"], json!(format!("{} waits for you", t.id)), "back in the column: its line again, same words");
        assert_ne!(told[0]["key"], json!(key));
        assert!(store.with(|tx| keep_seen(tx, &t.id, None)).unwrap(), "its window opened: the line it has now is seen");
        marks.refresh(&store).unwrap();
        assert_eq!(lines(&marks), (json!(1), json!([])));
        store.with(|tx| move_to(tx, HUMAN, &t.id, "DONE", 5)).unwrap();
        assert!(!store.with(|tx| keep_seen(tx, &t.id, None)).unwrap(), "a task that needs no human has no line to see");
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
        store.with(|tx| keep_seen(tx, &t.id, Some(&key))).unwrap();
        store.with(|tx| ops::log(tx, &t.key, HUMAN, "why", "Its agent: codex", None, 3).map(drop)).unwrap();
        marks.refresh(&store).unwrap();
        assert_eq!(lines(&marks), (json!(1), json!([])), "a human's note neither reopens the line nor rewords it");
        store.with(|tx| move_to(tx, HUMAN, &t.id, "WAIT FOR HUMAN", 4)).unwrap();
        marks.refresh(&store).unwrap();
        assert_eq!(lines(&marks), (json!(1), json!([])), "a human's move has no line");
        assert!(store.with(|tx| keep_seen(tx, "TASK-404", None)).is_err(), "an unknown task is refused");
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
        assert_eq!(told, json!({"t": "taskmarks", "marks": {key: [t.id]}, "counts": {"default": 1}, "waiting": 0, "failed": 0, "lines": []}));
        store.with(|tx| move_to(tx, HUMAN, &t.id, "DONE", 2)).unwrap();
        assert!(marks.refresh(&store).unwrap());
        let told: serde_json::Value = serde_json::from_str(&marks.message()).unwrap();
        assert_eq!(told, json!({"t": "taskmarks", "marks": {}, "counts": {"default": 1}, "waiting": 0, "failed": 0, "lines": []}), "DONE leaves its row, and still counts in its project");
    }

    #[test]
    fn ac39_a_failed_task_says_why_it_failed_never_an_older_moves_why() {
        let store = Store::open(crate::testutil::fresh_dir("marks-older-why").join("kanban.sqlite"));
        let marks = Marks::default();
        let t = store.with(|tx| create(tx, HUMAN, NewTask { title: "a".into(), ..Default::default() }, 1)).unwrap();
        store.with(|tx| { move_to(tx, DISPATCHER, &t.id, "IN PROGRESS", 2)?; ops::log(tx, &t.key, DISPATCHER, "why", "The agent took it.", None, 2).map(drop) }).unwrap();
        store.with(|tx| move_to(tx, DISPATCHER, &t.id, "FAILED", 3)).unwrap();
        marks.refresh(&store).unwrap();
        let line = |m: &Marks| serde_json::from_str::<serde_json::Value>(&m.message()).unwrap()["lines"][0]["words"].clone();
        assert_eq!(line(&marks), json!(format!("{} failed: Dispatcher moved it from IN PROGRESS to FAILED", t.id)), "who moved it, not the start's why");
        store.with(|tx| ops::log(tx, &t.key, DISPATCHER, "why", "No result from the agent within an hour.", None, 3).map(drop)).unwrap();
        marks.refresh(&store).unwrap();
        assert_eq!(line(&marks), json!(format!("{} failed: No result from the agent within an hour.", t.id)), "what was said with its move there");
    }

    #[test]
    fn ac117_kanban_counts_cards_waiting_and_failed_apart_a_subtask_under_its_main_task_once() {
        let store = Store::open(crate::testutil::fresh_dir("marks-cards").join("kanban.sqlite"));
        let marks = Marks::default();
        let task = |tx: &Transaction, title: &str| create(tx, HUMAN, NewTask { title: title.into(), ..Default::default() }, 1);
        let counts = |m: &Marks| {
            let told: Value = serde_json::from_str(&m.message()).unwrap();
            (told["waiting"].clone(), told["failed"].clone())
        };
        store.with(|tx| {
            let (main, sub, other, alone) = (task(tx, "main")?, task(tx, "sub")?, task(tx, "other sub")?, task(tx, "alone")?);
            crate::tasks::family::set_parent(tx, HUMAN, &sub.id, Some(&main.id), 1)?;
            crate::tasks::family::set_parent(tx, HUMAN, &other.id, Some(&main.id), 1)?;
            for (t, to) in [(&main, "FAILED"), (&sub, "FAILED"), (&other, "WAIT FOR HUMAN"), (&alone, "WAIT FOR HUMAN")] {
                move_to(tx, DISPATCHER, &t.id, to, 2)?;
            }
            Ok(())
        }).unwrap();
        marks.refresh(&store).unwrap();
        assert_eq!(counts(&marks), (json!(2), json!(1)), "a subtask in its main task's column counts once with it; in another it is a card");
        assert_eq!(lines(&marks).1.as_array().unwrap().len(), 4, "every one of them still has its line");
    }

    #[test]
    fn ac113_a_store_that_fails_keeps_the_last_numbers() {
        let marks = Marks::default();
        marks.held.lock().counts.insert("api".into(), 3);
        assert!(marks.refresh(&Store::absent()).is_err(), "said, so its caller tries again (D16)");
        assert_eq!(marks.held.lock().counts["api"], 3);
    }
}
