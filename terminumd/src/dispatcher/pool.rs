// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Slots (`terminum-dispatcher.md` AC-141): starting a process takes one of its agent's and one of its
//! project's, or is refused naming what holds them. Decided in the transaction that adds the run, so two
//! presses never both find one free; only starting is gated, a human typing never is.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{OptionalExtension, Transaction};
use serde::Serialize;
use serde_json::{json, Value};

use super::plan;
use super::runs::{self, Run};
use crate::tasks::agents::{self, Agent};
use crate::tasks::limits;
use crate::tasks::ops::{self, rows};
use crate::tasks::schedules::{self, Kept};

/// A limit reached: what to say, and the idle holder whose closing would make room, if one would.
#[derive(Debug, PartialEq, Serialize)]
pub struct Full {
    pub words: String,
    pub close: Option<Holder>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Holder {
    pub run: String,
    pub tasks: String,
}

/// Whether a session may be closed for a slot, as the store knows it: its run idle (`plan::idle_holder`) and
/// no task taken in it still open. What its pane shows is read just before the close.
pub fn closable(tx: &Transaction, r: &Run) -> Result<bool, String> {
    Ok(plan::idle_holder(r) && !r.pane_key.as_deref().map_or(Ok(false), |key| runs::others_open(tx, key, &r.task))?)
}

/// A holder and what it does, as the store knows it, in the words of a limit (AC-141): only one "idle" can be closed for
/// its slot.
fn doing(tx: &Transaction, r: &Run) -> Result<String, String> {
    if r.ad_hoc() {
        return Ok("a session with no task".to_string());
    }
    // one task, several
    let (one, many) = if r.is_starting() {
        ("is starting", "are starting")
    } else if r.state == runs::Phase::Ending {
        ("is being ended", "are being ended")
    } else if r.status == "WAIT FOR HUMAN" {
        ("waits for a human", "wait for a human")
    } else if r.busy() {
        ("is working", "are working")
    } else if closable(tx, r)? {
        ("is idle", "are idle")
    } else {
        ("has finished, not idle yet", "have finished, not idle yet")
    };
    let what = if r.targets().len() > 1 { many } else { one };
    Ok(format!("{} {what}", r.names()))
}

fn said(tx: &Transaction, holders: &[&Run]) -> Result<Vec<String>, String> {
    holders.iter().map(|r| doing(tx, r)).collect()
}

/// One closing frees a place only for what stands exactly at its limit, and only if it holds a slot of each that does.
fn victim<'a>(tx: &Transaction, full: &[(&[&'a Run], i64)]) -> Result<Option<&'a Run>, String> {
    let Some((first, _)) = full.first() else { return Ok(None) };
    if full.iter().any(|(holders, limit)| holders.len() as i64 != *limit) {
        return Ok(None);
    }
    let mut idle = Vec::new();
    for r in first.iter().copied().filter(|r| full.iter().all(|(holders, _)| holders.iter().any(|h| h.id == r.id))) {
        if closable(tx, r)? {
            idle.push(r);
        }
    }
    Ok(idle.into_iter().max_by_key(|r| r.closing_for))
}

/// What a kept slot is kept for (AC-161, terminum-integration.md AC-194).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Schedule,
    Integration,
    Supervisor,
}

impl Kept {
    /// How many it keeps for `slot`.
    fn of_slot(&self, slot: Slot) -> i64 {
        match slot {
            Slot::Schedule => self.slots,
            Slot::Integration => i64::from(self.integration),
            Slot::Supervisor => i64::from(self.supervisor),
        }
    }
}

