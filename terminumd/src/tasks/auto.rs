// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Auto-Dispatcher as the store keeps it (`terminum-dispatcher.md` AC-170): a project's setting, the agent a human
//! chose to run a task, the rounds and what each started. Who starts a round and what it may start:
//! `dispatcher/auto.rs`. Only a human's routes write the setting and a task's agent: no tool of an agent does.

use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::put;
use axum::{Json, Router};
use rusqlite::{OptionalExtension, Transaction};
use serde_json::{json, Value};

use super::agents;
use super::api::{answer, store, told, told_projects};
use super::ops::{self, rows, sql};
use super::store::HUMAN;
use crate::app::Shared;

/// The most candidates a round is given: the first of TODO, in priority and saved order.
pub const CANDIDATES_MOST: usize = 30;

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/task-projects/{id}/auto", put(set_auto)).route("/api/tasks/{id}/agent", put(set_task_agent))
}

/// A project's setting.
#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    /// whether it starts tasks: the board's switch
    pub on: bool,
    /// the agent that picks, kept while it is off; on needs one
    pub agent: Option<String>,
    /// the agent that runs a task naming none
    pub default: Option<String>,
    /// why it turned itself off, until a human turns it on again
    pub off_why: Option<String>,
    /// since when it is on: what it started before does not count against it
    pub on_at: i64,
    /// the severities it takes
    pub severities: Vec<String>,
    /// the minutes a task stands unchanged in TODO before it is taken
    pub wait: i64,
    /// the most tasks it starts in 24 hours
    pub day_most: i64,
    /// how tasks are picked: AGENT, a round of the picking agent, or ORDER, down the board with no agent
    pub mode: String,
}

/// The two ways of picking.
pub const AGENT: &str = "agent";
pub const ORDER: &str = "order";

impl Setting {
    /// Whether terminumd itself starts the tasks, down the board, with no agent picking.
    pub fn by_order(&self) -> bool {
        self.mode == ORDER
    }
}

/// The most a day's cap may be.
pub const DAY_MOST: i64 = 1000;
/// The cap's day.
pub const DAY: i64 = 24 * 3600;
/// For how long a task a round was given and left is passed over, unless a human changes it.
pub const PASSED: i64 = DAY;

/// The longest wait, in minutes: a day.
pub const WAIT_MOST: i64 = 24 * 60;

const SETTING: &str = "SELECT auto_agent, auto_default, auto_off_why, auto_on_at, id, auto_on, auto_severities, auto_wait_minutes, auto_day_most, auto_mode FROM projects";

fn setting_of(r: &rusqlite::Row) -> rusqlite::Result<(String, Setting)> {
    let severities: String = r.get(6)?;
    Ok((r.get(4)?, Setting { agent: r.get(0)?, default: r.get(1)?, off_why: r.get(2)?, on_at: r.get(3)?, on: r.get(5)?,
        severities: serde_json::from_str(&severities).unwrap_or_default(), wait: r.get(7)?, day_most: r.get(8)?, mode: r.get(9)? }))
}

pub fn setting(tx: &Transaction, project: &str) -> Result<Setting, String> {
    tx.query_row(&format!("{SETTING} WHERE id = ?1"), [project], setting_of).map(|(_, s)| s).map_err(sql)
}

impl Setting {
    /// What the board shows of it: its switch, the picking agent, the default one, what it takes, why it is off.
    pub fn shown(&self) -> Value {
        json!({"on": self.on, "agent": self.agent, "default": self.default, "severities": self.severities, "wait_minutes": self.wait, "day_most": self.day_most, "mode": self.mode, "off_why": self.off_why})
    }

