// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A task's comments and its history (AC-107, AC-108, AC-149), each change in the caller's transaction.

use std::collections::HashMap;

use rusqlite::{OptionalExtension, Transaction};
use serde_json::{json, Value};

use super::files::File;
use super::ops::{check_comment, key_of, log, rows, sql, touch, user_name};

/// The latest comment's event if the human already kept these words (a retried Run).
fn same_human_comment(tx: &Transaction, id: &str, text: &str) -> Result<Option<String>, String> {
    let key = key_of(tx, id)?;
    tx.query_row("SELECT CASE WHEN text = ?2 AND author = ?3 AND json_extract(data, '$.choice') IS NULL THEN id END FROM events
                  WHERE task = ?1 AND kind = 'comment' ORDER BY at DESC, rowid DESC LIMIT 1",
                 (&key, text, super::store::HUMAN), |r| r.get(0)).optional().map(Option::flatten).map_err(sql)
}

/// A comment (AC-108): the event's id.
pub fn comment(tx: &Transaction, author: &str, id: &str, text: &str, at: i64) -> Result<String, String> {
    comment_with(tx, author, id, text, None, at)
}

/// A comment by `author`; a project's manager's (terminum-agents.md AC-174) marked so, as a fresh start and a notice
/// say whose it is (terminum-dispatcher.md AC-209): the event's id.
pub fn comment_as(tx: &Transaction, author: &str, manager: bool, id: &str, text: &str, at: i64) -> Result<String, String> {
    comment_with(tx, author, id, text, manager.then(|| json!({"by": "manager"})).as_ref(), at)
}

/// The row of task ?1's last questions (AC-157): the humans' comments after it are their answers.
const ASKED: &str = "SELECT MAX(rowid) FROM events WHERE task = ?1 AND kind = 'comment' AND json_extract(data, '$.questions') IS NOT NULL";

pub(crate) fn last_question(tx: &Transaction, key: &str) -> Result<Option<i64>, String> {
    tx.query_row(ASKED, [key], |r| r.get(0)).map_err(sql)
}

/// Whether event `e` is a comment marked as a project's manager's by `comment_as`, as SQL.
pub(crate) const BY_MANAGER: &str = "coalesce(json_extract(e.data, '$.by') = 'manager', 0)";

/// What a fresh start carries of a task's comments (terminum-dispatcher.md AC-209): the humans' and the project's
/// manager's, oldest first, each with its author's name and whether a manager wrote it; and how many agents wrote,
/// whose words it never carries. Dispatcher's lines are neither.
#[derive(Debug, Default, PartialEq)]
pub struct Carried {
    pub theirs: Vec<(String, bool, String)>,
    pub agents: usize,
}

/// The comments on task `id` a fresh start carries, but the press's own `message` (said as the message: its newest
/// comment of those words, kept before the start read them) and, with `answered`, the humans' answers since the
/// last questions (carried as answers, `answers`).
pub fn carried(tx: &Transaction, id: &str, message: Option<&str>, answered: bool) -> Result<Carried, String> {
    let key = key_of(tx, id)?;
    let asked: Option<i64> = if answered { last_question(tx, &key)? } else { None };
    // an agent's words are only counted: they are not read
    let all = rows(tx, &format!("SELECT e.rowid, u.name, u.kind, {BY_MANAGER}, CASE WHEN u.kind = 'human' OR {BY_MANAGER} THEN coalesce(e.text, '') END
                                 FROM events e JOIN users u ON u.id = e.author WHERE e.task = ?1 AND e.kind = 'comment' AND u.kind <> 'system' ORDER BY e.at, e.rowid"),
                   [&key], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, bool>(3)?, r.get::<_, Option<String>>(4)?)))?;
    let mut out = Carried::default();
    for (row, name, kind, manager, text) in all {
        match text {
            Some(_) if kind == "human" && asked.is_some_and(|q| row > q) => {}
            Some(text) => out.theirs.push((name, manager, text)),
            None => out.agents += 1,
        }
    }
    if let Some(m) = message.map(str::trim) && let Some(i) = out.theirs.iter().rposition(|(_, _, t)| t.trim() == m) {
        out.theirs.remove(i);
    }
    Ok(out)
}

