// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Schedules (`terminum-schedules.md`): a human's task run on a cadence on one agent of the pool, each firing a
//! new task. Only a signed-in page writes one (schedulesapi.rs); `terminum mcp` has no tool for them. A schedule
//! not paused keeps a slot of its agent (AC-161); its firings are Dispatcher's (`dispatcher/fire.rs`).

use rusqlite::{OptionalExtension, Row, Transaction};
use serde::Serialize;
use serde_json::{json, Value};

use super::agents;
use super::cron::{self, Cron};
use super::ops::{self, rows, sql};

/// The most schedules a store keeps (§9 Limits).
const MOST: i64 = 200;
/// Two runs come at least this far apart, in seconds (AC-160).
const FLOOR: i64 = 15 * 60;
/// The most minutes of a scheduled run's budget.
const BUDGET_MOST: i64 = 24 * 60;
/// How many runs the floor is checked on.
const FLOOR_RUNS: usize = 500;

/// A schedule as the board and Dispatcher see it.
#[derive(Clone, Debug, Serialize)]
pub struct Schedule {
    pub id: String,
    /// "SCH-3"
    pub key: String,
    pub project: String,
    pub title: String,
    pub body: String,
    pub severity: String,
    pub agent: String,
    pub cron: Option<String>,
    pub once_at: Option<i64>,
    pub budget_seconds: i64,
    pub paused: bool,
    pub paused_why: Option<String>,
    /// the due time of its last firing, or its last save (§9)
    pub last_due: i64,
    pub author_name: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

const SELECT: &str = "SELECT s.id, s.number, s.project, s.title, s.body, s.severity, s.agent_id, s.cron, s.once_at, s.budget_seconds, s.paused, s.paused_why,
    s.last_due, u.name, s.created_at, s.updated_at, s.deleted_at FROM schedules s JOIN users u ON u.id = s.author";

fn of_row(r: &Row) -> rusqlite::Result<Schedule> {
    Ok(Schedule {
        id: r.get(0)?, key: format!("SCH-{}", r.get::<_, i64>(1)?), project: r.get(2)?, title: r.get(3)?, body: r.get(4)?, severity: r.get(5)?,
        agent: r.get(6)?, cron: r.get(7)?, once_at: r.get(8)?, budget_seconds: r.get(9)?, paused: r.get::<_, i64>(10)? != 0, paused_why: r.get(11)?,
        last_due: r.get(12)?, author_name: r.get(13)?, created_at: r.get(14)?, updated_at: r.get(15)?, deleted_at: r.get(16)?,
    })
}

/// Its cadence: five cron fields, or one moment.
pub enum When {
    Cron(Cron),
    Once(i64),
}

impl When {
    fn of(cron: Option<&str>, once_at: Option<i64>) -> Result<When, String> {
        match (cron, once_at) {
            (Some(c), None) => Cron::parse(c).map(When::Cron),
            (None, Some(at)) => Ok(When::Once(at)),
            _ => Err("When: choose how often it runs.".into()),
        }
    }

    /// The first run after `after`.
    pub fn next_after(&self, after: i64) -> Option<i64> {
        match self {
            When::Cron(c) => c.next_after(after),
            When::Once(at) => (*at > after).then_some(*at),
        }
    }

    /// Its next `n` runs after `from`, as its window and card show them.
    pub fn upcoming(&self, from: i64, n: usize) -> Vec<i64> {
        std::iter::successors(self.next_after(from), |at| self.next_after(*at)).take(n).collect()
    }
}

impl Schedule {
    pub fn when(&self) -> Result<When, String> {
        When::of(self.cron.as_deref(), self.once_at)
    }

    /// Its cadence in words.
    pub fn words(&self) -> String {
        match (&self.cron, self.once_at) {
            (Some(c), _) => cron::words(c),
            (None, Some(at)) => format!("Once at {}", cron::when_words(at)),
            _ => String::new(),
        }
    }

    /// Whether it keeps a slot of its agent (AC-161).
    pub fn keeps_slot(&self) -> bool {
        !self.paused && self.deleted_at.is_none()
    }

