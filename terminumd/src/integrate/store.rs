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
pub const BUSY: [&str; 4] = ["checking", "green", "publishing", "rebuild"];
/// The states of a published batch whose After or Cleanup runs: one at a time over all projects, beside the batch
/// in work (AC-191).
pub const TAIL: [&str; 2] = ["after", "cleanup"];
/// A batch whose tasks are DONE and whose After waits for its turn (AC-191).
pub const PUBLISHED: &str = "published";
/// A published batch whose After failed: a human closes it, and it holds its project's queue and later Afters.
pub const AFTER_FAILED: &str = "after failed";
/// A published batch whose After said it cannot run now (status 75): run again after a pause, holding only its
/// project's later Afters (AC-191).
pub const AFTER_WAITS: &str = "after waits";
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
    /// its integration agent's fixes: commits on the target, built in before its tasks (AC-194)
    pub fixes: Vec<String>,
    /// its integration agent's own worktree, while it has one
    pub agent_tree: Option<String>,
    pub unchanged_rechecks: i64,
    /// why it stands as it does: its red round's, its refusal's
    pub why: Option<String>,
    pub created_at: i64,
    /// when its After first said it cannot run now (AC-191)
    pub waits_since: Option<i64>,
}

const SELECT: &str = "SELECT id, project, number, task, tasks, base_sha, candidate_sha, state, pane, round, red_rounds, step_started, fixes, agent_tree, unchanged_rechecks, why, created_at, waits_since FROM integration_batches";

fn of_row(r: &rusqlite::Row) -> rusqlite::Result<Batch> {
    let list = |i: usize| r.get::<_, String>(i).map(|t| serde_json::from_str(&t).unwrap_or_default());
    Ok(Batch { id: r.get(0)?, project: r.get(1)?, number: r.get(2)?, task: r.get(3)?, tasks: list(4)?,
        base: r.get(5)?, candidate: r.get(6)?, state: r.get(7)?, pane: r.get(8)?, round: r.get(9)?, red_rounds: r.get(10)?, step_started: r.get(11)?,
        fixes: list(12)?, agent_tree: r.get(13)?, unchanged_rechecks: r.get(14)?, why: r.get(15)?, created_at: r.get(16)?, waits_since: r.get(17)? })
}

/// A project's batches, newest first, `most` of those numbered under `before`.
pub fn of_project(tx: &Transaction, project: &str, before: i64, most: i64) -> Result<Vec<Batch>, String> {
    ops::rows(tx, &format!("{SELECT} WHERE project = ?1 AND number < ?2 ORDER BY number DESC LIMIT ?3"), rusqlite::params![project, before, most], of_row)
}

/// The batch whose integration task is `key`, if one.
pub fn by_task(tx: &Transaction, key: &str) -> Result<Option<Batch>, String> {
    tx.query_row(&format!("{SELECT} WHERE task = ?1"), [key], of_row).optional().map_err(sql)
}

/// The number of the batch whose integration task is `key`, while that batch is not over: the task ends with it.
pub fn live_by_task(tx: &Transaction, key: &str) -> Result<Option<i64>, String> {
    tx.query_row("SELECT number FROM integration_batches WHERE task = ?1 AND state NOT IN ('ended', 'given up')", [key], |r| r.get(0)).optional().map_err(sql)
}

/// The batch the machine works on, if any.
pub fn busy(tx: &Transaction) -> Result<Option<Batch>, String> {
    first_in(tx, &BUSY)
}

/// The batch whose After or Cleanup runs, if any.
pub fn in_tail(tx: &Transaction) -> Result<Option<Batch>, String> {
    first_in(tx, &TAIL)
}

fn first_in(tx: &Transaction, states: &[&str]) -> Result<Option<Batch>, String> {
    let states = states.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", ");
    tx.query_row(&format!("{SELECT} WHERE state IN ({states}) ORDER BY id LIMIT 1"), [], of_row).optional().map_err(sql)
}

/// A failed After run again by a human (AC-191): it waits with no last try (a failed After has no `step_started`), so
/// it takes the tail's next turn among its project's, a new wait counted from its own first.
pub fn after_again(tx: &Transaction, b: &Batch, why: &str) -> Result<(), String> {
    tx.execute("UPDATE integration_batches SET state = ?2, why = ?3, waits_since = NULL WHERE id = ?1",
               rusqlite::params![b.id, AFTER_WAITS, why]).map(drop).map_err(sql)
}