/// What holds slots now, read once for every agent a window offers: the runs, the slots each agent keeps, and the runs
/// standing in kept slots, by run, its agent and slot (terminum-schedules.md AC-161).
pub struct Slots {
    holding: Vec<Run>,
    kept: BTreeMap<String, Kept>,
    in_kept: HashMap<String, (String, Slot)>,
    /// the runs of Auto-Dispatcher's rounds: live and shown, but no slot is theirs, so they are not in `holding` (AC-170)
    rounds: Vec<Run>,
    /// retained sessions whose work is in integration (AC-169), and an ended integration task's session, idle until its
    /// close (AC-125, terminum-integration.md AC-194): shown, but no ordinary slot is theirs
    integrating: Vec<Run>,
}

impl Slots {
    pub fn read(tx: &Transaction) -> Result<Slots, String> {
        let live: Vec<Run> = runs::live(tx)?.into_iter().filter(Run::holds_slot).collect();
        let of_rounds: HashSet<String> = rows(tx, "SELECT m.run FROM run_tasks m JOIN auto_rounds a ON a.task = m.task", [], |r| r.get(0))?.into_iter().collect();
        let (rounds, ordinary): (Vec<Run>, Vec<Run>) = live.into_iter().partition(|r| of_rounds.contains(&r.id));
        let ended = ended_integration(tx, &ordinary)?;
        // Integration owns handed-off work until it returns; a new process still reserves room during startup.
        let (integrating, holding): (Vec<Run>, Vec<Run>) = ordinary.into_iter().partition(|r| !r.is_starting() && (r.status == ops::INTEGRATION || ended.contains(&r.id)));
        let ids: Vec<&str> = holding.iter().map(|r| r.id.as_str()).collect();
        let kept = schedules::kept_by_agent(tx)?;
        Ok(Slots { in_kept: in_kept(tx, &ids, &kept)?, kept, holding, rounds, integrating })
    }

    fn kept_of(&self, agent: &str) -> Kept {
        self.kept.get(agent).copied().unwrap_or_default()
    }

    /// The runs of `agent` that hold one of its slots, kept or ordinary.
    pub fn holding_of<'a>(&'a self, agent: &'a str) -> impl Iterator<Item = &'a Run> {
        self.holding.iter().filter(move |r| r.agent_id == agent)
    }

    /// The kept slot run `run` stands in, if one.
    #[cfg(test)]
    pub fn slot_of(&self, run: &str) -> Option<Slot> {
        self.in_kept.get(run).map(|(_, slot)| *slot)
    }

    /// Whether a process of `agent` (none: one deleted since, its slot no one's) can start in `project`, `but` run
    /// not counted: the one it goes on from.
    pub fn check(&self, tx: &Transaction, agent: Option<&Agent>, project: &str, but: Option<&str>) -> Result<Option<Full>, String> {
        let holding: Vec<&Run> = self.holding.iter().filter(|r| Some(r.id.as_str()) != but).collect();
        judge(tx, &holding, agent.map(|a| (a, self.kept_of(&a.id))), project, &self.in_kept)
    }

    /// How many more processes of `agent` can start, as `judge` counts its slots.
    pub fn free_of(&self, agent: &Agent) -> i64 {
        let all: Vec<&Run> = self.holding.iter().collect();
        (agent.at_once - of_agent(&all, agent, &self.in_kept).len() as i64 - self.kept_of(&agent.id).total()).max(0)
    }

    /// How many more can start in `project`; none: it has no limit.
    pub fn free_in(&self, tx: &Transaction, project: &str) -> Result<Option<i64>, String> {
        let there = self.holding.iter().filter(|r| r.project == project).count() as i64;
        Ok(limits::agent_limit(tx, project)?.map(|most| (most - there).max(0)))
    }

    /// `check` for a run of task `task` alone (its key; none: several tasks): one of `agent`'s schedules takes a kept
    /// slot while fewer than its K stand in them, only its project's limit holding it, else an ordinary slot (AC-161); a
    /// batch's integration task the slot kept for integration, not even that: a red batch never waits for room
    /// (terminum-integration.md AC-194).
    pub fn check_for(&self, tx: &Transaction, agent: &Agent, project: &str, task: Option<&str>, but: Option<&str>) -> Result<Option<Full>, String> {
        let slot = match task {
            Some(t) => kept_slot(tx, &agent.id, t)?,
            None => None,
        };
        self.check_in(tx, agent, project, slot, but)
    }