    /// What is due by `now`: the last due time not after it, and how many before it were missed (AC-162).
    pub fn due(&self, now: i64) -> Option<(i64, i64)> {
        if !self.keeps_slot() {
            return None;
        }
        let when = self.when().ok()?;
        let mut due = when.next_after(self.last_due).filter(|d| *d <= now)?;
        let mut missed = 0;
        while let Some(next) = when.next_after(due).filter(|d| *d <= now) {
            (due, missed) = (next, missed + 1);
        }
        Some((due, missed))
    }
}

/// What a page sends for a schedule, trimmed and checked.
pub struct Given {
    title: String,
    body: String,
    project: String,
    severity: String,
    agent: String,
    cron: Option<String>,
    once_at: Option<i64>,
    budget_seconds: i64,
    paused: bool,
}

impl Given {
    /// The body's fields, each refused in the words of AC-160.
    pub fn of(tx: &Transaction, body: &Value, now: i64) -> Result<Given, String> {
        let text = |k: &str| body[k].as_str().unwrap_or("").to_string();
        let title = ops::check_title(&text("title"))?;
        let words = text("body");
        ops::check_text(&words)?;
        let or = |k: &str, default: &str| Some(text(k)).filter(|v| !v.is_empty()).unwrap_or_else(|| default.into());
        let severity = or("severity", "P2");
        ops::check_severity(&severity)?;
        let project = or("project", super::store::DEFAULT_PROJECT);
        ops::check_project(tx, &project)?;
        let agent = text("agent");
        if agent.is_empty() {
            return Err("Choose the agent that runs it.".into());
        }
        let minutes = body["budget_minutes"].as_i64().ok_or("A scheduled run needs a time budget: no human watches it.")?;
        if !(1..=BUDGET_MOST).contains(&minutes) {
            return Err(format!("Time budget: 1 to {BUDGET_MOST} minutes."));
        }
        let cron = body["cron"].as_str().map(|c| c.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|c| !c.is_empty());
        let once_at = if cron.is_some() { None } else { body["once_at"].as_i64() };
        let when = When::of(cron.as_deref(), once_at)?;
        check_when(&when, now)?;
        Ok(Given { title, body: words, project, severity, agent, cron, once_at, budget_seconds: minutes * 60, paused: body["paused"].as_bool().unwrap_or(false) })
    }

    /// The schedule it would be, before it is kept: what its window shows (AC-160).
    pub fn as_schedule(&self) -> Schedule {
        Schedule {
            id: String::new(), key: "SCH-?".into(), project: self.project.clone(), title: self.title.clone(), body: self.body.clone(), severity: self.severity.clone(),
            agent: self.agent.clone(), cron: self.cron.clone(), once_at: self.once_at, budget_seconds: self.budget_seconds, paused: self.paused, paused_why: None,
            last_due: 0, author_name: String::new(), created_at: 0, updated_at: 0, deleted_at: None,
        }
    }
}


/// A cadence that comes, at least 15 minutes apart (AC-160).
fn check_when(when: &When, now: i64) -> Result<(), String> {
    let Some(first) = when.next_after(now) else {
        return Err(match when { When::Once(_) => "Once: a time still to come.", When::Cron(_) => "This cadence never comes." }.into());
    };
    let mut at = first;
    for _ in 0..FLOOR_RUNS {
        let Some(next) = when.next_after(at) else { break };
        if next - at < FLOOR {
            return Err(format!("Runs come at least 15 minutes apart: this cadence has {}.", (next - at) / 60));
        }
        at = next;
    }
    Ok(())
}

/// The keys of the schedules keeping a slot of `agent`, `but` one not counted.
pub fn kept_on(tx: &Transaction, agent: &str, but: Option<&str>) -> Result<Vec<String>, String> {
    rows(tx, "SELECT 'SCH-' || number FROM schedules WHERE agent_id = ?1 AND paused = 0 AND deleted_at IS NULL AND id IS NOT ?2 ORDER BY number",
         (agent, but), |r| r.get(0))
}

/// The keys of the schedules keeping a slot, by agent.
pub fn kept_by_agent(tx: &Transaction) -> Result<std::collections::BTreeMap<String, Vec<String>>, String> {
    let mut by = std::collections::BTreeMap::<String, Vec<String>>::new();
    for (agent, key) in rows(tx, "SELECT agent_id, 'SCH-' || number FROM schedules WHERE paused = 0 AND deleted_at IS NULL ORDER BY number", [], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))? {
        by.entry(agent).or_default().push(key);
    }
    Ok(by)
}

/// A slot to keep on its agent, unless it is paused (AC-161).
fn check_slot(tx: &Transaction, given: &Given, id: Option<&str>) -> Result<(), String> {
    let a = agents::by_id(tx, &given.agent)?.ok_or("That agent is gone.")?;
    let kept = kept_on(tx, &a.id, id)?;
    if !given.paused && kept.len() as i64 >= a.at_once {
        return Err(format!("{} runs {} at once, kept for {}: raise its at once, or choose another agent.", a.name, a.at_once, kept.join(", ")));
    }
    Ok(())
}

