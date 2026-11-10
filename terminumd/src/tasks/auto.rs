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

/// The most candidates a round is given: the first of TODO, in the board's order.
pub const CANDIDATES_MOST: usize = 30;

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/task-projects/{id}/auto", put(set_auto)).route("/api/tasks/{id}/agent", put(set_task_agent))
}

/// A project's setting.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Setting {
    /// the agent that picks; none: off
    pub agent: Option<String>,
    /// the agent that runs a task naming none
    pub default: Option<String>,
    /// what its last round saw: not started again for the same
    pub seen: Option<String>,
    /// why it turned itself off, until a human turns it on again
    pub off_why: Option<String>,
    /// since when it is on: what it started before does not count against it
    pub on_at: i64,
}

const SETTING: &str = "SELECT auto_agent, auto_default, auto_seen, auto_off_why, auto_on_at FROM projects";

fn setting_of(r: &rusqlite::Row) -> rusqlite::Result<Setting> {
    Ok(Setting { agent: r.get(0)?, default: r.get(1)?, seen: r.get(2)?, off_why: r.get(3)?, on_at: r.get(4)? })
}

pub fn setting(tx: &Transaction, project: &str) -> Result<Setting, String> {
    tx.query_row(&format!("{SETTING} WHERE id = ?1"), [project], setting_of).map_err(sql)
}

/// The projects it is on for.
pub fn on(tx: &Transaction) -> Result<Vec<(String, Setting)>, String> {
    rows(tx, "SELECT id, auto_agent, auto_default, auto_seen, auto_off_why, auto_on_at FROM projects WHERE auto_agent IS NOT NULL ORDER BY place", [],
         |r| Ok((r.get(0)?, Setting { agent: r.get(1)?, default: r.get(2)?, seen: r.get(3)?, off_why: r.get(4)?, on_at: r.get(5)? })))
}

/// Each project's setting for the board, by its id: the picking agent, the default one, why it is off.
pub fn for_board(tx: &Transaction) -> Result<Vec<(String, Value)>, String> {
    rows(tx, "SELECT id, auto_agent, auto_default, auto_off_why FROM projects", [], |r| {
        Ok((r.get(0)?, json!({"agent": r.get::<_, Option<String>>(1)?, "default": r.get::<_, Option<String>>(2)?, "off_why": r.get::<_, Option<String>>(3)?})))
    })
}

/// An agent of the pool, or none.
fn in_pool(tx: &Transaction, agent: Option<&str>) -> Result<Option<String>, String> {
    match agent.filter(|a| !a.is_empty()) {
        Some(a) => agents::by_id(tx, a)?.map(|a| Some(a.id)).ok_or_else(|| "That agent is gone.".to_string()),
        None => Ok(None),
    }
}

/// A human's setting: `agent` picks (none: off), `default` runs a task naming no agent. Turned on, it starts anew:
/// why it was off and what it saw are forgotten.
pub fn set(tx: &Transaction, project: &str, agent: Option<&str>, default: Option<&str>, at: i64) -> Result<(), String> {
    ops::check_project(tx, project)?;
    let (agent, default) = (in_pool(tx, agent)?, in_pool(tx, default)?);
    let was = setting(tx, project)?;
    let on_at = if agent.is_some() && was.agent.is_none() { at } else { was.on_at };
    tx.execute("UPDATE projects SET auto_agent = ?2, auto_default = ?3, auto_seen = NULL, auto_off_why = NULL, auto_on_at = ?4 WHERE id = ?1", (project, agent, default, on_at)).map(drop).map_err(sql)
}

/// It turns itself off for `project`, saying why: only a human turns it on again.
pub fn turn_off(tx: &Transaction, project: &str, why: &str) -> Result<(), String> {
    tx.execute("UPDATE projects SET auto_agent = NULL, auto_seen = NULL, auto_off_why = ?2 WHERE id = ?1", (project, why)).map(drop).map_err(sql)
}

pub fn saw(tx: &Transaction, project: &str, seen: &str) -> Result<(), String> {
    tx.execute("UPDATE projects SET auto_seen = ?2 WHERE id = ?1", (project, seen)).map(drop).map_err(sql)
}

/// The agent a human chose to run task `key`, while it is in the pool.
pub fn agent_of(tx: &Transaction, key: &str) -> Result<Option<String>, String> {
    tx.query_row("SELECT a.id FROM tasks t JOIN agents a ON a.id = t.agent WHERE t.id = ?1", [key], |r| r.get(0)).optional().map_err(sql)
}