    /// `check_for` of a run that may stand in a kept slot for `slot`, when one is free. A schedules' slot is free only
    /// while the agent runs fewer than its at once: K raised over ordinary runs already there waits for them to end.
    pub fn check_in(&self, tx: &Transaction, agent: &Agent, project: &str, slot: Option<Slot>, but: Option<&str>) -> Result<Option<Full>, String> {
        let live = self.holding.iter().filter(|r| r.agent_id == agent.id && Some(r.id.as_str()) != but).count() as i64;
        let free = slot.filter(|s| {
            let used = self.in_kept.iter().filter(|(run, (of, kind))| kind == s && *of == agent.id && Some(run.as_str()) != but).count() as i64;
            used < self.kept_of(&agent.id).of_slot(*s) && (*s == Slot::Integration || live < agent.at_once)
        });
        match free {
            Some(Slot::Integration) => Ok(None),
            Some(Slot::Schedule | Slot::Supervisor) => self.check(tx, None, project, but),
            None if slot == Some(Slot::Supervisor) => Ok(Some(Full {
                words: format!("{} has no free slot kept for Supervisor investigations.", agent.name), close: None,
            })),
            None => self.check(tx, Some(agent), project, but),
        }
    }
}

/// The runs of `live` of a batch's integration task that ended, their agent idle: their sessions are never gone on in,
/// the next batch's is a task of its own, so none waits for their close (terminum-integration.md AC-194).
fn ended_integration(tx: &Transaction, live: &[Run]) -> Result<HashSet<String>, String> {
    let ids: Vec<&str> = live.iter().filter(|r| !r.is_starting() && !r.busy() && plan::finished(&r.status)).map(|r| r.id.as_str()).collect();
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    Ok(rows(tx, "SELECT m.run FROM run_tasks m JOIN integration_batches b ON b.task = m.task WHERE m.run IN (SELECT value FROM json_each(?1))",
            [runs::listed_keys(&ids)], |r| r.get(0))?.into_iter().collect())
}

/// `check_for`, read now.
pub fn check_for(tx: &Transaction, agent: &Agent, project: &str, task: Option<&str>, but: Option<&str>) -> Result<Option<Full>, String> {
    Slots::read(tx)?.check_for(tx, agent, project, task, but)
}

/// The kept slot of `agent` a run of task `task` (its key) alone may stand in, if one: a task of one of its schedules
/// not paused (AC-161), or a batch's integration task on its project's integration agent (AC-194).
fn kept_slot(tx: &Transaction, agent: &str, task: &str) -> Result<Option<Slot>, String> {
    let integration: Option<i64> = tx.query_row("SELECT 1 FROM integration_batches b JOIN project_integration i ON i.project = b.project WHERE b.task = ?1 AND i.agent = ?2 AND i.is_on = 1",
        (task, agent), |r| r.get(0)).optional().map_err(ops::sql)?;
    if integration.is_some() {
        return Ok(Some(Slot::Integration));
    }
    Ok(schedules::of_task(tx, task)?.filter(|s| s.active() && s.agent == agent).map(|_| Slot::Schedule))
}