pub fn by_id(tx: &Transaction, id: &str) -> Result<Schedule, String> {
    tx.query_row(&format!("{SELECT} WHERE s.id = ?1 AND s.deleted_at IS NULL"), [id], of_row).optional().map_err(sql)?.ok_or_else(|| "That schedule is gone.".into())
}

/// Every schedule not deleted, oldest first.
pub fn all(tx: &Transaction) -> Result<Vec<Schedule>, String> {
    rows(tx, &format!("{SELECT} WHERE s.deleted_at IS NULL ORDER BY s.number"), [], of_row)
}

/// Every schedule that may fire: not paused, not deleted.
pub fn active(tx: &Transaction) -> Result<Vec<Schedule>, String> {
    rows(tx, &format!("{SELECT} WHERE s.paused = 0 AND s.deleted_at IS NULL ORDER BY s.number"), [], of_row)
}

/// The schedule a task was made by, if one was (AC-163).
pub fn of_task(tx: &Transaction, task_key: &str) -> Result<Option<Schedule>, String> {
    tx.query_row(&format!("{SELECT} JOIN schedule_fires f ON f.schedule = s.id WHERE f.task = ?1"), [task_key], of_row).optional().map_err(sql)
}

/// The schedule each of `keys` (tasks' keys) was made by, "SCH-1", for its card (AC-163).
pub fn on_cards(tx: &Transaction, keys: &[&str]) -> Result<std::collections::HashMap<String, String>, String> {
    Ok(rows(tx, "SELECT f.task, 'SCH-' || s.number FROM schedule_fires f JOIN schedules s ON s.id = f.schedule WHERE f.task IN (SELECT value FROM json_each(?1))",
            [crate::dispatcher::runs::listed_keys(keys)], |r| Ok((r.get(0)?, r.get(1)?)))?.into_iter().collect())
}

/// Keeps a new one by `author`. Its id and key.
pub fn add(tx: &Transaction, author: &str, given: &Given, now: i64) -> Result<(String, String), String> {
    let count: i64 = tx.query_row("SELECT count(*) FROM schedules WHERE deleted_at IS NULL", [], |r| r.get(0)).map_err(sql)?;
    if count >= MOST {
        return Err(format!("A store keeps {MOST} schedules: delete one first."));
    }
    check_slot(tx, given, None)?;
    let id = crate::auth::crypto::random_hex::<8>();
    let number: i64 = tx.query_row("SELECT coalesce(max(number), 0) + 1 FROM schedules", [], |r| r.get(0)).map_err(sql)?;
    tx.execute("INSERT INTO schedules (id, number, project, title, body, severity, agent_id, cron, once_at, budget_seconds, paused, last_due, author, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?12, ?12)",
               rusqlite::params![id, number, given.project, given.title, given.body, given.severity, given.agent, given.cron, given.once_at, given.budget_seconds, given.paused as i64, now, author]).map_err(sql)?;
    Ok((id, format!("SCH-{number}")))
}

/// Changes one: the next firing takes it as it is then (AC-160); a save fires nothing it passed. Whether it was
/// paused by this save, so a task of it waiting for room is called off.
pub fn change(tx: &Transaction, author: &str, id: &str, given: &Given, now: i64) -> Result<bool, String> {
    let was = by_id(tx, id)?;
    check_slot(tx, given, Some(id))?;
    tx.execute("UPDATE schedules SET project = ?2, title = ?3, body = ?4, severity = ?5, agent_id = ?6, cron = ?7, once_at = ?8, budget_seconds = ?9, paused = ?10,
                paused_why = CASE WHEN ?10 = 0 THEN NULL ELSE paused_why END, last_due = ?11, author = ?12, updated_at = ?11 WHERE id = ?1",
               rusqlite::params![id, given.project, given.title, given.body, given.severity, given.agent, given.cron, given.once_at, given.budget_seconds, given.paused as i64, now, author]).map_err(sql)?;
    Ok(given.paused && !was.paused)
}

/// Deletes one: its tasks and firings stay under its name (AC-160).
pub fn remove(tx: &Transaction, id: &str, now: i64) -> Result<(), String> {
    by_id(tx, id)?;
    tx.execute("UPDATE schedules SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1", (id, now)).map(drop).map_err(sql)
}