    /// This setting with what a human's `body` changes: a field it does not hold stays, so the board's switch sends
    /// {"on"} alone. "agent" and "default": an agent's id, or null for none.
    pub fn with(&self, body: &Value) -> Result<Setting, String> {
        let agent = |name: &str, was: &Option<String>| match body.get(name) {
            None => was.clone(),
            Some(v) => v.as_str().filter(|a| !a.is_empty()).map(str::to_string),
        };
        let mut s = Setting { agent: agent("agent", &self.agent), default: agent("default", &self.default), ..self.clone() };
        if let Some(on) = body.get("on") {
            s.on = on.as_bool().ok_or("on: true or false")?;
        }
        if let Some(list) = body.get("severities") {
            let named = list.as_array().ok_or("severities: a list")?.iter().map(|v| v.as_str().ok_or("severities: a list of names")).collect::<Result<Vec<_>, _>>()?;
            named.iter().try_for_each(|n| ops::check_severity(n))?;
            s.severities = ops::SEVERITIES.iter().filter(|p| named.contains(p)).map(|p| p.to_string()).collect();
        }
        if let Some(wait) = body.get("wait_minutes") {
            s.wait = wait.as_i64().filter(|m| (0..=WAIT_MOST).contains(m)).ok_or(format!("Wait: 0 to {WAIT_MOST} minutes"))?;
        }
        if let Some(mode) = body.get("mode") {
            s.mode = mode.as_str().filter(|m| [AGENT, ORDER].contains(m)).ok_or(format!("mode: {AGENT} or {ORDER}"))?.to_string();
        }
        if let Some(most) = body.get("day_most") {
            s.day_most = most.as_i64().filter(|n| (1..=DAY_MOST).contains(n)).ok_or(format!("Tasks a day: 1 to {DAY_MOST}"))?;
        }
        Ok(s)
    }
}

/// How many tasks it started in each project in the 24 hours before `now`: what its cap counts. A press that started
/// nothing left no row; a project with none is not listed.
pub fn started_in_day_all(tx: &Transaction, now: i64) -> Result<std::collections::HashMap<String, i64>, String> {
    Ok(rows(tx, "SELECT project, count(*) FROM auto_starts WHERE at > ?1 GROUP BY project", [now - DAY / crate::dispatcher::scale()], |r| Ok((r.get(0)?, r.get(1)?)))?.into_iter().collect())
}

/// The same for `project` alone.
pub fn started_in_day(tx: &Transaction, project: &str, now: i64) -> Result<i64, String> {
    Ok(started_in_day_all(tx, now)?.remove(project).unwrap_or(0))
}

/// Every project's setting, by its id.
pub fn all(tx: &Transaction) -> Result<Vec<(String, Setting)>, String> {
    rows(tx, SETTING, [], setting_of)
}

