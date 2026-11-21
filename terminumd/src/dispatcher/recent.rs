// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Saved human comments carried by a later human Run (terminum-dispatcher.md AC-122).

use rusqlite::{OptionalExtension, Transaction};

use crate::tasks::comments::last_question;
use crate::tasks::ops::{self, Task};

use super::gate::has_stray;
use crate::page::typing::PASTE_MAX;

use super::press::{COMMENT_SEPARATOR, again_words};

pub(super) const BAD_COMMENT: &str = "A saved comment has control or invisible characters. Edit it before running again.";

pub(super) struct Again {
    pub words: String,
    pub through: Vec<(String, String)>,
}

impl From<String> for Again {
    fn from(words: String) -> Self {
        Self { words, through: Vec::new() }
    }
}

/// A task's own event ID survives database rebuilds and purges of other tasks.
pub(super) fn through(tx: &Transaction, tasks: &[Task]) -> Result<Vec<(String, String)>, String> {
    let mut stmt = tx.prepare_cached("SELECT id FROM events WHERE task = ?1 ORDER BY rowid DESC LIMIT 1").map_err(ops::sql)?;
    tasks.iter().map(|t| stmt.query_row([&t.key], |r| r.get(0)).map(|id| (t.key.clone(), id)).map_err(ops::sql)).collect()
}

/// A live paste succeeded, or a new run started with the prepared prompt. Record exactly its snapshot.
pub(super) fn record(tx: &Transaction, through: &[(String, String)]) -> Result<(), String> {
    let mut stmt = tx.prepare_cached("UPDATE tasks SET run_comments = ?2 WHERE id = ?1
        AND coalesce((SELECT rowid FROM events WHERE id = run_comments), 0) < (SELECT rowid FROM events WHERE id = ?2)").map_err(ops::sql)?;
    for (task, event) in through {
        stmt.execute((task, event)).map_err(ops::sql)?;
    }
    Ok(())
}

/// A replacement process has not taken the continuation yet. Keep its cutoff beside its prompt.
pub(super) fn stage(tx: &Transaction, run: &str, through: &[(String, String)]) -> Result<(), String> {
    if !through.is_empty() {
        let kept = serde_json::to_string(through).map_err(|e| e.to_string())?;
        tx.execute("UPDATE runs SET pending_comments = ?2 WHERE id = ?1", (run, kept)).map_err(ops::sql)?;
    }
    Ok(())
}

/// Advance the cutoff only when the new process received the continuation through the gate.
pub(super) fn delivered(tx: &Transaction, run: &str) -> Result<(), String> {
    let pending: Option<String> = tx.query_row("SELECT pending_comments FROM runs WHERE id = ?1", [run], |r| r.get(0)).map_err(ops::sql)?;
    if let Some(pending) = pending {
        let through: Vec<(String, String)> = serde_json::from_str(&pending).map_err(|e| e.to_string())?;
        record(tx, &through)?;
        tx.execute("UPDATE runs SET pending_comments = NULL WHERE id = ?1", [run]).map_err(ops::sql)?;
    }
    Ok(())
}

/// Older waits kept their draft without event IDs. Reuse the matching comments they kept, even if a newer one follows.
pub(super) fn legacy_drafts(tx: &Transaction, tasks: &[Task], draft: &str, at: i64) -> Result<Vec<String>, String> {
    let mut stmt = tx.prepare_cached("SELECT id FROM events WHERE task = ?1 AND kind = 'comment' AND author = ?2
        AND text = ?3 AND at <= ?4 ORDER BY at DESC, rowid DESC LIMIT 1").map_err(ops::sql)?;
    let mut events = Vec::new();
    for task in tasks {
        if let Some(id) = stmt.query_row((&task.key, crate::tasks::store::HUMAN, draft, at), |r| r.get(0)).optional().map_err(ops::sql)? {
            events.push(id);
        }
    }
    Ok(events)
}

fn too_long() -> String {
    "Saved comments make this prompt larger than 1 MiB. Shorten them before running again.".to_string()
}

pub(super) struct Saved {
    comments: Vec<String>,
    asked: bool,
    answered: bool,
    earlier: bool,
}

/// Read only human comments after each task's cutoff. Exclude the press's own field even when it was kept for a wait.
pub(super) fn saved(tx: &Transaction, tasks: &[Task], drafts: &[String], asked: bool) -> Result<Saved, String> {
    let mut out = Saved { comments: Vec::new(), asked: false, answered: false, earlier: false };
    let mut saved = Vec::new();
    let mut size = 0;
    let drafts = serde_json::to_string(drafts).map_err(|e| e.to_string())?;
    let mut stmt = tx.prepare_cached(
        "SELECT e.at, e.rowid, e.text FROM tasks t JOIN events e ON e.task = t.id JOIN users u ON u.id = e.author
         WHERE t.id = ?1 AND e.kind = 'comment' AND u.kind = 'human' AND e.text IS NOT NULL
           AND e.id NOT IN (SELECT value FROM json_each(?2))
           AND e.rowid > coalesce((SELECT rowid FROM events WHERE id = t.run_comments), 0)").map_err(ops::sql)?;
    for task in tasks {
        let question = if asked { last_question(tx, &task.key)? } else { None };
        out.asked |= question.is_some();
        let mut rows = stmt.query((&task.key, &drafts)).map_err(ops::sql)?;
        while let Some(row) = rows.next().map_err(ops::sql)? {
            let at: i64 = row.get(0).map_err(ops::sql)?;
            let rank: i64 = row.get(1).map_err(ops::sql)?;
            let text: String = row.get(2).map_err(ops::sql)?;
            if has_stray(&text) {
                return Err(BAD_COMMENT.to_string());
            }
            let answer = question.is_some_and(|q| rank > q);
            out.answered |= answer;
            out.earlier |= !answer;
            let text = if tasks.len() > 1 { format!("{}: {text}", task.id) } else { text };
            size += text.len() + if saved.is_empty() { 0 } else { COMMENT_SEPARATOR.len() };
            if size > PASTE_MAX {
                return Err(too_long());
            }
            saved.push((at, rank, text));
        }
    }
    saved.sort_unstable_by_key(|(at, row, _)| (*at, *row));
    out.comments = saved.into_iter().map(|(_, _, text)| text).collect();
    Ok(out)
}

impl Saved {
    pub(super) fn words(mut self, id: &str, who: &str, draft: Option<&str>) -> Result<String, String> {
        let carries_saved = !self.comments.is_empty();
        let words = if self.asked && (self.answered || draft.is_some()) {
            self.comments.extend(draft.map(str::to_string));
            let answers = again_words(id, who, None, Some(&self.comments), false);
            if self.earlier {
                format!("{id}: The comments below also include saved comments from before your questions.\n\n{answers}")
            } else {
                answers
            }
        } else {
            let answers = self.asked.then_some([].as_slice());
            let again = again_words(id, who, draft, answers, false);
            if carries_saved {
                format!("{id}: saved human comments, oldest first. Take them into account.\n\n{}\n\n{again}", self.comments.join(COMMENT_SEPARATOR))
            } else {
                again
            }
        };
        if carries_saved && words.len() > PASTE_MAX {
            return Err(too_long());
        }
        Ok(words)
    }
}

#[cfg(test)]
#[path = "recent_tests.rs"]
mod tests;