/// Pauses one, saying why on its card (AC-162).
pub fn pause(tx: &Transaction, id: &str, why: &str, now: i64) -> Result<(), String> {
    tx.execute("UPDATE schedules SET paused = 1, paused_why = ?2, updated_at = ?3 WHERE id = ?1", (id, why, now)).map(drop).map_err(sql)
}

/// What the board shows of each schedule (AC-163): its next runs, its cadence in words, its last task and the
/// firings skipped since that task.
pub fn listed(tx: &Transaction, now: i64) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for s in all(tx)? {
        let next = match (s.when(), s.paused) {
            (Ok(when), false) => when.upcoming(now, 3),
            _ => Vec::new(),
        };
        let last = last_task(tx, &s)?;
        let skipped: i64 = tx.query_row("SELECT count(*) FROM schedule_fires WHERE schedule = ?1 AND state = 'skipped'
                                         AND id > coalesce((SELECT max(id) FROM schedule_fires WHERE schedule = ?1 AND task IS NOT NULL), 0)", [&s.id], |r| r.get(0)).map_err(sql)?;
        let mut v = json!(s);
        v["words"] = json!(s.words());
        v["next_words"] = json!(next.iter().map(|at| cron::when_words(*at)).collect::<Vec<_>>());   // the Mac's clock, whatever the browser's
        v["next"] = json!(next);
        v["last"] = json!(last.map(|t| json!({"task": t.id, "status": t.status})));
        v["skipped"] = json!(skipped);
        out.push(v);
    }
    Ok(out)
}

/// One firing written; its id.
pub fn fired(tx: &Transaction, s: &Schedule, (due, missed): (i64, i64), task: Option<&str>, state: &str, why: Option<&str>, now: i64) -> Result<i64, String> {
    tx.execute("INSERT INTO schedule_fires (schedule, due_at, at, task, state, missed, why) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
               rusqlite::params![s.id, due, now, task, state, missed, why]).map_err(sql)?;
    Ok(tx.last_insert_rowid())
}

/// A firing's task started, or waits for room, with what it was sent: kept for the Scheduled page, as a run's own
/// prompt goes once it is taken (AC-164).
pub fn fire_started(tx: &Transaction, fire: i64, prompt: &str) -> Result<(), String> {
    tx.execute("UPDATE schedule_fires SET state = 'started', prompt = ?2 WHERE id = ?1", rusqlite::params![fire, prompt]).map(drop).map_err(sql)
}

/// A firing's state changes.
pub fn fire_is(tx: &Transaction, fire: i64, state: &str, why: Option<&str>) -> Result<(), String> {
    tx.execute("UPDATE schedule_fires SET state = ?2, why = coalesce(?3, why) WHERE id = ?1", rusqlite::params![fire, state, why]).map(drop).map_err(sql)
}

/// Firings whose task was made and whose run was never asked for: terminumd ended between the two (AC-162).
pub fn unstarted(tx: &Transaction) -> Result<Vec<(i64, String)>, String> {
    rows(tx, "SELECT f.id, t.slug FROM schedule_fires f JOIN tasks t ON t.id = f.task WHERE f.state = 'made' ORDER BY f.id", [], |r| Ok((r.get(0)?, r.get(1)?)))
}

/// A task of `s` still open, the newest, by its ID (AC-162).
pub fn open_task(tx: &Transaction, s: &Schedule) -> Result<Option<String>, String> {
    tx.query_row("SELECT t.slug FROM schedule_fires f JOIN tasks t ON t.id = f.task WHERE f.schedule = ?1 AND t.status NOT IN ('DONE', 'FAILED') AND t.archived_at IS NULL
                  ORDER BY f.id DESC LIMIT 1", [&s.id], |r| r.get(0)).optional().map_err(sql)
}

/// The task of `s`'s last firing, if it made one, with its status.
pub fn last_task(tx: &Transaction, s: &Schedule) -> Result<Option<ops::Task>, String> {
    let key: Option<String> = tx.query_row("SELECT t.slug FROM schedule_fires f JOIN tasks t ON t.id = f.task WHERE f.schedule = ?1 ORDER BY f.id DESC LIMIT 1", [&s.id], |r| r.get(0))
        .optional().map_err(sql)?;
    key.map(|k| ops::get(tx, &k)).transpose()
}

#[cfg(test)]
#[path = "schedules_tests.rs"]
mod tests;