/// An agent's questions for a human (terminum-dispatcher.md AC-157): a comment the task's window shows as such.
pub fn questions(tx: &Transaction, author: &str, id: &str, text: &str, at: i64) -> Result<String, String> {
    record_question(tx, author, id, text, None, None, at)
}

pub(crate) fn normalized_question(args: &Value) -> Result<Value, String> {
    super::choices::question(args).map(|question| json!(question))
}

/// Store an owner question and put its card in the ordinary waiting column.
pub(crate) fn wait_for_owner(tx: &Transaction, by: &str, task: &super::ops::Task, text: &str, args: &Value, at: i64) -> Result<String, String> {
    wait_in(tx, by, task, text, args, None, at)
}

/// As `wait_for_owner`, asked in conversation `asked_in` when known, where an answer is typed (terminum-dispatcher.md AC-285).
pub(crate) fn wait_in(tx: &Transaction, by: &str, task: &super::ops::Task, text: &str, args: &Value, asked_in: Option<&str>, at: i64) -> Result<String, String> {
    let question = super::choices::question(args)?;
    let event = record_question(tx, by, &task.id, text, question.as_ref(), asked_in, at)?;
    if task.status != "WAIT FOR HUMAN" { super::ops::move_to(tx, by, &task.id, "WAIT FOR HUMAN", at)?; }
    Ok(event)
}

/// `asked_in`: the conversation that asks, an answer typed only where it still runs (terminum-dispatcher.md AC-285).
pub(super) fn record_question(tx: &Transaction, author: &str, id: &str, text: &str, recorded: Option<&super::choices::Question>, asked_in: Option<&str>, at: i64) -> Result<String, String> {
    let mut data = json!({"questions": true});
    if let Some(question) = recorded {
        data["question"] = json!(question);
    }
    if let Some(session) = asked_in {
        data["asked_in"] = json!(session);
    }
    comment_with(tx, author, id, text, Some(&data), at)
}

/// Dispatcher's words on a task leaving the integration queue, marked with how it left (`terminum-integration.md`
/// AC-187): `landed`, `red`, `conflict` or `out`, read by `terminum integration wait`.
pub fn left_queue(tx: &Transaction, author: &str, id: &str, text: &str, how: &str, at: i64) -> Result<String, String> {
    notice(tx, author, id, text, json!({"left": how}), at)
}

/// A notice of the integration queue to a task's agent (`terminum-integration.md` AC-192): a comment the queue's
/// loop also hands its session, `data`'s fields kept with it (what it was said for, so it is said once).
pub fn notice(tx: &Transaction, author: &str, id: &str, text: &str, mut data: Value, at: i64) -> Result<String, String> {
    data["notice"] = json!(true);
    comment_with(tx, author, id, text, Some(&data), at)
}