/// The runs standing in kept slots, of the runs `ids`: of each agent's runs of one task of its schedules not paused,
/// the oldest, as many as `kept` says it keeps, counted, never placed; of its integration tasks' runs the oldest (AC-161,
/// terminum-integration.md AC-194). By run, its agent and slot.
fn in_kept(tx: &Transaction, ids: &[&str], kept: &BTreeMap<String, Kept>) -> Result<HashMap<String, (String, Slot)>, String> {
    if kept.values().all(|k| k.total() == 0) {
        return Ok(HashMap::new());
    }
    let ids = runs::listed_keys(ids);
    let scheduled: Vec<(String, String)> = rows(tx, "SELECT DISTINCT r.id, r.agent_id, r.rowid FROM runs r JOIN run_tasks rt ON rt.run = r.id JOIN schedule_fires f ON f.task = rt.task JOIN schedules s ON s.id = f.schedule
                 WHERE r.id IN (SELECT value FROM json_each(?1)) AND s.agent_id = r.agent_id AND s.paused = 0 AND s.deleted_at IS NULL
                   AND (SELECT count(*) FROM run_tasks x WHERE x.run = r.id) = 1 ORDER BY r.rowid",
            [&ids], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let integrating: Vec<(String, String)> = rows(tx, "SELECT DISTINCT r.id, r.agent_id, r.rowid FROM runs r JOIN run_tasks rt ON rt.run = r.id JOIN integration_batches b ON b.task = rt.task
                 JOIN project_integration i ON i.project = b.project WHERE r.id IN (SELECT value FROM json_each(?1)) AND i.agent = r.agent_id AND i.is_on = 1 ORDER BY r.rowid",
            [&ids], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let supervising = rows(tx, "SELECT r.id, r.agent_id FROM runs r JOIN supervisor_rounds q ON q.run = r.id
        WHERE q.ended_at IS NULL AND r.id IN (SELECT value FROM json_each(?1))",
        [&ids], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let mut out = HashMap::new();
    let mut used: HashMap<(String, Slot), i64> = HashMap::new();
    for (slot, found) in [(Slot::Schedule, scheduled), (Slot::Integration, integrating), (Slot::Supervisor, supervising)] {
        for (run, agent) in found {
            let most = kept.get(&agent).map_or(0, |k| k.of_slot(slot));
            let n = used.entry((agent.clone(), slot)).or_default();
            if *n < most && !out.contains_key(&run) {
                *n += 1;
                out.insert(run, (agent, slot));
            }
        }
    }
    Ok(out)
}

/// `check` of one agent, read now.
pub fn check(tx: &Transaction, agent: Option<&Agent>, project: &str, but: Option<&str>) -> Result<Option<Full>, String> {
    Slots::read(tx)?.check(tx, agent, project, but)
}

/// The runs of `holding` that take an ordinary slot of `agent`: its own, but those standing in kept slots (AC-161).
fn of_agent<'a>(holding: &[&'a Run], agent: &Agent, in_kept: &HashMap<String, (String, Slot)>) -> Vec<&'a Run> {
    holding.iter().copied().filter(|r| r.agent_id == agent.id && !in_kept.contains_key(&r.id)).collect()
}

/// `agent` with the slots it keeps; the runs standing in kept slots count once, as those slots (AC-161).
fn judge(tx: &Transaction, holding: &[&Run], agent: Option<(&Agent, Kept)>, project: &str, in_kept: &HashMap<String, (String, Slot)>) -> Result<Option<Full>, String> {
    let of_agent: Vec<&Run> = agent.map_or_else(Vec::new, |(a, _)| of_agent(holding, a, in_kept));
    let of_project: Vec<&Run> = holding.iter().copied().filter(|r| r.project == project).collect();
    let limit = limits::agent_limit(tx, project)?;
    let mut full: Vec<(&[&Run], i64)> = Vec::new();
    let mut words = None;
    if let Some((a, kept)) = agent.filter(|(a, kept)| of_agent.len() as i64 + kept.total() >= a.at_once) {
        let held: Vec<String> = said(tx, &of_agent)?.into_iter().chain(kept.held()).collect();
        words = Some(format!("{} is at its limit of {}: {}.", a.name, a.at_once, held.join("; ")));
        full.push((&of_agent, a.at_once - kept.total()));
    }
    if let Some(most) = limit.filter(|most| of_project.len() as i64 >= *most) {
        let name = ops::project_place(tx, project)?.0;
        if words.is_none() {
            words = Some(format!("{name} runs {most} agents at once, its limit: {}.", said(tx, &of_project)?.join("; ")));
        }
        full.push((&of_project, most));
    }
    let Some(words) = words else { return Ok(None) };
    let close = victim(tx, &full)?.map(|r| Holder { run: r.id.clone(), tasks: r.names() });
    Ok(Some(Full { words, close }))
}

