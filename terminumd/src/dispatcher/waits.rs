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
    /// the store keys of the tasks that must be DONE before it goes on (AC-210); none for a wait for room alone
    pub after: Vec<String>,
}

/// Whether the task of wait `w` is a schedule's.
macro_rules! scheduled {
    () => { "EXISTS (SELECT 1 FROM schedule_fires f WHERE f.task = w.task)" };
}
const SCHEDULED: &str = scheduled!();

/// The line's order: schedules' tasks first, then the place each was given; every reader of the line sorts by it.
pub(crate) const ORDER: &str = concat!("NOT ", scheduled!(), ", w.pos, w.rowid");

fn select() -> String {
    format!("SELECT w.task, w.agent_id, w.project, w.status, w.body, w.at, w.pos, {SCHEDULED}, w.after FROM run_waits w")
}

fn of_row(r: &rusqlite::Row) -> rusqlite::Result<Wait> {
    let (body, after): (String, Option<String>) = (r.get(4)?, r.get(8)?);
    let after = after.and_then(|a| serde_json::from_str(&a).ok()).unwrap_or_default();
    Ok(Wait { task: r.get(0)?, agent: r.get(1)?, project: r.get(2)?, status: r.get(3)?, body: serde_json::from_str(&body).unwrap_or(Value::Null), at: r.get(5)?, pos: r.get(6)?, scheduled: r.get(7)?, after })
}

/// Every wait, in the line's order.
pub fn all(tx: &Transaction) -> Result<Vec<Wait>, String> {
    rows(tx, &format!("{} ORDER BY {ORDER}", select()), [], of_row)
}

/// Every wait for room alone (AC-158), in the line's order: one for other tasks holds no place (AC-210).
fn for_room(tx: &Transaction) -> Result<Vec<Wait>, String> {
    rows(tx, &format!("{} WHERE w.after IS NULL ORDER BY {ORDER}", select()), [], of_row)
}

/// How many of this project's presses are in the queue and when the oldest joined.
pub(crate) fn queue_depth(tx: &Transaction, project: &str) -> Result<(i64, Option<i64>), String> {
    tx.query_row("SELECT count(*), min(at) FROM run_waits WHERE project = ?1 AND after IS NULL",
        [project], |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)
}

/// The wait of `task` is for room alone from now on: the tasks it waited for are DONE (AC-210).
fn room_alone(tx: &Transaction, task: &str) -> Result<(), String> {
    tx.execute("UPDATE run_waits SET after = NULL WHERE task = ?1", [task]).map(drop).map_err(sql)
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
    /// It waits for other tasks to be DONE (AC-210): it holds no place in a queue until they are.
    pub fn for_tasks(&self) -> bool {
        !self.after.is_empty()
    }

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
    add_after(tx, task, agent, project, body, &[], at)
}

/// `task` waits for `agent` with `body` once the tasks of store keys `after` are DONE (AC-210), none: for room alone.
pub fn add_after(tx: &Transaction, task: &ops::Task, agent: &str, project: &str, body: &Value, after: &[String], at: i64) -> Result<(), String> {
    let after = (!after.is_empty()).then(|| serde_json::json!(after).to_string());
    tx.execute("INSERT INTO run_waits (task, agent_id, project, status, body, at, pos, after) VALUES (?1, ?2, ?3, ?4, ?5, ?6, (SELECT COALESCE(MAX(pos), 0) + 1 FROM run_waits), ?7)
                ON CONFLICT (task) DO UPDATE SET agent_id = excluded.agent_id, project = excluded.project, status = excluded.status, body = excluded.body, after = excluded.after",
               (&task.key, agent, project, &task.status, body.to_string(), at, after)).map(drop).map_err(sql)
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
    for w in for_room(tx)?.into_iter().filter(|w| w.task != task && w.before(scheduled, place) && (w.agent == agent || w.project == project)) {
        if let Some(a) = agents::in_project(tx, &w.agent, &w.project)? && slots.check_for(tx, &a, &w.project, Some(&w.task), None)?.is_none() {
            return Ok(Some(ops::get(tx, &w.task)?.id));
        }
    }
    Ok(None)
}