/// Whether task `key` has a notice by `author` whose field `field` is `value`: said already.
pub fn noticed(tx: &Transaction, author: &str, key: &str, field: &'static str, value: &str) -> Result<bool, String> {
    tx.query_row(&format!("SELECT EXISTS (SELECT 1 FROM events WHERE task = ?1 AND kind = 'comment' AND author = ?2
                           AND json_extract(data, '$.notice') IS NOT NULL AND json_extract(data, '$.{field}') = ?3)"), (key, author, value), |r| r.get(0)).map_err(sql)
}

/// How task `key` last left the integration queue, since it was last queued: none while it stands there, or when
/// it left by a move no mark goes with (a human's drag, withdraw_integration).
pub fn how_left(tx: &Transaction, key: &str) -> Result<Option<String>, String> {
    tx.query_row("SELECT json_extract(data, '$.left') FROM events WHERE task = ?1 AND json_extract(data, '$.left') IS NOT NULL
                  AND rowid > coalesce((SELECT max(rowid) FROM events WHERE task = ?1 AND kind = 'queue'), 0) ORDER BY rowid DESC LIMIT 1", [key], |r| r.get(0))
        .optional().map_err(sql)
}

/// Dispatcher's newest comments on task `key`, at most `most`, oldest first: (event, text).
pub fn newest_by(tx: &Transaction, key: &str, author: &str, most: i64) -> Result<Vec<(String, String)>, String> {
    rows(tx, "SELECT id, text FROM (SELECT id, coalesce(text, '') AS text, at, rowid FROM events WHERE task = ?1 AND kind = 'comment' AND author = ?2 ORDER BY at DESC, rowid DESC LIMIT ?3) ORDER BY at, rowid",
         (key, author, most), |r| Ok((r.get(0)?, r.get(1)?)))
}

pub(super) fn comment_with(tx: &Transaction, author: &str, id: &str, text: &str, data: Option<&Value>, at: i64) -> Result<String, String> {
    check_comment(text)?;
    let key = key_of(tx, id)?;
    touch(tx, &key, at)?;
    log(tx, &key, author, "comment", text, data, at)
}

/// The human's comment of a press, with its files, once: a press tried again, or pressed again by its wait
/// (terminum-dispatcher.md AC-158), adds it no second time. Its event, when it was kept now.
pub fn keep_human_comment(tx: &Transaction, id: &str, text: &str, files: &[String], at: i64) -> Result<(String, bool), String> {
    if let Some(event) = same_human_comment(tx, id, text)? {
        return Ok((event, false));
    }
    let event = comment(tx, super::store::HUMAN, id, text, at)?;
    super::files::attach(tx, files, id, Some(&event))?;
    Ok((event, true))
}

/// A human's comment taken back, its files free again: a press that did not go keeps nothing (terminum-dispatcher.md
/// AC-285).
pub(crate) fn retract(tx: &Transaction, event: &str) -> Result<(), String> {
    tx.execute("UPDATE files SET task = NULL, event = NULL WHERE event = ?1", [event]).map_err(sql)?;
    tx.execute("DELETE FROM events WHERE id = ?1 AND kind = 'comment'", [event]).map(drop).map_err(sql)
}

/// Keep the field of a grouped human Run on another task, tied to its original event for retries.
pub(crate) fn copy_run_draft(tx: &Transaction, task: &str, text: &str, original: &str, at: i64) -> Result<String, String> {
    comment_with(tx, super::store::HUMAN, task, text, Some(&json!({"run_draft": original})), at)
}

pub(crate) fn run_draft_copy(tx: &Transaction, task: &str, original: &str) -> Result<Option<String>, String> {
    tx.query_row("SELECT id FROM events WHERE task = ?1 AND json_extract(data, '$.run_draft') = ?2",
        (task, original), |r| r.get(0)).optional().map_err(sql)
}

/// The humans' comments since the agent's last questions, oldest first: their answers (AC-157); none asked: none.
pub fn answers(tx: &Transaction, id: &str) -> Result<Option<Vec<String>>, String> {
    let key = key_of(tx, id)?;
    let asked: Option<i64> = last_question(tx, &key)?;
    let Some(at) = asked else { return Ok(None) };
    rows(tx, "SELECT e.text FROM events e JOIN users u ON u.id = e.author WHERE e.task = ?1 AND e.kind = 'comment' AND u.kind = 'human' AND e.text IS NOT NULL
              AND e.rowid > ?2 ORDER BY e.at, e.rowid", (&key, at), |r| r.get(0)).map(Some)
}

/// A human's own comment changed or deleted: a new event holds it, and the old text leaves the
/// shown history (AC-108). Those of agents and Dispatcher stay as written. Whether its text changed.
pub fn change_comment(tx: &Transaction, author: &str, id: &str, event: &str, text: Option<&str>, at: i64) -> Result<bool, String> {
    let key = key_of(tx, id)?;
    let (kind, by, was, choice): (String, String, Option<String>, bool) = tx.query_row("SELECT kind, author, text, json_extract(data, '$.choice') IS NOT NULL FROM events WHERE id = ?1 AND task = ?2", (event, &key), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .optional().map_err(sql)?.ok_or("That comment is gone.")?;
    if kind != "comment" || by != author {
        return Err("Only its human author can change a comment.".into());
    }
    if choice && text.is_some() {
        return Err("Remove the chosen option before changing your answer.".into());
    }
    check_comment(text.unwrap_or(""))?;
    let who = user_name(tx, author)?;
    match text {
        Some(t) => {
            tx.execute("UPDATE events SET text = ?1, edited_at = ?2 WHERE id = ?3", (t, at, event)).map_err(sql)?;
            log(tx, &key, author, "edit", &format!("{who} changed a comment"), None, at)?;
        }
        None => {
            tx.execute("UPDATE events SET kind = 'deleted', text = '', data = json_remove(data, '$.choice') WHERE id = ?1", [event]).map_err(sql)?;
            log(tx, &key, author, "edit", &format!("{who} deleted a comment"), None, at)?;
        }
    }
    touch(tx, &key, at)?;
    Ok(text.is_some_and(|t| was.as_deref() != Some(t)))
}

/// A task's comments and history, oldest first, with its files by event.
pub fn history(tx: &Transaction, id: &str) -> Result<Value, String> {
    let key = key_of(tx, id)?;
    let mut files: HashMap<String, Vec<File>> = HashMap::new();
    for (event, file) in rows(tx, "SELECT event, id, name, bytes, mime FROM files WHERE task = ?1 AND event IS NOT NULL", [&key], |r| Ok((r.get(0)?, File::of_row(r, 1)?)))? {
        files.entry(event).or_default().push(file);
    }
    let out = rows(tx, "SELECT e.id, e.at, e.author, u.name, u.kind, e.kind, e.text, e.edited_at, json_extract(e.data, '$.questions') IS NOT NULL,
                        CASE WHEN e.kind = 'told' THEN json_extract(e.data, '$.comments') END, CASE WHEN e.kind = 'told' THEN json_extract(e.data, '$.not') END,
                        CASE WHEN e.kind = 'comment' THEN json_extract(e.data, '$.question') END,
                        CASE WHEN e.kind = 'comment' THEN json_extract(e.data, '$.choice') END,
                        CASE WHEN e.kind = 'answer' THEN e.data END
                        FROM events e JOIN users u ON u.id = e.author WHERE e.task = ?1 ORDER BY e.at, e.rowid", [&key], |r| {
        let event: String = r.get(0)?;
        let attached = files.remove(&event).unwrap_or_default();
        let mut v = json!({"id": event, "at": r.get::<_, i64>(1)?, "author": r.get::<_, String>(2)?,
            "author_name": r.get::<_, String>(3)?, "author_kind": r.get::<_, String>(4)?,
            "kind": r.get::<_, String>(5)?, "text": r.get::<_, Option<String>>(6)?, "edited_at": r.get::<_, Option<i64>>(7)?, "files": attached});
        if r.get::<_, bool>(8)? {
            v["questions"] = json!(true);
        }
        // a notice's outcome names the comments it told of (terminum-dispatcher.md AC-209)
        if let Some(ids) = r.get::<_, Option<String>>(9)?.and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
            v["comments"] = ids;
            v["not"] = json!(r.get::<_, Option<bool>>(10)?.unwrap_or(false));
        }
        // a key's answer to a question: its choice, the question, how it went (terminum-dispatcher.md AC-285)
        for (column, field) in [(11, "question"), (12, "choice"), (13, "answer")] {
            if let Some(data) = r.get::<_, Option<String>>(column)?.and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
                v[field] = data;
            }
        }
        Ok(v)
    })?;
    Ok(Value::Array(out))
}
