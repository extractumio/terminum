// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A batch in the store (`terminum-integration.md` AC-188, AC-189, AC-194): made with its integration task I_<ID>,
//! its rounds, and what each task of the queue is told, once, in the fixed words of §9 "Notices".

use rusqlite::{OptionalExtension, Transaction};

use super::batch::{Choice, Conflict, Queued};
use crate::tasks::integration::Settings;
use crate::tasks::integration_queue::and_list;
use crate::tasks::ops::{self, sql, NewTask, INTEGRATION};
use crate::tasks::store::DISPATCHER;

/// The states of a batch the machine works on: one at a time over all projects (AC-188).
pub const BUSY: [&str; 5] = ["checking", "green", "publishing", "after", "built"];
/// A batch that waits for its agent or a human: it holds only its own project's queue.
pub const RED: &str = "red";

#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    pub id: i64,
    pub project: String,
    pub number: i64,
    /// its integration task's key
    pub task: Option<String>,
    /// its tasks' keys, in the queue's order
    pub tasks: Vec<String>,
    pub base: String,
    pub candidate: Option<String>,
    pub state: String,
    pub pane: Option<String>,
    pub round: i64,
    pub red_rounds: i64,
    pub step_started: Option<i64>,
}

const SELECT: &str = "SELECT id, project, number, task, tasks, base_sha, candidate_sha, state, pane, round, red_rounds, step_started FROM integration_batches";

fn of_row(r: &rusqlite::Row) -> rusqlite::Result<Batch> {
    let tasks: String = r.get(4)?;
    Ok(Batch { id: r.get(0)?, project: r.get(1)?, number: r.get(2)?, task: r.get(3)?, tasks: serde_json::from_str(&tasks).unwrap_or_default(),
        base: r.get(5)?, candidate: r.get(6)?, state: r.get(7)?, pane: r.get(8)?, round: r.get(9)?, red_rounds: r.get(10)?, step_started: r.get(11)? })
}

/// The batch the machine works on, if any.
pub fn busy(tx: &Transaction) -> Result<Option<Batch>, String> {
    let states = BUSY.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", ");
    tx.query_row(&format!("{SELECT} WHERE state IN ({states}) ORDER BY id LIMIT 1"), [], of_row).optional().map_err(sql)
}

pub fn by_id(tx: &Transaction, id: i64) -> Result<Batch, String> {
    tx.query_row(&format!("{SELECT} WHERE id = ?1"), [id], of_row).map_err(sql)
}