/// After passed: an integration task a failed After left waiting is DONE, by Dispatcher (AC-191).
pub fn after_passed(tx: &Transaction, b: &Batch, at: i64) -> Result<(), String> {
    let Some(id) = task_of(tx, b)? else { return Ok(()) };
    let t = ops::get(tx, &id)?;
    if t.status == "DONE" {
        return Ok(());
    }
    ops::comment(tx, DISPATCHER, &id, &format!("After passed for batch {} when run again.", b.number), at)?;
    ops::place(tx, DISPATCHER, &t, "DONE", at)
}

/// The project's oldest batch in `state`, if one.
fn first_of(tx: &Transaction, project: &str, state: &str) -> Result<Option<Batch>, String> {
    tx.query_row(&format!("{SELECT} WHERE project = ?1 AND state = ?2 ORDER BY id LIMIT 1"), [project, state], of_row).optional().map_err(sql)
}

/// The project's batch whose After waits, if one (AC-191).
pub fn after_waiting(tx: &Transaction, project: &str) -> Result<Option<Batch>, String> {
    first_of(tx, project, AFTER_WAITS)
}

/// The project's batch whose After failed, while a human has not closed it.
pub fn after_failed(tx: &Transaction, project: &str) -> Result<Option<Batch>, String> {
    first_of(tx, project, AFTER_FAILED)
}

/// The batch whose After's turn comes: the oldest published or waiting one of each project, whose later ones wait
/// behind it, of a project with no batch whose After failed (a human decides that one first, and no newer batch's
/// outcome replaces it); one that waits only once `pause` seconds passed since its last try (AC-191).
pub fn next_after(tx: &Transaction, now: i64, pause: i64) -> Result<Option<Batch>, String> {
    let mut seen = std::collections::HashSet::new();
    for b in ops::rows(tx, &format!("{SELECT} WHERE state IN ('{PUBLISHED}', '{AFTER_WAITS}') ORDER BY id"), [], of_row)? {
        if !seen.insert(b.project.clone()) || after_failed(tx, &b.project)?.is_some() {
            continue;
        }
        // a human's After again has no last try: no pause
        if b.state == AFTER_WAITS && b.step_started.is_some_and(|tried| now - tried < pause) {
            continue;
        }
        return Ok(Some(b));
    }
    Ok(None)
}

/// Its After cannot run now: it waits with `why`, its last try ended `at` (`step_started`), its first wait kept; by
/// hand, not set_state and set_pane, which clear `step_started`.
pub fn after_waits(tx: &Transaction, b: &Batch, why: &str, at: i64) -> Result<(), String> {
    tx.execute("UPDATE integration_batches SET state = ?2, why = ?3, pane = NULL, step_started = ?4, waits_since = coalesce(waits_since, ?4) WHERE id = ?1",
               rusqlite::params![b.id, AFTER_WAITS, why, at]).map(drop).map_err(sql)
}

pub fn by_id(tx: &Transaction, id: i64) -> Result<Batch, String> {
    tx.query_row(&format!("{SELECT} WHERE id = ?1"), [id], of_row).map_err(sql)
}

/// The next project to take a batch from: integration on and not paused, on this Mac, a task queued and not in a
/// batch, no red batch of its own; the one whose oldest entry waited longest first.
pub fn next_project(tx: &Transaction) -> Result<Option<String>, String> {
    tx.query_row("SELECT e.project FROM integration_entries e JOIN project_integration i ON i.project = e.project JOIN projects p ON p.id = e.project
                  WHERE e.batch IS NULL AND e.stage IS NULL AND i.is_on = 1 AND i.paused = 0 AND p.host IS NULL
                    AND NOT EXISTS (SELECT 1 FROM integration_batches b WHERE b.project = e.project AND b.state IN ('red', 'not published', 'after failed'))
                  GROUP BY e.project ORDER BY min(e.queued_at) LIMIT 1", [], |r| r.get(0)).optional().map_err(sql)
}

/// A project's queued commits not in a batch, in the queue's order.
pub fn waiting(tx: &Transaction, project: &str) -> Result<Vec<Queued>, String> {
    ops::rows(tx, "SELECT e.task, t.slug, e.commit_sha FROM integration_entries e JOIN tasks t ON t.id = e.task
                   WHERE e.project = ?1 AND e.batch IS NULL AND e.stage IS NULL AND t.archived_at IS NULL ORDER BY e.queued_at, t.number", [project],
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
    ops::notice(tx, DISPATCHER, &slug(tx, key)?, words, serde_json::json!({"told": kind}), at).map(drop)
}

pub fn slug(tx: &Transaction, key: &str) -> Result<String, String> {
    tx.query_row("SELECT slug FROM tasks WHERE id = ?1", [key], |r| r.get(0)).map_err(sql)
}

/// `key`'s commit returned for a red check: the same commit is not queued again (AC-187).
pub fn not_again(tx: &Transaction, key: &str, commit: &str, at: i64) -> Result<(), String> {
    tx.execute("INSERT OR IGNORE INTO integration_returns (task, commit_sha, why, at) VALUES (?1, ?2, 'red', ?3)", rusqlite::params![key, commit, at]).map(drop).map_err(sql)
}

/// How a task went back from the queue, as `terminum integration wait` exits for it (AC-187).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Left {
    /// returned for a red check
    Red,
    /// it does not merge
    Conflict,
    /// its batch given up, or integration turned off
    Out,
}

impl Left {
    fn word(self) -> &'static str {
        match self { Left::Red => "red", Left::Conflict => "conflict", Left::Out => "out" }
    }
}