/// Each agent's queue for the board (AC-169): its waiting presses in the line's order, each with its task's title and
/// project, and the other tasks of a press for several (`also`).
pub fn queues(tx: &Transaction) -> Result<BTreeMap<String, Vec<Value>>, String> {
    let found: Vec<(String, Value)> = rows(tx, &format!("SELECT w.agent_id, t.slug, t.shown, w.project, COALESCE(json_extract(w.body, '$.also'), '[]') FROM run_waits w JOIN tasks t ON t.id = w.task WHERE w.after IS NULL ORDER BY {ORDER}"), [],
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
    let name = if w.for_tasks() { "the tasks it waited for".to_string() } else { agents::name_of(tx, &w.agent)? };
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

/// Each wait and the tasks it carries (AC-158, AC-169). A wait for other tasks (AC-210) holds its primary alone.
fn members() -> String {
    let also = crate::tasks::slugs::key_named("j.value");
    format!("WITH chosen AS (SELECT w.task press, w.task, w.after IS NULL for_room FROM run_waits w
            UNION SELECT w.task, t.id, 1 FROM run_waits w JOIN tasks p ON p.id = w.task
                JOIN json_each(w.body, '$.also') j
                JOIN tasks t ON t.id = {also} AND t.project = p.project
                WHERE w.after IS NULL AND j.type = 'text')
        SELECT press, task FROM chosen
        UNION SELECT c.press, t.id FROM chosen c JOIN tasks t ON t.parent = c.task
            WHERE c.for_room AND t.status <> 'DONE' AND t.archived_at IS NULL
                AND NOT EXISTS (SELECT 1 FROM schedule_fires f JOIN schedules s ON s.id = f.schedule WHERE f.task = c.task)")
}

/// The wait that protects this task from an agent's move or result. Prefer its own wait, then the first press carrying it.
pub(crate) fn protecting(tx: &Transaction, task: &str) -> Result<Option<Wait>, String> {
    let query = format!("{} WHERE w.task IN (SELECT press FROM ({}) WHERE task = ?1)
        ORDER BY {} LIMIT 1", select(), members(), member_order("?1"));
    tx.prepare_cached(&query).map_err(sql)?.query_row([task], of_row).optional().map_err(sql)
}

fn member_order(task: &str) -> String {
    format!("w.task <> {task}, {ORDER}")
}

/// Whether task alias `t` belongs to a queued press, including its other tasks (AC-169).
/// Aliases are source literals, never a request's words.
pub fn queued_task(t: &str) -> String {
    format!("{t}.id IN (SELECT task FROM ({}))", members())
}

/// A card's wait: its agent, place in that agent's whole queue and the primary task that can cancel the press; or the
/// tasks it waits for (AC-210), which hold no place in a queue.
pub struct CardWait {
    pub agent: String,
    pub position: i64,
    pub task: String,
    pub after: Vec<String>,
}

impl CardWait {
    pub fn mark(&self, card: &mut Value) {
        if !self.after.is_empty() {
            card["waits_for"] = serde_json::json!(self.after);
            return;
        }
        card["waits"] = serde_json::json!(self.agent);
        card["wait_position"] = serde_json::json!(self.position);
        card["wait_task"] = serde_json::json!(self.task);
    }
}

/// Queue positions are ranked before card filters and pagination, shared by every task of one press (AC-169).
pub fn on_cards(tx: &Transaction, keys: &[&str]) -> Result<HashMap<String, CardWait>, String> {
    if keys.is_empty() { return Ok(HashMap::new()); }
    let found = rows(tx, &format!("WITH queue AS (
            SELECT w.task, t.slug, COALESCE(a.name, w.agent_id) agent,
                row_number() OVER (PARTITION BY w.agent_id, w.after IS NULL ORDER BY {ORDER}) position,
                CASE WHEN w.after IS NULL THEN '[]' ELSE (SELECT json_group_array(x.slug) FROM json_each(w.after) j JOIN tasks x ON x.id = j.value) END after
            FROM run_waits w JOIN tasks t ON t.id = w.task LEFT JOIN agents a ON a.id = w.agent_id), members AS ({})
        SELECT m.task, q.agent, q.position, q.slug, q.after FROM members m
        JOIN queue q ON q.task = m.press JOIN run_waits w ON w.task = m.press
        WHERE m.task IN (SELECT value FROM json_each(?1))
        ORDER BY {}", members(), member_order("m.task")),
        [super::runs::listed_keys(keys)], |r| Ok((r.get(0)?, CardWait { agent: r.get(1)?, position: r.get(2)?, task: r.get(3)?,
            after: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default() })))?;
    let mut marks = HashMap::new();
    for (key, mark) in found {
        marks.entry(key).or_insert(mark);
    }
    Ok(marks)
}

/// What a wait comes to now: pressed again, still waiting, or called off with these words; the task's ID. Or, a
/// schedule's task with no room, its schedule's idle session whose close makes room: the run, the schedule's key.
#[derive(Debug)]
enum Due {
    Press(String, Value),
    Stays,
    Off(String, String),
    /// a task it waited for will not be DONE: a human decides, these words asking (AC-210)
    Asks(String, String),
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
    if w.for_tasks() {
        match super::after::blockers(tx, &t.id, &w.after)? {
            super::after::Blockers::Open => return Ok(Due::Stays),
            super::after::Blockers::Gone(words) => return Ok(Due::Asks(t.id, words)),
            // all DONE: from now on it is a wait for room alone, in its place in the line
            super::after::Blockers::Done => room_alone(tx, &w.task)?,
        }
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
            match &d {
                Due::Off(_, words) => call_off(tx, &wanted.task, words)?,
                Due::Asks(id, words) => super::after::ask(tx, &wanted.task, id, words)?,
                _ => {}
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
            Ok(Due::Off(id, _) | Due::Asks(id, _)) => id,
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
