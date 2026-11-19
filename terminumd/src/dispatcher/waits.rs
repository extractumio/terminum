// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A RUN at a slot's limit waits for it (`terminum-dispatcher.md` AC-158): the press is kept, oldest first, and
//! pressed again once its agent and its project have room, every check of a press read anew then. A wait is no
//! run: it holds no slot, no pane, no clock. The waits for one agent are its queue, in an order a human can change
//! (AC-169), a schedule's task's ahead of every other (terminum-schedules.md AC-161).

use std::collections::{BTreeMap, HashMap};

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
    /// its place in the line, lowest first, among the waits of its kind
    pub pos: i64,
    /// its task is a schedule's: it stands ahead of every wait that is not (terminum-schedules.md AC-161)
    pub scheduled: bool,
}

/// Whether the task of wait `w` is a schedule's.
macro_rules! scheduled {
    () => { "EXISTS (SELECT 1 FROM schedule_fires f WHERE f.task = w.task)" };
}
const SCHEDULED: &str = scheduled!();

/// The line's order: schedules' tasks first, then the place each was given; every reader of the line sorts by it.
pub(crate) const ORDER: &str = concat!("NOT ", scheduled!(), ", w.pos, w.rowid");

fn select() -> String {
    format!("SELECT w.task, w.agent_id, w.project, w.status, w.body, w.at, w.pos, {SCHEDULED} FROM run_waits w")
}

fn of_row(r: &rusqlite::Row) -> rusqlite::Result<Wait> {
    let body: String = r.get(4)?;
    Ok(Wait { task: r.get(0)?, agent: r.get(1)?, project: r.get(2)?, status: r.get(3)?, body: serde_json::from_str(&body).unwrap_or(Value::Null), at: r.get(5)?, pos: r.get(6)?, scheduled: r.get(7)? })
}

/// Every wait, in the line's order.
pub fn all(tx: &Transaction) -> Result<Vec<Wait>, String> {
    rows(tx, &format!("{} ORDER BY {ORDER}", select()), [], of_row)
}

/// The waits of `agent`, in the line's order.
pub fn of_agent(tx: &Transaction, agent: &str) -> Result<Vec<Wait>, String> {
    rows(tx, &format!("{} WHERE w.agent_id = ?1 ORDER BY {ORDER}", select()), [agent], of_row)
}

pub fn of_task(tx: &Transaction, task: &str) -> Result<Option<Wait>, String> {
    tx.query_row(&format!("{} WHERE w.task = ?1", select()), [task], of_row).optional().map_err(sql)
}

/// The waits of tasks of schedule `schedule`, in the line's order: its firings and Run now keep at most one, a
/// human's RUN of an older task may add one (AC-162).
pub fn of_schedule(tx: &Transaction, schedule: &str) -> Result<Vec<Wait>, String> {
    rows(tx, &format!("{} JOIN schedule_fires f2 ON f2.task = w.task WHERE f2.schedule = ?1 ORDER BY {ORDER}", select()), [schedule], of_row)
}

impl Wait {
    /// A human pressed it: no manager, no Dispatcher (terminum-dispatcher.md AC-183); a newer run of its schedule never
    /// supersedes it (terminum-schedules.md AC-162).
    pub fn by_human(&self) -> bool {
        self.body["manager"].is_null() && !super::runs::How::of_word(self.body["how"].as_str().unwrap_or("run")).is_some_and(super::runs::How::automatic)
    }

    /// Whether it stands before a press of a task of that kind (`scheduled`) at `place` in the line.
    fn before(&self, scheduled: bool, place: i64) -> bool {
        (!self.scheduled, self.pos) < (!scheduled, place)
    }
}

/// `task` waits for `agent` with `body`; one waiting already keeps its place in the line.
pub fn add(tx: &Transaction, task: &ops::Task, agent: &str, project: &str, body: &Value, at: i64) -> Result<(), String> {
    tx.execute("INSERT INTO run_waits (task, agent_id, project, status, body, at, pos) VALUES (?1, ?2, ?3, ?4, ?5, ?6, (SELECT COALESCE(MAX(pos), 0) + 1 FROM run_waits))
                ON CONFLICT (task) DO UPDATE SET agent_id = excluded.agent_id, project = excluded.project, status = excluded.status, body = excluded.body",
               (&task.key, agent, project, &task.status, body.to_string(), at)).map(drop).map_err(sql)
}

/// The wait of `task` takes place `pos` in the line: a newer run of a schedule takes its superseded wait's (AC-162).
pub fn take_place(tx: &Transaction, task: &str, pos: i64) -> Result<(), String> {
    tx.execute("UPDATE run_waits SET pos = ?2 WHERE task = ?1", (task, pos)).map(drop).map_err(sql)
}

/// The wait of `task` ends: it started, or was called off.
pub fn remove(tx: &Transaction, task: &str) -> Result<bool, String> {
    tx.execute("DELETE FROM run_waits WHERE task = ?1", [task]).map(|n| n > 0).map_err(sql)
}