/// Back to its agent: IN PROGRESS, out of the queue, told why (§9 "returned"), marked how.
pub fn give_back(tx: &Transaction, key: &str, words: &str, how: Left, at: i64) -> Result<(), String> {
    let t = ops::get(tx, &slug(tx, key)?)?;
    if t.status == INTEGRATION {
        ops::place(tx, DISPATCHER, &t, "IN PROGRESS", at)?;
    }
    ops::left_queue(tx, DISPATCHER, &t.id, words, how.word(), at).map(drop)
}

use super::git::short;

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
        give_back(tx, &q.task, &conflict_words(q, why, s, base), Left::Conflict, at)?;
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

/// The batch's round ends `result`; a red round (its check red, over its time or not started) is final: building the
/// batch anew keeps its result, words and end time, so its checked time holds no analysis (AC-190).
pub fn round_ends(tx: &Transaction, b: &Batch, result: &str, why: Option<&str>, at: i64) -> Result<(), String> {
    tx.execute("UPDATE integration_rounds SET result = ?3, why = ?4, ended = ?5, started = coalesce(started, ?6) WHERE batch = ?1 AND round = ?2 AND result <> 'red'",
               rusqlite::params![b.id, b.round, result, why, at, b.step_started]).map(drop).map_err(sql)
}

/// The batch's integration task's ID, while it has one.
pub fn task_of(tx: &Transaction, b: &Batch) -> Result<Option<String>, String> {
    b.task.as_deref().map(|k| slug(tx, k)).transpose()
}

/// A word on the batch's integration task, while it has one.
pub fn tell_task(tx: &Transaction, b: &Batch, words: &str, at: i64) -> Result<(), String> {
    match task_of(tx, b)? {
        Some(id) => ops::comment(tx, DISPATCHER, &id, words, at).map(drop),
        None => Ok(()),
    }
}

/// The integration task waits for a human, saying why (until its agent comes, AC-194).
pub fn wait_for_human(tx: &Transaction, b: &Batch, words: &str, at: i64) -> Result<(), String> {
    let Some(id) = task_of(tx, b)? else { return Ok(()) };
    ops::questions(tx, DISPATCHER, &id, words, at)?;
    let t = ops::get(tx, &id)?;
    ops::place(tx, DISPATCHER, &t, "WAIT FOR HUMAN", at)
}

/// The integration task DONE with `words`.
pub fn close_task(tx: &Transaction, b: &Batch, words: &str, at: i64) -> Result<(), String> {
    let Some(id) = task_of(tx, b)? else { return Ok(()) };
    ops::comment(tx, DISPATCHER, &id, words, at)?;
    let t = ops::get(tx, &id)?;
    if t.status == "DONE" { Ok(()) } else { ops::place(tx, DISPATCHER, &t, "DONE", at) }
}

/// The batch given up: each task back to its agent, its integration task DONE.
pub fn give_up(tx: &Transaction, b: &Batch, why: &str, at: i64) -> Result<(), String> {
    for k in &b.tasks {
        if let Ok(id) = slug(tx, k) && tx.query_row("SELECT 1 FROM integration_entries WHERE task = ?1", [k], |_| Ok(())).optional().map_err(sql)?.is_some() {
            give_back(tx, k, &format!("{id} returned: batch {} was given up: {why}. Integrate again when you are told to, or ask.", b.number), Left::Out, at)?;
        }
    }
    set_pane(tx, b.id, None, at)?;
    set_state(tx, b.id, "given up", Some(why), at)?;
    close_task(tx, b, &format!("Batch {}: nothing published, {why}.", b.number), at)
}

