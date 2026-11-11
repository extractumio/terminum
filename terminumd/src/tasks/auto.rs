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

const SETTING: &str = "SELECT auto_agent, auto_default, auto_seen, auto_off_why, auto_on_at, id FROM projects";

fn setting_of(r: &rusqlite::Row) -> rusqlite::Result<(String, Setting)> {
    Ok((r.get(5)?, Setting { agent: r.get(0)?, default: r.get(1)?, seen: r.get(2)?, off_why: r.get(3)?, on_at: r.get(4)? }))
}

pub fn setting(tx: &Transaction, project: &str) -> Result<Setting, String> {
    tx.query_row(&format!("{SETTING} WHERE id = ?1"), [project], setting_of).map(|(_, s)| s).map_err(sql)
}

/// The projects it is on for.
pub fn on(tx: &Transaction) -> Result<Vec<(String, Setting)>, String> {
    rows(tx, &format!("{SETTING} WHERE auto_agent IS NOT NULL ORDER BY place"), [], setting_of)
}

impl Setting {
    /// What the board shows of it: the picking agent, the default one, why it is off.
    pub fn shown(&self) -> Value {
        json!({"agent": self.agent, "default": self.default, "off_why": self.off_why})
    }
}

/// Every project's setting, by its id.
pub fn all(tx: &Transaction) -> Result<Vec<(String, Setting)>, String> {
    rows(tx, SETTING, [], setting_of)
}

/// An agent of the pool, or none.
fn in_pool(tx: &Transaction, agent: Option<&str>) -> Result<Option<agents::Agent>, String> {
    match agent.filter(|a| !a.is_empty()) {
        Some(a) => agents::by_id(tx, a)?.map(Some).ok_or_else(|| "That agent is gone.".to_string()),
        None => Ok(None),
    }
}

/// A human's setting: `agent` picks (none: off), `default` runs a task naming no agent. Turned on, it starts anew:
/// why it was off and what it saw are forgotten.
pub fn set(tx: &Transaction, project: &str, agent: Option<&str>, default: Option<&str>, at: i64) -> Result<(), String> {
    ops::check_project(tx, project)?;
    let (agent, default) = (in_pool(tx, agent)?.map(|a| a.id), in_pool(tx, default)?.map(|a| a.id));
    let was = setting(tx, project)?;
    let on_at = if agent.is_some() && was.agent.is_none() { at } else { was.on_at };
    // what the last round saw stands while the same agents pick and run: a Save that changes nothing makes no round
    let seen = was.seen.filter(|_| was.agent == agent && was.default == default);
    tx.execute("UPDATE projects SET auto_agent = ?2, auto_default = ?3, auto_seen = ?5, auto_off_why = NULL, auto_on_at = ?4 WHERE id = ?1", (project, agent, default, on_at, seen)).map(drop).map_err(sql)
}

/// It turns itself off for `project`, saying why: only a human turns it on again.
pub fn turn_off(tx: &Transaction, project: &str, why: &str) -> Result<(), String> {
    tx.execute("UPDATE projects SET auto_agent = NULL, auto_off_why = ?2 WHERE id = ?1", (project, why)).map(drop).map_err(sql)
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

/// Round `round` starts `task`: written before its press, so two calls at once never both find a slot of the round.
pub fn started(tx: &Transaction, task: &str, round: &str, at: i64) -> Result<(), String> {
    tx.execute("INSERT INTO auto_starts (task, round, at) VALUES (?1, ?2, ?3)", (task, round, at)).map(drop).map_err(sql)
}

/// The press of `task` by `round` started nothing: its slot of the round is free again.
pub fn not_started(tx: &Transaction, task: &str, round: &str) -> Result<(), String> {
    tx.execute("DELETE FROM auto_starts WHERE task = ?1 AND round = ?2", (task, round)).map(drop).map_err(sql)
}

/// The last round of `project` since `on_at`, when it started nothing: when it was made and what it was given. After
/// such a round the next waits (dispatcher/auto.rs), so tasks its agent leaves do not bring it back at every change.
pub fn fruitless(tx: &Transaction, project: &str, on_at: i64) -> Result<Option<(i64, Vec<String>)>, String> {
    let last: Option<(String, i64, String)> = tx.query_row("SELECT task, at, candidates FROM auto_rounds WHERE project = ?1 AND at >= ?2 ORDER BY at DESC, rowid DESC LIMIT 1", (project, on_at),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(sql)?;
    Ok(match last {
        Some((key, at, ids)) if started_by(tx, &key)? == 0 => Some((at, serde_json::from_str(&ids).unwrap_or_default())),
        _ => None,
    })
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

/// A task a round may start.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub key: String,
    pub updated_at: i64,
    /// the agent it runs on: the task's own, else the setting's default
    pub agent: String,
}

/// What a round may start in `project`: its TODO tasks no session, run, wait, schedule or round holds, whose agent
/// (the task's own, else `default`) is in the pool: one naming an agent deleted since is none, never the default's;
/// in the board's order, CANDIDATES_MOST at most. `only`: that task alone, by its ID.
pub fn candidates(tx: &Transaction, project: &str, default: Option<&str>, only: Option<&str>) -> Result<Vec<Candidate>, String> {
    let found: Vec<Candidate> = rows(tx, "SELECT t.slug, t.id, t.updated_at, a.id FROM tasks t JOIN agents a ON a.id = COALESCE(t.agent, ?2)
                 WHERE t.project = ?1 AND t.status = 'TODO' AND t.archived_at IS NULL AND t.session_key IS NULL AND (?3 IS NULL OR t.slug = ?3)
                   AND NOT EXISTS (SELECT 1 FROM run_waits w WHERE w.task = t.id) AND NOT EXISTS (SELECT 1 FROM auto_rounds r WHERE r.task = t.id)
                   AND NOT EXISTS (SELECT 1 FROM schedule_fires f WHERE f.task = t.id)
                 ORDER BY t.severity, t.rank LIMIT ?4", (project, default, only, (CANDIDATES_MOST * 2) as i64),
        |r| Ok(Candidate { id: r.get(0)?, key: r.get(1)?, updated_at: r.get(2)?, agent: r.get(3)? }))?;
    let mut out = Vec::new();
    for c in found {
        // a run still holding it (its agent in a turn after it went back to TODO) is no free task
        if out.len() < CANDIDATES_MOST && crate::dispatcher::runs::holding(tx, &c.key)?.is_none() {
            out.push(c);
        }
    }
    Ok(out)
}

/// A human's setting for a project: {"agent": id or null, "default": id or null, "off_why": what its window showed}.
/// A window that did not show why it turned itself off does not turn it on again unseen.
async fn set_auto(State(app): State<Shared>, Path(project): Path<String>, Json(body): Json<Value>) -> Response {
    let done = store(&app, move |tx| {
        ops::check_project(tx, &project)?;
        if let Some(why) = setting(tx, &project)?.off_why.filter(|why| body["off_why"].as_str() != Some(why)) {
            return Err(format!("It turned itself off meanwhile: {why} Open its window again."));
        }
        set(tx, &project, body["agent"].as_str(), body["default"].as_str(), ops::now()).map(|()| json!({"ok": true}))
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