/// A human names the agent that runs task `id`, or none; its history says so.
pub fn set_agent(tx: &Transaction, id: &str, agent: Option<&str>, at: i64) -> Result<(), String> {
    let (key, agent) = (ops::key_of(tx, id)?, in_pool(tx, agent)?);
    if agent_of(tx, &key)? == agent {
        return Ok(());
    }
    tx.execute("UPDATE tasks SET agent = ?2 WHERE id = ?1", (&key, &agent)).map_err(sql)?;
    let words = match &agent {
        Some(a) => format!("Its agent: {}", agents::by_id(tx, a)?.map_or_else(|| a.clone(), |a| a.name)),
        None => "Its agent: none".to_string(),
    };
    ops::log(tx, &key, HUMAN, "why", &words, None, at).map(drop)
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

/// The rounds of `project` still on the board, oldest first: (ID, status).
pub fn rounds_shown(tx: &Transaction, project: &str) -> Result<Vec<(String, String)>, String> {
    rows(tx, "SELECT t.slug, t.status FROM auto_rounds a JOIN tasks t ON t.id = a.task WHERE a.project = ?1 AND t.archived_at IS NULL ORDER BY a.at, a.rowid", [project], |r| Ok((r.get(0)?, r.get(1)?)))
}

/// Round `round` started `task` on `agent`.
pub fn started(tx: &Transaction, task: &str, round: &str, agent: &str, at: i64) -> Result<(), String> {
    tx.execute("INSERT INTO auto_starts (task, round, agent_id, at) VALUES (?1, ?2, ?3, ?4)", (task, round, agent, at)).map(drop).map_err(sql)
}

/// How many tasks round `round` started.
pub fn started_by(tx: &Transaction, round: &str) -> Result<i64, String> {
    tx.query_row("SELECT count(*) FROM auto_starts WHERE round = ?1", [round], |r| r.get(0)).map_err(sql)
}

/// Why it must stop for `project`, if it must (its circuit breaker): its last round failed having started nothing, or
/// the last two tasks it started since `on_at` both failed. A broken picker or executor fails no more of the board.
pub fn tripped(tx: &Transaction, project: &str, on_at: i64) -> Result<Option<String>, String> {
    let last: Option<(String, String, String)> = tx.query_row("SELECT t.slug, t.status, t.id FROM auto_rounds a JOIN tasks t ON t.id = a.task WHERE a.project = ?1 AND a.at >= ?2 ORDER BY a.at DESC, a.rowid DESC LIMIT 1",
        (project, on_at), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(sql)?;
    if let Some((id, status, key)) = last && status == "FAILED" && started_by(tx, &key)? == 0 {
        return Ok(Some(format!("its round {id} failed: see it in Archived.")));
    }
    let ended: Vec<(String, String)> = rows(tx, "SELECT t.slug, t.status FROM auto_starts s JOIN tasks t ON t.id = s.task JOIN auto_rounds a ON a.task = s.round
                 WHERE a.project = ?1 AND s.at >= ?2 AND t.status NOT IN ('IN PROGRESS', 'WAIT FOR HUMAN') ORDER BY s.at DESC, s.rowid DESC LIMIT 2", (project, on_at), |r| Ok((r.get(0)?, r.get(1)?)))?;
    if ended.len() == 2 && ended.iter().all(|(_, status)| status == "FAILED") {
        return Ok(Some(format!("the last two tasks it started failed: {}, {}.", ended[1].0, ended[0].0)));
    }
    Ok(None)
}

/// What a round may start in `project`: its TODO tasks no session, run, wait, schedule or round holds, whose agent
/// (the task's own, else `default`) is in the pool; in the board's order, CANDIDATES_MOST at most. Each with that agent.
pub fn candidates(tx: &Transaction, project: &str, default: Option<&str>) -> Result<Vec<(ops::Task, String)>, String> {
    let found: Vec<(String, String)> = rows(tx, "SELECT t.slug, a.id FROM tasks t JOIN agents a ON a.id = COALESCE((SELECT x.id FROM agents x WHERE x.id = t.agent), ?2)
                 WHERE t.project = ?1 AND t.status = 'TODO' AND t.archived_at IS NULL AND t.session_key IS NULL
                   AND NOT EXISTS (SELECT 1 FROM run_waits w WHERE w.task = t.id) AND NOT EXISTS (SELECT 1 FROM auto_rounds r WHERE r.task = t.id)
                   AND NOT EXISTS (SELECT 1 FROM schedule_fires f WHERE f.task = t.id)
                 ORDER BY t.severity, t.rank LIMIT ?3", (project, default, (CANDIDATES_MOST * 2) as i64), |r| Ok((r.get(0)?, r.get(1)?)))?;
    let mut out = Vec::new();
    for (id, agent) in found {
        let t = ops::get(tx, &id)?;
        if crate::dispatcher::runs::holding(tx, &t.key)?.is_none() && out.len() < CANDIDATES_MOST {
            out.push((t, agent));
        }
    }
    Ok(out)
}

/// A human's setting for a project: {"agent": id or null, "default": id or null}.
async fn set_auto(State(app): State<Shared>, Path(project): Path<String>, Json(body): Json<Value>) -> Response {
    let done = store(&app, move |tx| set(tx, &project, body["agent"].as_str(), body["default"].as_str(), ops::now()).map(|()| json!({"ok": true}))).await;
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