/// The batch's tasks still queued in it, as the chooser takes them.
pub fn queued_of(tx: &Transaction, b: &Batch) -> Result<Vec<Queued>, String> {
    let mut out = Vec::new();
    for k in &b.tasks {
        let commit: Option<String> = tx.query_row("SELECT commit_sha FROM integration_entries WHERE task = ?1", [k], |r| r.get(0)).optional().map_err(sql)?;
        if let Some(commit) = commit {
            out.push(Queued { task: k.clone(), id: slug(tx, k)?, commit });
        }
    }
    Ok(out)
}

/// Back in the queue, for the next batch.
pub fn unbatch(tx: &Transaction, key: &str) -> Result<(), String> {
    tx.execute("UPDATE integration_entries SET batch = NULL WHERE task = ?1", [key]).map(drop).map_err(sql)
}

/// A new round of `chosen` tasks and `fixes` on `target`, in `state`.
pub fn new_round(tx: &Transaction, b: &Batch, chosen: &[Queued], fixes: &[String], target: &str, candidate: Option<&str>, state: &str) -> Result<(), String> {
    let keys = serde_json::to_string(&chosen.iter().map(|q| &q.task).collect::<Vec<_>>()).unwrap_or_default();
    let fixes = serde_json::to_string(fixes).unwrap_or_default();
    tx.execute("UPDATE integration_batches SET tasks = ?2, fixes = ?6, base_sha = ?3, candidate_sha = ?4, round = round + 1, state = ?5, pane = NULL, unchanged_rechecks = 0 WHERE id = ?1",
               rusqlite::params![b.id, keys, target, candidate, state, fixes]).map_err(sql)?;
    tx.execute("INSERT INTO integration_rounds (batch, round, tasks, fixes, base_sha, candidate_sha, result) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'built')",
               rusqlite::params![b.id, b.round + 1, keys, fixes, target, candidate]).map(drop).map_err(sql)
}

/// A red batch decided again, by its agent or a human (AC-194): built anew at the engine's next step when its tasks
/// or fixes are not its round's; else the same candidate checked again, a new round, `why` written on it.
pub fn check_again(tx: &Transaction, b: &Batch, why: Option<&str>, at: i64) -> Result<String, String> {
    if changed_since_round(tx, b)? {
        rebuild(tx, b, at)?;
        return Ok(format!("Batch {} is built anew from what is in it now and checked: round {}.", b.number, b.round + 1));
    }
    tx.execute("UPDATE integration_batches SET unchanged_rechecks = unchanged_rechecks + 1, state = 'checking', pane = NULL, round = round + 1 WHERE id = ?1", [b.id]).map_err(sql)?;
    tx.execute("INSERT INTO integration_rounds (batch, round, tasks, fixes, base_sha, candidate_sha, result, why) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'built', ?7)",
               rusqlite::params![b.id, b.round + 1, serde_json::to_string(&b.tasks).unwrap_or_default(), serde_json::to_string(&b.fixes).unwrap_or_default(),
                                 b.base, b.candidate, why.map(|w| format!("checked again unchanged: {w}"))]).map_err(sql)?;
    back_to_work(tx, b, at)?;
    Ok(format!("Batch {} is checked again: round {}.", b.number, b.round + 1))
}

/// Whether the batch's tasks or fixes are not those its round was built from: a task went back or a fix came.
pub fn changed_since_round(tx: &Transaction, b: &Batch) -> Result<bool, String> {
    let built: Option<(String, String)> = tx.query_row("SELECT tasks, fixes FROM integration_rounds WHERE batch = ?1 AND round = ?2", (b.id, b.round), |r| Ok((r.get(0)?, r.get(1)?)))
        .optional().map_err(sql)?;
    let list = |t: &str| serde_json::from_str::<Vec<String>>(t).unwrap_or_default();
    Ok(built.is_none_or(|(tasks, fixes)| b.tasks != list(&tasks) || b.fixes != list(&fixes)))
}

