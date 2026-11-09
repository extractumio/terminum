// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A RUN at a slot's limit waits for it (`terminum-dispatcher.md` AC-158): the press is kept, oldest first, and
//! pressed again once its agent and its project have room, every check of a press read anew then. A wait is no
//! run: it holds no slot, no pane, no clock.

use std::collections::HashMap;

use rusqlite::{OptionalExtension, Transaction};
use serde_json::Value;

use super::pool;
use crate::app::Shared;
use crate::tasks::agents;
use crate::tasks::ops::{self, rows, sql};
use crate::tasks::store::DISPATCHER;

/// One kept press.
#[derive(Clone, Debug, PartialEq)]
pub struct Wait {
    pub task: String,
    pub agent: String,
    pub project: String,
    /// the task's column when it began: a move since calls it off
    pub status: String,
    /// the press, as RUN's route takes it
    pub body: Value,
    pub at: i64,
}

const SELECT: &str = "SELECT w.task, w.agent_id, w.project, w.status, w.body, w.at FROM run_waits w";

fn of_row(r: &rusqlite::Row) -> rusqlite::Result<Wait> {
    let body: String = r.get(4)?;
    Ok(Wait { task: r.get(0)?, agent: r.get(1)?, project: r.get(2)?, status: r.get(3)?, body: serde_json::from_str(&body).unwrap_or(Value::Null), at: r.get(5)? })
}

/// Every wait, oldest first.
pub fn all(tx: &Transaction) -> Result<Vec<Wait>, String> {
    rows(tx, &format!("{SELECT} ORDER BY w.at, w.rowid"), [], of_row)
}

pub fn of_task(tx: &Transaction, task: &str) -> Result<Option<Wait>, String> {
    tx.query_row(&format!("{SELECT} WHERE w.task = ?1"), [task], of_row).optional().map_err(sql)
}

/// `task` waits for `agent` with `body`; one waiting already keeps its place in the line.
pub fn add(tx: &Transaction, task: &ops::Task, agent: &str, project: &str, body: &Value, at: i64) -> Result<(), String> {
    tx.execute("INSERT INTO run_waits (task, agent_id, project, status, body, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT (task) DO UPDATE SET agent_id = excluded.agent_id, project = excluded.project, status = excluded.status, body = excluded.body",
               (&task.key, agent, project, &task.status, body.to_string(), at)).map(drop).map_err(sql)
}

/// The wait of `task` ends: it started, or was called off.
pub fn remove(tx: &Transaction, task: &str) -> Result<bool, String> {
    tx.execute("DELETE FROM run_waits WHERE task = ?1", [task]).map(|n| n > 0).map_err(sql)
}

/// The wait of `task` is called off, `words` saying why on it.
pub fn call_off(tx: &Transaction, task: &str, words: &str) -> Result<(), String> {
    remove(tx, task)?;
    ops::log(tx, task, DISPATCHER, "told", words, None, ops::now()).map(drop)
}

/// An older wait of `agent` or `project` that could start now, but `task`'s own: a press for them goes behind it,
/// so the walk's beat never lets a newer press take the slot an older one waited for.
pub fn ahead(tx: &Transaction, agent: &str, project: &str, task: &str) -> Result<Option<String>, String> {
    let since = of_task(tx, task)?.map_or(i64::MAX, |w| w.at);
    let slots = pool::Slots::read(tx)?;
    for w in all(tx)?.into_iter().filter(|w| w.task != task && w.at <= since && (w.agent == agent || w.project == project)) {
        if let Some(a) = agents::by_id(tx, &w.agent)? && slots.check_for(tx, &a, &w.project, Some(&w.task), None)?.is_none() {
            return Ok(Some(ops::get(tx, &w.task)?.id));
        }
    }
    Ok(None)
}

/// The words a waiting press answers with, and its card says.
pub fn words(agent: &str, why: &str) -> String {
    format!("It waits for {agent}: {why} It starts when there is room.")
}

/// The agent each of `keys` waits for, by name, for its card.
pub fn on_cards(tx: &Transaction, keys: &[&str]) -> Result<HashMap<String, String>, String> {
    Ok(rows(tx, "SELECT w.task, COALESCE(a.name, w.agent_id) FROM run_waits w LEFT JOIN agents a ON a.id = w.agent_id WHERE w.task IN (SELECT value FROM json_each(?1))",
            [super::runs::listed_keys(keys)], |r| Ok((r.get(0)?, r.get(1)?)))?.into_iter().collect())
}

/// What a wait comes to now: pressed again, still waiting, or called off with these words; the task's ID.
#[derive(Debug)]
enum Due {
    Press(String, Value),
    Stays,
    Off(String, String),
}

fn due(tx: &Transaction, w: &Wait) -> Result<Due, String> {
    let t = ops::get(tx, &w.task)?;
    let Some(a) = agents::by_id(tx, &w.agent)? else {
        return Ok(Due::Off(t.id, "Its wait was called off: its agent is no longer in the pool. Run it again.".into()));
    };
    if t.archived_at.is_some() {
        return Ok(Due::Off(t.id, "Its wait was called off: it was archived.".into()));
    }
    if t.status != w.status {
        return Ok(Due::Off(t.id, format!("Its wait for {} was called off: it moved to {}.", a.name, t.status)));
    }
    if pool::check_for(tx, &a, &w.project, Some(&w.task), None)?.is_some() {
        return Ok(Due::Stays);
    }
    Ok(Due::Press(t.id, w.body.clone()))
}

/// Each wait in turn, oldest first: one with room is pressed again as RUN's route would press it now; what that
/// refuses is said on the task and the wait goes (AC-158). A wait blocked by its project blocks no other.
pub async fn start_due(app: &Shared) {
    let Ok(waiting) = super::store(app, all).await else { return };
    for w in waiting {
        let wanted = w.clone();
        let decided = super::store(app, move |tx| {
            let d = due(tx, &wanted)?;
            if let Due::Off(_, words) = &d {
                call_off(tx, &wanted.task, words)?;
            }
            Ok(d)
        }).await;
        let id = match decided {
            Ok(Due::Press(id, body)) => {
                if let Err(why) = super::api::press_again(app, &id, body).await {
                    let (task, words) = (w.task.clone(), format!("Did not start after its wait: {why}"));
                    let _ = super::store(app, move |tx| call_off(tx, &task, &words)).await;
                }
                id
            }
            Ok(Due::Off(id, _)) => id,
            Ok(Due::Stays) => continue,
            Err(e) => {
                tracing::warn!(event = "dispatcher.wait", error = %e);
                continue;
            }
        };
        crate::tasks::api::told(app, &id);
    }
}

#[cfg(test)]
#[path = "waits_tests.rs"]
mod tests;