/// The wait of `task` is called off, `words` saying why on it; one gone already was said by what called it off.
pub fn call_off(tx: &Transaction, task: &str, words: &str) -> Result<(), String> {
    if !remove(tx, task)? {
        return Ok(());
    }
    ops::log(tx, task, DISPATCHER, "told", words, None, ops::now())?;
    super::fire::wait_ended(tx, task, words)
}

/// A wait of `agent` or `project` before `task`'s in the line that could start now, but `task`'s own: a press for
/// them goes behind it, so the walk's beat never lets a newer press take the slot an older one waited for, nor any
/// press a schedule's task's (terminum-schedules.md AC-161).
pub fn ahead(tx: &Transaction, agent: &str, project: &str, task: &str) -> Result<Option<String>, String> {
    let (scheduled, place) = match of_task(tx, task)? {
        Some(w) => (w.scheduled, w.pos),
        None => (tx.query_row("SELECT EXISTS (SELECT 1 FROM schedule_fires WHERE task = ?1)", [task], |r| r.get(0)).map_err(sql)?, i64::MAX),
    };
    let slots = pool::Slots::read(tx)?;
    for w in all(tx)?.into_iter().filter(|w| w.task != task && w.before(scheduled, place) && (w.agent == agent || w.project == project)) {
        if let Some(a) = agents::in_project(tx, &w.agent, &w.project)? && slots.check_for(tx, &a, &w.project, Some(&w.task), None)?.is_none() {
            return Ok(Some(ops::get(tx, &w.task)?.id));
        }
    }
    Ok(None)
}

/// Each agent's queue for the board (AC-169): its waiting presses in the line's order, each with its task's title and
/// project, and the other tasks of a press for several (`also`).
pub fn queues(tx: &Transaction) -> Result<BTreeMap<String, Vec<Value>>, String> {
    let found: Vec<(String, Value)> = rows(tx, &format!("SELECT w.agent_id, t.slug, t.shown, w.project, COALESCE(json_extract(w.body, '$.also'), '[]') FROM run_waits w JOIN tasks t ON t.id = w.task ORDER BY {ORDER}"), [],
        |r| Ok((r.get(0)?, serde_json::json!({"id": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?, "project": r.get::<_, String>(3)?,
                                               "also": serde_json::from_str::<Value>(&r.get::<_, String>(4)?).unwrap_or_default()}))))?;
    let mut out: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (agent, task) in found {
        out.entry(agent).or_default().push(task);
    }
    Ok(out)
}

/// The queue of `agent` in the order `ids` (tasks' IDs), the human's: the places its waits held, dealt anew, so no
/// other agent's wait moves. A queue that is not these tasks changed meanwhile: nothing moves; nor does one that puts
/// a wait ahead of a schedule's task (terminum-schedules.md AC-161).
pub fn reorder(tx: &Transaction, agent: &str, ids: &[String]) -> Result<(), String> {
    let mut mine: Vec<Wait> = of_agent(tx, agent)?;
    mine.sort_by_key(|w| w.pos);
    let keys: Vec<String> = ids.iter().map(|id| ops::key_of(tx, id)).collect::<Result<_, _>>()?;
    let (mut had, mut wanted): (Vec<&str>, Vec<&str>) = (mine.iter().map(|w| w.task.as_str()).collect(), keys.iter().map(String::as_str).collect());
    had.sort_unstable();
    wanted.sort_unstable();
    if had != wanted {
        return Err("The queue changed meanwhile: look at it again.".to_string());
    }
    let scheduled = |key: &str| mine.iter().any(|w| w.task == key && w.scheduled);
    if let Some(at) = keys.iter().position(|k| !scheduled(k)) && let Some(late) = keys[at..].iter().find(|k| scheduled(k)) {
        return Err(format!("{} is a scheduled run: it stays ahead of the other waits.", ops::get(tx, late)?.id));
    }
    // each kind deals its own places, so a wait never takes the place of another kind and no other agent's moves
    for kind in [true, false] {
        let places = mine.iter().filter(|w| w.scheduled == kind).map(|w| w.pos);
        for (key, pos) in keys.iter().filter(|k| scheduled(k) == kind).zip(places) {
            tx.execute("UPDATE run_waits SET pos = ?1 WHERE task = ?2", (pos, key)).map_err(sql)?;
        }
    }
    Ok(())
}

/// The human takes `id` out of its queue (AC-169): its wait alone. One that started meanwhile is said, and its run
/// is not touched.
pub fn take_out(tx: &Transaction, id: &str) -> Result<(), String> {
    let key = ops::key_of(tx, id)?;
    let Some(w) = of_task(tx, &key)? else { return Err(format!("{id} waits no more: it started, or its wait was called off.")) };
    let name = agents::name_of(tx, &w.agent)?;
    call_off(tx, &key, &format!("The human called off its wait for {name}."))
}