/// The batch built anew at the engine's next step, from what is in it then.
pub fn rebuild(tx: &Transaction, b: &Batch, at: i64) -> Result<(), String> {
    tx.execute("UPDATE integration_batches SET state = 'rebuild', pane = NULL WHERE id = ?1", [b.id]).map_err(sql)?;
    back_to_work(tx, b, at)
}

/// Its integration task back in INTEGRATION while terminumd works on its batch again: no agent is wanted for it.
pub fn back_to_work(tx: &Transaction, b: &Batch, at: i64) -> Result<(), String> {
    tx.execute("UPDATE integration_batches SET agent_wanted = 0 WHERE id = ?1", [b.id]).map_err(sql)?;
    let Some(id) = task_of(tx, b)? else { return Ok(()) };
    let t = ops::get(tx, &id)?;
    if t.status == INTEGRATION { Ok(()) } else { ops::place(tx, DISPATCHER, &t, INTEGRATION, at) }
}

/// The batch's size: doubled after a green batch up to the project's most, halved after its first red round.
pub fn resize(tx: &Transaction, project: &str, green: bool) -> Result<(), String> {
    let set = if green { "size = min(batch_most, size * 2)" } else { "size = max(1, size / 2)" };
    tx.execute(&format!("UPDATE project_integration SET {set} WHERE project = ?1"), [project]).map(drop).map_err(sql)
}

/// A red round counted; the first halves the size.
pub fn red_round(tx: &Transaction, b: &Batch) -> Result<(), String> {
    if b.red_rounds == 0 {
        resize(tx, &b.project, false)?;
    }
    tx.execute("UPDATE integration_batches SET red_rounds = red_rounds + 1 WHERE id = ?1", [b.id]).map(drop).map_err(sql)
}

/// Each task of a published batch DONE with its kept result, said where it landed; a task gone meanwhile skipped.
pub fn done_all(tx: &Transaction, b: &Batch, target: &str, candidate: &str, at: i64) -> Result<Vec<String>, String> {
    // landed, not left: its tasks' DONE takes them out of the queue without taking them out of the batch (left)
    tx.execute("UPDATE integration_entries SET batch = NULL WHERE batch = ?1", [b.id]).map_err(sql)?;
    let pairs: Vec<(&String, String)> = b.tasks.iter().filter_map(|k| slug(tx, k).ok().map(|id| (k, id))).collect();
    let ids: Vec<String> = pairs.iter().map(|(_, id)| id.clone()).collect();
    for (k, id) in pairs.iter().map(|(k, id)| (*k, id.clone())) {
        let kept: Option<(String, String)> = tx.query_row("SELECT result, author FROM integration_entries WHERE task = ?1", [k], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(sql)?;
        let Some((result, author)) = kept else { continue };
        ops::comment(tx, &author, &id, &result, at)?;
        let mates: Vec<&str> = ids.iter().map(String::as_str).filter(|i| *i != id).collect();
        let with = if mates.is_empty() { String::new() } else { format!(", with {}", and_list(&mates)) };
        ops::left_queue(tx, DISPATCHER, &id, &format!("Integrated into {target} at {} in batch {}{with}.", short(candidate), b.number), "landed", at)?;
        let t = ops::get(tx, &id)?;
        ops::place(tx, DISPATCHER, &t, "DONE", at)?;
    }
    Ok(ids)
}

/// Task `key` left the queue while in batch `batch`: the batch goes on without it, built anew (AC-187); with none
/// left, given up. A red batch only loses it: its agent or a human decides what follows, so several may go back
/// for one more round (AC-194). A batch already published or given up keeps its tasks as they are.
pub fn left(tx: &Transaction, key: &str, batch: i64, at: i64) -> Result<(), String> {
    let b = by_id(tx, batch)?;
    if !BUSY.contains(&b.state.as_str()) && !["red", "not published"].contains(&b.state.as_str()) {
        return Ok(());
    }
    let rest: Vec<&String> = b.tasks.iter().filter(|k| *k != key).collect();
    if rest.is_empty() {
        set_state(tx, b.id, "given up", Some("its last task left"), at)?;
        return close_task(tx, &b, &format!("Batch {}: nothing published, its last task left the queue.", b.number), at);
    }
    let state = if b.state == RED { RED } else { "rebuild" };
    tx.execute("UPDATE integration_batches SET tasks = ?2, state = ?3 WHERE id = ?1", rusqlite::params![b.id, serde_json::to_string(&rest).unwrap_or_default(), state]).map_err(sql)?;
    if state == RED { Ok(()) } else { back_to_work(tx, &b, at) }
}