/// The projects a walk looks at: those it is on for, and those with a round still on the board, which leaves it when
/// it ended though the setting is off. The one whose last round is oldest first: projects that share agents take
/// the free slots in turn.
pub fn due_for(tx: &Transaction) -> Result<Vec<(String, Setting)>, String> {
    rows(tx, &format!("{SETTING} WHERE auto_on = 1 OR EXISTS (SELECT 1 FROM auto_rounds r JOIN tasks t ON t.id = r.task WHERE r.project = projects.id AND t.archived_at IS NULL)
        ORDER BY COALESCE((SELECT max(r.at) FROM auto_rounds r WHERE r.project = projects.id), 0), place"), [], setting_of)
}

/// An agent of the pool, or none.
fn in_pool(tx: &Transaction, agent: Option<&str>) -> Result<Option<agents::Agent>, String> {
    match agent.filter(|a| !a.is_empty()) {
        Some(a) => agents::by_id(tx, a)?.map(Some).ok_or_else(|| "That agent is gone.".to_string()),
        None => Ok(None),
    }
}

/// A human's setting, kept as `new` says: on needs the project's folder (AC-140), an agent that picks and a severity
/// to take. Turned on, it starts anew: why it was off is forgotten, and what it started before does not count against
/// it; kept off, why stays.
pub fn set(tx: &Transaction, project: &str, new: &Setting, at: i64) -> Result<(), String> {
    ops::check_project(tx, project)?;
    if new.on {
        let (name, _, dir) = ops::project_place(tx, project)?;
        ops::no_folder(&name, &dir)?;
    }
    let (agent, default) = (in_pool(tx, new.agent.as_deref())?.map(|a| a.id), in_pool(tx, new.default.as_deref())?.map(|a| a.id));
    if new.on && agent.is_none() && !new.by_order() {
        return Err("Choose the agent that picks first: its settings.".to_string());
    }
    if new.on && new.severities.is_empty() {
        return Err("Tick a severity it takes first: with none it starts nothing.".to_string());
    }
    let was = setting(tx, project)?;
    let on_at = if new.on && !was.on { at } else { was.on_at };
    tx.execute("UPDATE projects SET auto_on = ?2, auto_agent = ?3, auto_default = ?4, auto_off_why = CASE WHEN ?2 THEN NULL ELSE auto_off_why END, auto_on_at = ?5, auto_severities = ?6, auto_wait_minutes = ?7, auto_day_most = ?8, auto_mode = ?9 WHERE id = ?1",
        (project, new.on, agent, default, on_at, json!(new.severities).to_string(), new.wait, new.day_most, &new.mode)).map(drop).map_err(sql)
}

/// It turns itself off for `project`, saying why: only a human turns it on again. Its agents stay chosen.
pub fn turn_off(tx: &Transaction, project: &str, why: &str) -> Result<(), String> {
    tx.execute("UPDATE projects SET auto_on = 0, auto_off_why = ?2 WHERE id = ?1", (project, why)).map(drop).map_err(sql)
}

/// Whether a round of another project than `project` is open: one round at a time over all projects, so two never
/// count on the same free slot of an agent they share.
pub fn open_elsewhere(tx: &Transaction, project: &str) -> Result<bool, String> {
    tx.query_row("SELECT EXISTS (SELECT 1 FROM auto_rounds r JOIN tasks t ON t.id = r.task WHERE r.project <> ?1 AND t.archived_at IS NULL AND t.status NOT IN ('DONE', 'FAILED'))", [project], |r| r.get(0)).map_err(sql)
}

/// The agent a human chose to run task `key`, while it is in the pool.
pub fn agent_of(tx: &Transaction, key: &str) -> Result<Option<String>, String> {
    tx.query_row("SELECT a.id FROM tasks t JOIN agents a ON a.id = t.agent WHERE t.id = ?1", [key], |r| r.get(0)).optional().map_err(sql)
}

/// A human names the agent that runs task `id`, or none; its history says so.
pub fn set_agent(tx: &Transaction, id: &str, agent: Option<&str>, at: i64) -> Result<(), String> {
    let (key, agent) = (ops::key_of(tx, id)?, in_pool(tx, agent)?);
    if agent_of(tx, &key)? == agent.as_ref().map(|a| a.id.clone()) {
        return Ok(());
    }
    tx.execute("UPDATE tasks SET agent = ?2 WHERE id = ?1", (&key, agent.as_ref().map(|a| &a.id))).map_err(sql)?;
    ops::log(tx, &key, HUMAN, "why", &format!("Its agent: {}", agent.map_or_else(|| "none".to_string(), |a| a.name)), None, at).map(drop)
}

/// A round as it was given.
#[derive(Clone, Debug, PartialEq)]
pub struct Round {
    /// its task's key
    pub task: String,
    pub project: String,
    /// the free slots it may fill
    pub slots: i64,
    /// the tasks it may start, by ID
    pub candidates: Vec<String>,
}

/// The round task `key` is, if it is one.
pub fn round_of(tx: &Transaction, key: &str) -> Result<Option<Round>, String> {
    tx.query_row("SELECT task, project, slots, candidates FROM auto_rounds WHERE task = ?1", [key], |r| {
        let ids: String = r.get(3)?;
        Ok(Round { task: r.get(0)?, project: r.get(1)?, slots: r.get(2)?, candidates: serde_json::from_str(&ids).unwrap_or_default() })
    }).optional().map_err(sql)
}

pub fn add_round(tx: &Transaction, round: &Round, at: i64) -> Result<(), String> {
    tx.execute("INSERT INTO auto_rounds (task, project, slots, candidates, at) VALUES (?1, ?2, ?3, ?4, ?5)", (&round.task, &round.project, round.slots, json!(round.candidates).to_string(), at)).map(drop).map_err(sql)
}

/// Task `key` is no round any more: one terminumd made and never pressed (dispatcher/auto.rs).
pub fn drop_round(tx: &Transaction, key: &str) -> Result<(), String> {
    tx.execute("DELETE FROM auto_rounds WHERE task = ?1", [key]).map(drop).map_err(sql)
}

/// The round whose run is in pane `pane`, with its task: the session Dispatcher started for it, while it stands.
pub fn round_in_pane(tx: &Transaction, pane: &str) -> Result<Option<(Round, ops::Task)>, String> {
    let Some(run) = crate::dispatcher::runs::in_pane(tx, pane)?.filter(|r| r.ids.len() == 1) else { return Ok(None) };
    let own = ops::get(tx, &run.ids[0])?;
    Ok(round_of(tx, &own.key)?.map(|round| (round, own)))
}

/// The rounds of `project` still on the board, oldest first: (key, ID, status).
pub fn rounds_shown(tx: &Transaction, project: &str) -> Result<Vec<(String, String, String)>, String> {
    rows(tx, "SELECT t.id, t.slug, t.status FROM auto_rounds a JOIN tasks t ON t.id = a.task WHERE a.project = ?1 AND t.archived_at IS NULL ORDER BY a.at, a.rowid", [project], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
}

/// It starts `task` in `project`, by round `round` or, with none, by priority and saved order: written before its press, so
/// two calls at once never both find a slot of the round, and its cap counts it.
pub fn started(tx: &Transaction, task: &str, round: Option<&str>, project: &str, at: i64) -> Result<(), String> {
    tx.execute("INSERT INTO auto_starts (task, round, project, at) VALUES (?1, ?2, ?3, ?4)", (task, round, project, at)).map(drop).map_err(sql)
}

/// That press of `task` started nothing: its slot of the round and of the day is free again.
pub fn not_started(tx: &Transaction, task: &str, round: Option<&str>) -> Result<(), String> {
    tx.execute("DELETE FROM auto_starts WHERE rowid = (SELECT max(rowid) FROM auto_starts WHERE task = ?1 AND round IS ?2)", (task, round)).map(drop).map_err(sql)
}

/// Gives back the starts of `project` whose press never came: terminumd stopped between taking the slot and pressing.
/// Such a task stands in TODO with no run made since (a run is walked from its making on); left, it would count
/// against the day and never start again. One whose run was made keeps its start, whatever its agent did: it is not
/// started twice. Called when no round is open and no press of this walk is under way.
pub fn give_back_unpressed(tx: &Transaction, project: &str) -> Result<(), String> {
    tx.execute("DELETE FROM auto_starts WHERE project = ?1 AND task IN (SELECT id FROM tasks WHERE status = 'TODO')
                  AND NOT EXISTS (SELECT 1 FROM run_tasks m JOIN runs r ON r.id = m.run WHERE m.task = auto_starts.task AND r.last_walk >= auto_starts.at)", [project]).map(drop).map_err(sql)
}

/// When the last round of `project` since `on_at` was made, if it started nothing: the next rests a while
/// (dispatcher/auto.rs), so a long list its agent leaves is not read through at once.
pub fn fruitless(tx: &Transaction, project: &str, on_at: i64) -> Result<Option<i64>, String> {
    let last: Option<(String, i64)> = tx.query_row("SELECT task, at FROM auto_rounds WHERE project = ?1 AND at >= ?2 ORDER BY at DESC, rowid DESC LIMIT 1", (project, on_at), |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(sql)?;
    Ok(match last {
        Some((key, at)) if started_by(tx, &key)? == 0 => Some(at),
        _ => None,
    })
}

/// How many tasks round `round` started.
pub fn started_by(tx: &Transaction, round: &str) -> Result<i64, String> {
    tx.query_row("SELECT count(*) FROM auto_starts WHERE round = ?1", [round], |r| r.get(0)).map_err(sql)
}

/// Why it must stop for `project`, if it must (its circuit breaker): its last round failed having started nothing, or
/// the last two tasks it started since `on_at` both failed. A broken picker or executor fails no more of the board.
/// A task a human moved or gave up on since is none of its agent's failures: it does not count.
pub fn tripped(tx: &Transaction, project: &str, on_at: i64) -> Result<Option<String>, String> {
    let last: Option<(String, String, String)> = tx.query_row("SELECT t.slug, t.status, t.id FROM auto_rounds a JOIN tasks t ON t.id = a.task WHERE a.project = ?1 AND a.at >= ?2 ORDER BY a.at DESC, a.rowid DESC LIMIT 1",
        (project, on_at), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(sql)?;
    if let Some((id, status, key)) = last && status == "FAILED" && started_by(tx, &key)? == 0 {
        return Ok(Some(format!("its round {id} failed: see it in Archived.")));
    }
    let ended: Vec<(String, String)> = rows(tx, "SELECT t.slug, t.status FROM auto_starts s JOIN tasks t ON t.id = s.task
                 WHERE s.project = ?1 AND s.at >= ?2 AND t.status NOT IN ('IN PROGRESS', 'WAIT FOR HUMAN')
                   AND NOT EXISTS (SELECT 1 FROM events e JOIN users u ON u.id = e.author WHERE e.task = t.id AND e.at >= s.at
                     AND ((e.kind = 'move' AND u.kind = 'human') OR (e.kind = 'why' AND e.text = ?3)))
                 ORDER BY s.at DESC, s.rowid DESC LIMIT 2", (project, on_at, crate::dispatcher::runs::GAVE_UP), |r| Ok((r.get(0)?, r.get(1)?)))?;
    if ended.len() == 2 && ended.iter().all(|(_, status)| status == "FAILED") {
        return Ok(Some(format!("the last two tasks it started failed: {}, {}.", ended[1].0, ended[0].0)));
    }
    Ok(None)
}

/// A task a round may start.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub key: String,
    /// the agent it runs on: the task's own, else the setting's default
    pub agent: String,
    /// when it was last made, moved or edited
    pub changed: i64,
}


/// What a round may start in `project` at `now`, by setting `s`: its TODO tasks
/// - of the severities it takes;
/// - that stand for its wait: made, moved or edited no later than that ago (a comment, a new place among the cards or
///   a subtask does not start the wait anew), so one still being written is not taken;
/// - that no session, run, wait, schedule or round holds;
/// - that are no subtask of a main task not DONE: it goes with its main task (AC-185);
/// - whose subtasks still to do all stand in TODO, free as it is: they go with it, and its wait counts from the last
///   of them made, moved or edited; a failed or waiting one is the human's to run or move first;
/// - whose agent (the task's own, else the setting's default) is in the pool: one naming an agent deleted since is
///   none, never the default's;
/// - that it did not start before, unless a human moved or edited it since: one its agent put back is not run again;
/// - that no round which ended DONE with a slot unfilled was given in the last day, since it is on, unless a human
///   moved or edited it since: what a round judged and left does not hold up the tasks below it, and an agent's own
///   edit does not bring it back; one that filled its slots left the rest for want of room, and they are offered again.
///
/// In priority and saved order; CANDIDATES_MOST at most are looked at for an agent with room. `only`: that task alone, by its ID.
pub fn candidates(tx: &Transaction, project: &str, s: &Setting, now: i64, only: Option<&str>) -> Result<Vec<Candidate>, String> {
    let scale = crate::dispatcher::scale();
    let found: Vec<Candidate> = rows(tx, "WITH passed AS MATERIALIZED (SELECT g.value AS slug, max(r.at) AS at FROM auto_rounds r JOIN tasks rt ON rt.id = r.task, json_each(r.candidates) g
                   WHERE r.project = ?1 AND r.at >= ?7 AND rt.status = 'DONE' AND (SELECT count(*) FROM auto_starts f WHERE f.round = r.task) < r.slots GROUP BY g.value),
                 c AS (SELECT t.slug, t.id, t.severity, t.rank, a.id AS agent,
                   COALESCE((SELECT max(e.at) FROM events e WHERE (e.task = t.id OR e.task IN (SELECT k.id FROM tasks k WHERE k.parent = t.id AND k.archived_at IS NULL AND k.status <> 'DONE'))
                     AND e.kind IN ('made', 'move', 'edit')), 0) AS changed,
                   COALESCE((SELECT max(e.at) FROM events e JOIN users u ON u.id = e.author WHERE e.task = t.id AND u.kind = 'human' AND e.kind IN ('move', 'edit')), 0) AS by_human
                 FROM tasks t JOIN agents a ON a.id = COALESCE(t.agent, ?2)
                 WHERE t.project = ?1 AND t.status = 'TODO' AND t.archived_at IS NULL AND t.session_key IS NULL AND (?3 IS NULL OR t.slug = ?3)
                   AND t.severity IN (SELECT value FROM json_each(?5))
                   AND NOT EXISTS (SELECT 1 FROM run_waits w WHERE w.task = t.id) AND NOT EXISTS (SELECT 1 FROM auto_rounds r WHERE r.task = t.id)
                   AND NOT EXISTS (SELECT 1 FROM schedule_fires f WHERE f.task = t.id)
                   AND NOT EXISTS (SELECT 1 FROM tasks p WHERE p.id = t.parent AND p.archived_at IS NULL AND p.status <> 'DONE')
                   AND NOT EXISTS (SELECT 1 FROM tasks k WHERE k.parent = t.id AND k.archived_at IS NULL AND k.status <> 'DONE'
                     AND (k.status <> 'TODO' OR k.session_key IS NOT NULL OR EXISTS (SELECT 1 FROM run_waits w WHERE w.task = k.id))))
                 SELECT c.slug, c.id, c.agent, c.changed FROM c WHERE c.changed <= ?6
                   AND NOT EXISTS (SELECT 1 FROM auto_starts s WHERE s.task = c.id AND s.at > c.by_human)
                   AND NOT EXISTS (SELECT 1 FROM passed p WHERE p.slug = c.slug AND p.at >= c.by_human)
                 ORDER BY c.severity, c.rank LIMIT ?4",
        (project, &s.default, only, (CANDIDATES_MOST * 2) as i64, json!(s.severities).to_string(), now - s.wait * 60 / scale, if s.by_order() { i64::MAX } else { (now - PASSED / scale + 1).max(s.on_at) }),
        |r| Ok(Candidate { id: r.get(0)?, key: r.get(1)?, agent: r.get(2)?, changed: r.get(3)? }))?;
    let mut out = Vec::new();
    for c in found {
        // a run still holding it (its agent in a turn after it went back to TODO) is no free task
        if out.len() >= CANDIDATES_MOST || crate::dispatcher::runs::holding(tx, &c.key)?.is_some() {
            continue;
        }
        let mut free = true;
        for sub in super::family::to_do(tx, &c.key)? {
            free &= crate::dispatcher::runs::holding(tx, &sub.key)?.is_none();
        }
        if free {
            out.push(c);
        }
    }
    Ok(out)
}

/// A human's setting for a project: any of {"on", "agent": id or null, "default": id or null, "severities": ["P0"…],
/// "wait_minutes", "day_most"}, and "off_why": what the page showed. A page that did not show why it turned itself off does not
/// turn it on again unseen.
async fn set_auto(State(app): State<Shared>, Path(project): Path<String>, Json(body): Json<Value>) -> Response {
    let done = store(&app, move |tx| {
        ops::check_project(tx, &project)?;
        let was = setting(tx, &project)?;
        if let Some(why) = was.off_why.as_ref().filter(|why| body["off_why"].as_str() != Some(why)) {
            return Err(format!("It turned itself off meanwhile: {why} Read why, then turn it on again."));
        }
        set(tx, &project, &was.with(&body)?, ops::now()).map(|()| json!({"ok": true}))
    }).await;
    if done.is_ok() {
        app.dispatcher.soon();
    }
    told_projects(&app, done)
}

/// A human's choice of the agent that runs a task: {"agent": id or null}.
async fn set_task_agent(State(app): State<Shared>, Path(id): Path<String>, Json(body): Json<Value>) -> Response {
    let wanted = id.clone();
    let done = store(&app, move |tx| set_agent(tx, &wanted, body["agent"].as_str(), ops::now()).map(|()| json!({"ok": true}))).await;
    if done.is_ok() {
        told(&app, &id);
    }
    answer(done)
}

#[cfg(test)]
#[path = "auto_tests.rs"]
mod tests;