/// Each of `runs`' tasks, as the agent's window lists them (AC-138): ID, title and status, in the order they were made.
fn in_runs(tx: &Transaction, ids: &[&str]) -> Result<HashMap<String, Vec<Value>>, String> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let found: Vec<(String, Value)> = rows(tx, "SELECT m.run, x.slug, x.shown, x.status FROM run_tasks m JOIN tasks x ON x.id = m.task
                 WHERE m.run IN (SELECT value FROM json_each(?1)) ORDER BY x.number",
            [runs::listed_keys(ids)], |r| Ok((r.get(0)?, json!({"id": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?, "status": r.get::<_, String>(3)?}))))?;
    let mut out: HashMap<String, Vec<Value>> = HashMap::new();
    for (run, task) in found {
        out.entry(run).or_default().push(task);
    }
    Ok(out)
}

/// What the board shows of the slots (AC-138): each agent's sessions, and the agents and projects at their limit.
pub fn board(tx: &Transaction) -> Result<Value, String> {
    let Slots { holding, kept, in_kept, rounds, integrating } = Slots::read(tx)?;
    let mut tasks = in_runs(tx, &holding.iter().chain(&rounds).chain(&integrating).map(|r| r.id.as_str()).collect::<Vec<_>>())?;
    let mut of_agent: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
    let mut in_project: HashMap<&str, i64> = HashMap::new();
    // a kept slot counts once, whether a run stands in it or not (AC-161)
    let mut others: HashMap<&str, usize> = HashMap::new();
    // rounds, integration waits and ended integration tasks' sessions stay visible on their agent without taking
    // ordinary slots (AC-138, AC-169, AC-170, terminum-integration.md AC-194)
    for (r, round, no_slot) in holding.iter().map(|r| (r, false, false))
        .chain(rounds.iter().map(|r| (r, true, false))).chain(integrating.iter().map(|r| (r, false, true))) {
        let in_slot = in_kept.contains_key(&r.id);
        let (integration, ended) = (no_slot && r.status == ops::INTEGRATION, no_slot && r.status != ops::INTEGRATION);
        let mut shown = json!({"run": r.id, "tasks": r.names(), "state": r.state, "key": r.pane_key, "project": r.project, "idle": !r.busy() && !r.ad_hoc(), "kept": in_slot,
            "round": round, "integration": integration, "ended": ended, "list": tasks.remove(&r.id).unwrap_or_default()});
        if in_kept.get(&r.id).is_some_and(|(_, slot)| *slot == Slot::Supervisor) {
            shown["supervisor"] = json!(true);
        }
        of_agent.entry(&r.agent_id).or_default().push(shown);
        if round || no_slot {
            continue;
        }
        if !in_slot {
            *others.entry(&r.agent_id).or_default() += 1;
        }
        *in_project.entry(&r.project).or_default() += 1;
    }
    let full: Vec<String> = agents::list(tx)?.into_iter()
        .filter(|a| others.get(a.id.as_str()).copied().unwrap_or(0) as i64 + kept.get(&a.id).map_or(0, Kept::total) >= a.at_once).map(|a| a.id).collect();
    let full_projects: Vec<String> = limits::agent_limits(tx)?.into_iter().filter(|(p, most)| in_project.get(p.as_str()).copied().unwrap_or(0) >= *most).map(|(p, _)| p).collect();
    Ok(json!({"agents": of_agent, "kept": kept, "full": full, "full_projects": full_projects, "queue": super::waits::queues(tx)?}))
}

#[cfg(test)]
#[path = "pool_tests.rs"]
mod tests;