/// The words a press at a limit is offered the queue with: nothing waits until the human agrees (AC-169).
pub fn offer(why: &str) -> String {
    format!("{why} Add it to the queue: it starts when there is room.")
}

/// The words a waiting press answers with, and its card says.
pub fn words(agent: &str, why: &str) -> String {
    format!("It waits for {agent}: {why} It starts when there is room.")
}

/// Each queued press and every task it carries (AC-169), shared by cards and human-attention counts.
const MEMBERS: &str = "WITH chosen AS (SELECT w.task press, w.task FROM run_waits w
        UNION SELECT w.task, t.id FROM run_waits w, json_each(w.body, '$.also') j JOIN tasks t ON t.slug = j.value)
    SELECT press, task FROM chosen
    UNION SELECT c.press, t.id FROM chosen c JOIN tasks t ON t.parent = c.task
        WHERE t.status <> 'DONE' AND t.archived_at IS NULL
            AND NOT EXISTS (SELECT 1 FROM schedule_fires f JOIN schedules s ON s.id = f.schedule WHERE f.task = c.task)";

/// Whether task alias `t` belongs to a queued press, including its other tasks (AC-169).
/// Aliases are source literals, never a request's words.
pub fn queued_task(t: &str) -> String {
    format!("{t}.id IN (SELECT task FROM ({MEMBERS}))")
}

/// A card's wait: its agent, place in that agent's whole queue and the primary task that can cancel the press.
pub struct CardWait {
    pub agent: String,
    pub position: i64,
    pub task: String,
}

impl CardWait {
    pub fn mark(&self, card: &mut Value) {
        card["waits"] = serde_json::json!(self.agent);
        card["wait_position"] = serde_json::json!(self.position);
        card["wait_task"] = serde_json::json!(self.task);
    }
}

/// Queue positions are ranked before card filters and pagination, shared by every task of one press (AC-169).
pub fn on_cards(tx: &Transaction, keys: &[&str]) -> Result<HashMap<String, CardWait>, String> {
    if keys.is_empty() { return Ok(HashMap::new()); }
    Ok(rows(tx, &format!("WITH queue AS (
            SELECT w.task, t.slug, COALESCE(a.name, w.agent_id) agent,
                row_number() OVER (PARTITION BY w.agent_id ORDER BY {ORDER}) position
            FROM run_waits w JOIN tasks t ON t.id = w.task LEFT JOIN agents a ON a.id = w.agent_id), members AS ({MEMBERS})
        SELECT m.task, q.agent, q.position, q.slug FROM members m JOIN queue q ON q.task = m.press
        WHERE m.task IN (SELECT value FROM json_each(?1))"),
        [super::runs::listed_keys(keys)], |r| Ok((r.get(0)?, CardWait { agent: r.get(1)?, position: r.get(2)?, task: r.get(3)? })))?.into_iter().collect())
}

/// What a wait comes to now: pressed again, still waiting, or called off with these words; the task's ID. Or, a
/// schedule's task with no room, its schedule's idle session whose close makes room: the run, the schedule's key.
#[derive(Debug)]
enum Due {
    Press(String, Value),
    Stays,
    Off(String, String),
    Close(String, String),
}

fn due(tx: &Transaction, w: &Wait) -> Result<Due, String> {
    let t = ops::get(tx, &w.task)?;
    let Some(a) = agents::in_project(tx, &w.agent, &w.project)? else {
        return Ok(Due::Off(t.id, "Its wait was called off: its agent is no longer in the pool. Run it again.".into()));
    };
    if t.archived_at.is_some() {
        return Ok(Due::Off(t.id, "Its wait was called off: it was archived.".into()));
    }
    if t.status != w.status {
        return Ok(Due::Off(t.id, format!("Its wait for {} was called off: it moved to {}.", a.name, t.status)));
    }
    if pool::check_for(tx, &a, &w.project, Some(&w.task), None)?.is_some() {
        // a schedule's task closes its own schedule's idle session for room, as its firing does (terminum-schedules.md AC-161)
        if let Some(s) = crate::tasks::schedules::of_task(tx, &w.task)?.filter(|s| s.agent == w.agent)
            && let Some(r) = super::fire::own_idle(tx, &s)? {
            return Ok(Due::Close(r.id, s.key));
        }
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
                if let Err(why) = super::api::press_again(app, &id, body, true).await {
                    let (task, words) = (w.task.clone(), format!("Did not start after its wait: {why}"));
                    let _ = super::store(app, move |tx| call_off(tx, &task, &words)).await;
                }
                id
            }
            Ok(Due::Off(id, _)) => id,
            // closed, the next walk presses it, first of the line; one that stays open was said on its task
            Ok(Due::Close(run, key)) => {
                let _ = super::api::close_idle(app, &run, &key).await;
                continue;
            }
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