/// The next project to take a batch from: integration on and not paused, on this Mac, a task queued and not in a
/// batch, no red batch of its own; the one whose oldest entry waited longest first.
pub fn next_project(tx: &Transaction) -> Result<Option<String>, String> {
    tx.query_row("SELECT e.project FROM integration_entries e JOIN project_integration i ON i.project = e.project JOIN projects p ON p.id = e.project
                  WHERE e.batch IS NULL AND i.is_on = 1 AND i.paused = 0 AND p.host IS NULL
                    AND NOT EXISTS (SELECT 1 FROM integration_batches b WHERE b.project = e.project AND b.state = 'red')
                  GROUP BY e.project ORDER BY min(e.queued_at) LIMIT 1", [], |r| r.get(0)).optional().map_err(sql)
}

/// A project's queued commits not in a batch, in the queue's order.
pub fn waiting(tx: &Transaction, project: &str) -> Result<Vec<Queued>, String> {
    ops::rows(tx, "SELECT e.task, t.slug, e.commit_sha FROM integration_entries e JOIN tasks t ON t.id = e.task
                   WHERE e.project = ?1 AND e.batch IS NULL ORDER BY e.queued_at, t.number", [project],
        |r| Ok(Queued { task: r.get(0)?, id: r.get(1)?, commit: r.get(2)? }))
}

/// Why a project's queue waits, for its column's head; none clears it.
pub fn waits(tx: &Transaction, project: &str, why: Option<&str>) -> Result<(), String> {
    tx.execute("UPDATE project_integration SET waits_why = ?2 WHERE project = ?1", (project, why)).map(drop).map_err(sql)
}

/// Tells task `key` `words` once for `kind` (its entry's told list), as Dispatcher's comment.
pub fn tell(tx: &Transaction, key: &str, kind: &str, words: &str, at: i64) -> Result<(), String> {
    let told: Option<String> = tx.query_row("SELECT told FROM integration_entries WHERE task = ?1", [key], |r| r.get(0)).optional().map_err(sql)?;
    if told.as_deref().is_some_and(|t| t.split('\n').any(|k| k == kind)) {
        return Ok(());
    }
    tx.execute("UPDATE integration_entries SET told = told || ?2 || char(10) WHERE task = ?1", (key, kind)).map_err(sql)?;
    ops::comment(tx, DISPATCHER, &slug(tx, key)?, words, at).map(drop)
}

pub fn slug(tx: &Transaction, key: &str) -> Result<String, String> {
    tx.query_row("SELECT slug FROM tasks WHERE id = ?1", [key], |r| r.get(0)).map_err(sql)
}

/// Back to its agent: IN PROGRESS, out of the queue, told why (§9 "returned").
pub fn give_back(tx: &Transaction, key: &str, words: &str, at: i64) -> Result<(), String> {
    let t = ops::get(tx, &slug(tx, key)?)?;
    if t.status == INTEGRATION {
        ops::place(tx, DISPATCHER, &t, "IN PROGRESS", at)?;
    }
    ops::comment(tx, DISPATCHER, &t.id, words, at).map(drop)
}

fn short(sha: &str) -> &str {
    &sha[..9.min(sha.len())]
}

/// The returned's words (§9 Notices).
pub fn conflict_words(q: &Queued, c: &Conflict, s: &Settings, base: &str) -> String {
    match c {
        Conflict::Target(files) => format!("{} returned: conflict with {} at {} in {}. Merge {}, resolve, commit, and integrate again.", q.id, s.target, short(base), files.join(", "), short(base)),
        Conflict::Carries(other) => format!("{} returned: it carries {other}'s commits, which did not land. Merge {}, take them out or wait for {other}, and integrate again.", q.id, s.target),
        Conflict::Git(words) => format!("{} returned: git could not merge {}: {words}", q.id, short(&q.commit)),
    }
}

/// The pass's batch in the store: its I_ task, its round 1, each task told; the returned given back, the held
/// told what they wait for. None when nothing was chosen.
pub fn open(tx: &Transaction, project: &str, s: &Settings, base: &str, c: &Choice, at: i64) -> Result<Option<Batch>, String> {
    for (q, why) in &c.returned {
        give_back(tx, &q.task, &conflict_words(q, why, s, base), at)?;
    }
    let number: i64 = tx.query_row("SELECT coalesce(max(number), 0) + 1 FROM integration_batches WHERE project = ?1", [project], |r| r.get(0)).map_err(sql)?;
    for (q, waits) in &c.held {
        tx.execute("UPDATE integration_entries SET waits_for = ?2 WHERE task = ?1", (&q.task, waits.join(" "))).map_err(sql)?;
        tell(tx, &q.task, &format!("held {}", waits.join(" ")), &format!("{} is held back: it does not merge with {} of batch {number}. Nothing to do until that batch ends; it is judged again then.", q.id, and_list(&waits.iter().map(String::as_str).collect::<Vec<_>>())), at)?;
    }
    let Some(candidate) = &c.candidate else { return Ok(None) };
    let ids: Vec<&str> = c.chosen.iter().map(|q| q.id.as_str()).collect();
    let text = facts(s, base, candidate, &c.chosen, project, number);
    let task = ops::create(tx, DISPATCHER, NewTask { title: format!("Integration batch {number}: {}", ids.join(", ")), text, project: Some(project.to_string()),
        severity: Some("P1".into()), owner: Some(DISPATCHER.into()), integration: true, ..Default::default() }, at)?;
    let keys: Vec<String> = c.chosen.iter().map(|q| q.task.clone()).collect();
    let state = if s.check.is_empty() { "green" } else { "checking" };
    tx.execute("INSERT INTO integration_batches (project, number, task, tasks, base_sha, candidate_sha, state, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
               rusqlite::params![project, number, task.key.clone(), serde_json::to_string(&keys).unwrap_or_default(), base, candidate, state, at]).map_err(sql)?;
    let id = tx.last_insert_rowid();
    tx.execute("INSERT INTO integration_rounds (batch, round, tasks, base_sha, candidate_sha, result) VALUES (?1, 1, ?2, ?3, ?4, 'built')",
               rusqlite::params![id, serde_json::to_string(&keys).unwrap_or_default(), base, candidate]).map_err(sql)?;
    for q in &c.chosen {
        tx.execute("UPDATE integration_entries SET batch = ?2, waits_for = NULL WHERE task = ?1", (&q.task, id)).map_err(sql)?;
        let mates: Vec<&str> = ids.iter().copied().filter(|i| *i != q.id).collect();
        let with = if mates.is_empty() { ", alone".to_string() } else { format!(" with {}", and_list(&mates)) };
        tell(tx, &q.task, &format!("batch {id}"), &format!("{} is in batch {number}{with}: {}.", q.id, if s.check.is_empty() { "no check runs, it is published" } else { "its check runs" }), at)?;
    }
    by_id(tx, id).map(Some)
}

/// The integration task's text: the facts of its batch, never a task's title or text (§9 Security).
fn facts(s: &Settings, base: &str, candidate: &str, chosen: &[Queued], project: &str, number: i64) -> String {
    let tasks: String = chosen.iter().map(|q| format!("- {}: {}\n", q.id, short(&q.commit))).collect();
    let check = if s.check.is_empty() { "none: a candidate that merges is published".to_string() } else { format!("`{}`", s.check) };
    format!("## Batch {number}\nTarget {} at {}; candidate {}.\n\n## Tasks\n{tasks}\n## Check\n{check}, in {}; its logs in the same folder's parent.\n",
            s.target, short(base), short(candidate), crate::tasks::integration::tree_of(project).display())
}

pub fn set_state(tx: &Transaction, id: i64, state: &str, why: Option<&str>, at: i64) -> Result<(), String> {
    let ended = matches!(state, "ended" | "given up");
    tx.execute("UPDATE integration_batches SET state = ?2, why = ?3, ended_at = CASE WHEN ?4 THEN ?5 ELSE ended_at END WHERE id = ?1", rusqlite::params![id, state, why, ended, at]).map(drop).map_err(sql)
}

pub fn set_pane(tx: &Transaction, id: i64, pane: Option<&str>, at: i64) -> Result<(), String> {
    tx.execute("UPDATE integration_batches SET pane = ?2, step_started = ?3 WHERE id = ?1", rusqlite::params![id, pane, pane.map(|_| at)]).map(drop).map_err(sql)
}

pub fn round_ends(tx: &Transaction, b: &Batch, result: &str, why: Option<&str>, at: i64) -> Result<(), String> {
    tx.execute("UPDATE integration_rounds SET result = ?3, why = ?4, ended = ?5, started = coalesce(started, ?6) WHERE batch = ?1 AND round = ?2",
               rusqlite::params![b.id, b.round, result, why, at, b.step_started]).map(drop).map_err(sql)
}
