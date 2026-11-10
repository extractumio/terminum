// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Slots (`terminum-dispatcher.md` AC-141): starting a process takes one of its agent's and one of its
//! project's, or is refused naming what holds them. Decided in the transaction that adds the run, so two
//! presses never both find one free; only starting is gated, a human typing never is.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::Transaction;
use serde::Serialize;
use serde_json::{json, Value};

use super::plan;
use super::runs::{self, Run};
use crate::tasks::agents::{self, Agent};
use crate::tasks::limits;
use crate::tasks::ops::{self, rows};
use crate::tasks::schedules;

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

/// What holds slots now, read once for every agent a window offers: the runs, the slots schedules keep by agent,
/// and the run standing in each kept slot, by run, its schedule (terminum-schedules.md AC-161).
pub struct Slots {
    holding: Vec<Run>,
    kept: BTreeMap<String, Vec<String>>,
    in_kept: HashMap<String, String>,
    /// the runs of Auto-Dispatcher's rounds: live and shown, but no slot is theirs (AC-170)
    rounds: HashSet<String>,
}

impl Slots {
    pub fn read(tx: &Transaction) -> Result<Slots, String> {
        let holding: Vec<Run> = runs::live(tx)?.into_iter().filter(Run::holds_slot).collect();
        let ids: Vec<&str> = holding.iter().map(|r| r.id.as_str()).collect();
        let rounds = rows(tx, "SELECT m.run FROM run_tasks m JOIN auto_rounds a ON a.task = m.task WHERE m.run IN (SELECT value FROM json_each(?1))", [runs::listed_keys(&ids)], |r| r.get(0))?.into_iter().collect();
        Ok(Slots { in_kept: in_kept(tx, &ids)?, kept: schedules::kept_by_agent(tx)?, holding, rounds })
    }

    /// Whether a process of `agent` (none: one deleted since, its slot no one's) can start in `project`, `but` run
    /// not counted: the one it goes on from.
    pub fn check(&self, tx: &Transaction, agent: Option<&Agent>, project: &str, but: Option<&str>) -> Result<Option<Full>, String> {
        let holding: Vec<&Run> = self.counted().filter(|r| Some(r.id.as_str()) != but).collect();
        let kept = agent.and_then(|a| self.kept.get(&a.id)).map_or(&[][..], Vec::as_slice);
        judge(tx, &holding, agent, project, (kept, &self.in_kept))
    }

    /// The runs that hold a slot: all but a round's.
    fn counted(&self) -> impl Iterator<Item = &Run> {
        self.holding.iter().filter(|r| !self.rounds.contains(&r.id))
    }

    /// How many more processes of `agent` can start, and how many more in `project` (none: it has no limit).
    pub fn free(&self, tx: &Transaction, agent: &Agent, project: &str) -> Result<(i64, Option<i64>), String> {
        let mine = self.counted().filter(|r| r.agent_id == agent.id && !self.in_kept.contains_key(&r.id)).count() + self.kept.get(&agent.id).map_or(0, Vec::len);
        let there = self.counted().filter(|r| r.project == project).count() as i64;
        Ok(((agent.at_once - mine as i64).max(0), limits::agent_limit(tx, project)?.map(|most| (most - there).max(0))))
    }

    /// `check` for a run of task `task` alone (its key; none: several tasks): one of a schedule of `agent` takes the
    /// slot kept for it while no other run, but `but`, stands there, only its project's limit holding it (AC-161).
    pub fn check_for(&self, tx: &Transaction, agent: &Agent, project: &str, task: Option<&str>, but: Option<&str>) -> Result<Option<Full>, String> {
        let schedule = match task {
            Some(t) => kept_schedule(tx, &agent.id, t)?,
            None => None,
        };
        let free = schedule.is_some_and(|s| !self.in_kept.iter().any(|(run, of)| *of == s && Some(run.as_str()) != but));
        self.check(tx, (!free).then_some(agent), project, but)
    }
}

/// `check_for`, read now.
pub fn check_for(tx: &Transaction, agent: &Agent, project: &str, task: Option<&str>, but: Option<&str>) -> Result<Option<Full>, String> {
    Slots::read(tx)?.check_for(tx, agent, project, task, but)
}

/// The schedule of `agent` keeping a slot that task `task` (its key) was made by, if one (AC-161).
fn kept_schedule(tx: &Transaction, agent: &str, task: &str) -> Result<Option<String>, String> {
    Ok(rows(tx, "SELECT s.id FROM schedule_fires f JOIN schedules s ON s.id = f.schedule WHERE f.task = ?1 AND s.agent_id = ?2 AND s.paused = 0 AND s.deleted_at IS NULL",
            (task, agent), |r| r.get(0))?.pop())
}

/// The run standing in each kept slot, of the runs `ids`: a run of one task of its agent's schedule that keeps one, the
/// oldest; a second, or a run of several tasks, counts as any run (AC-161). By run, its schedule.
fn in_kept(tx: &Transaction, ids: &[&str]) -> Result<HashMap<String, String>, String> {
    let found: Vec<(String, String)> = rows(tx, "SELECT r.id, s.id FROM runs r JOIN run_tasks rt ON rt.run = r.id JOIN schedule_fires f ON f.task = rt.task JOIN schedules s ON s.id = f.schedule
                 WHERE r.id IN (SELECT value FROM json_each(?1)) AND s.agent_id = r.agent_id AND s.paused = 0 AND s.deleted_at IS NULL
                   AND (SELECT count(*) FROM run_tasks x WHERE x.run = r.id) = 1 ORDER BY r.rowid",
            [runs::listed_keys(ids)], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let mut seen = HashSet::new();
    Ok(found.into_iter().filter(|(_, s)| seen.insert(s.clone())).collect())
}

/// `check` of one agent, read now.
pub fn check(tx: &Transaction, agent: Option<&Agent>, project: &str, but: Option<&str>) -> Result<Option<Full>, String> {
    Slots::read(tx)?.check(tx, agent, project, but)
}

/// `kept`: the slots `agent`'s schedules keep, and the runs standing in kept slots: those count once (AC-161).
fn judge(tx: &Transaction, holding: &[&Run], agent: Option<&Agent>, project: &str, (kept, in_kept): (&[String], &HashMap<String, String>)) -> Result<Option<Full>, String> {
    let of_agent: Vec<&Run> = holding.iter().copied().filter(|r| agent.is_some_and(|a| a.id == r.agent_id) && !in_kept.contains_key(&r.id)).collect();
    let of_project: Vec<&Run> = holding.iter().copied().filter(|r| r.project == project).collect();
    let limit = limits::agent_limit(tx, project)?;
    let mut full: Vec<(&[&Run], i64)> = Vec::new();
    let mut words = None;
    if let Some(a) = agent.filter(|a| (of_agent.len() + kept.len()) as i64 >= a.at_once) {
        let held: Vec<String> = said(tx, &of_agent)?.into_iter().chain(kept.iter().map(|k| format!("{k} (kept)"))).collect();
        words = Some(format!("{} is at its limit of {}: {}.", a.name, a.at_once, held.join("; ")));
        full.push((&of_agent, a.at_once - kept.len() as i64));
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
    let found: Vec<(String, Value)> = rows(tx, "SELECT m.run, x.slug, x.title, x.status FROM run_tasks m JOIN tasks x ON x.id = m.task
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
    let Slots { holding, kept, in_kept, rounds } = Slots::read(tx)?;
    let mut tasks = in_runs(tx, &holding.iter().map(|r| r.id.as_str()).collect::<Vec<_>>())?;
    let mut of_agent: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
    let mut in_project: HashMap<&str, i64> = HashMap::new();
    // a slot its schedules keep counts once, whether its run is live or not (AC-161)
    let mut others: HashMap<&str, usize> = HashMap::new();
    for r in &holding {
        let in_slot = in_kept.contains_key(&r.id);
        of_agent.entry(&r.agent_id).or_default().push(json!({"run": r.id, "tasks": r.names(), "state": r.state, "key": r.pane_key, "project": r.project, "idle": !r.busy() && !r.ad_hoc(), "kept": in_slot,
            "round": rounds.contains(&r.id), "list": tasks.remove(&r.id).unwrap_or_default()}));
        // a round's session is shown and takes no slot (AC-170)
        if rounds.contains(&r.id) {
            continue;
        }
        if !in_slot {
            *others.entry(&r.agent_id).or_default() += 1;
        }
        *in_project.entry(&r.project).or_default() += 1;
    }
    let full: Vec<String> = agents::list(tx)?.into_iter()
        .filter(|a| (others.get(a.id.as_str()).copied().unwrap_or(0) + kept.get(&a.id).map_or(0, Vec::len)) as i64 >= a.at_once).map(|a| a.id).collect();
    let full_projects: Vec<String> = limits::agent_limits(tx)?.into_iter().filter(|(p, most)| in_project.get(p.as_str()).copied().unwrap_or(0) >= *most).map(|(p, _)| p).collect();
    Ok(json!({"agents": of_agent, "kept": kept, "full": full, "full_projects": full_projects, "queue": super::waits::queues(tx)?}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::ops::{sql, NewTask};
    use crate::tasks::store::{Store, HUMAN};

    fn store(name: &str) -> Store {
        Store::open(crate::testutil::fresh_dir(name).join("kanban.sqlite"))
    }

    fn agent(id: &str, at_once: i64) -> Agent {
        Agent { id: id.into(), name: id.to_uppercase(), kind: "claude".into(), command: "claude".into(), effort: String::new(), color: 0, line: "claude".into(), at_once, week_limit: None, month_limit: None }
    }

    fn project(tx: &Transaction, id: &str, limit: Option<i64>) -> Result<(), String> {
        tx.execute("INSERT INTO projects (id, name, color, dir, place, agent_limit) VALUES (?1, ?1, 0, '/srv', 0, ?2)", (id, limit)).map(drop).map_err(sql)
    }

    /// A run on `a` in `p`, working, its task open.
    fn working(tx: &Transaction, a: &str, p: &str) -> (String, ops::Task) {
        let t = ops::create(tx, HUMAN, NewTask { title: format!("t in {p}"), project: Some(p.into()), ..Default::default() }, 1).unwrap();
        let id = runs::add(tx, &Run { agent_id: a.into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: p.into(), ..Default::default() }, std::slice::from_ref(&t.key), 1).unwrap();
        runs::change(tx, &id, |r| { r.state = crate::dispatcher::runs::Phase::Working; r.pane_key = Some(format!("tmux:local:t:%{id}")); }).unwrap();
        (id, t)
    }

    /// The same, finished and quiet.
    fn idle(tx: &Transaction, a: &str, p: &str) -> (String, ops::Task) {
        let (id, t) = working(tx, a, p);
        ops::move_to(tx, HUMAN, &t.id, "DONE", 2).unwrap();
        runs::change(tx, &id, |r| { r.turn_ended = true; r.closing_for = 60; }).unwrap();
        (id, t)
    }

    #[test]
    fn ac141_the_agent_s_limit_names_what_holds_it() {
        store("pool-agent").with(|tx| {
            project(tx, "api", None)?;
            assert_eq!(check(tx, Some(&agent("a", 1)), "api", None)?, None);
            let (_, t) = working(tx, "a", "api");
            let full = check(tx, Some(&agent("a", 1)), "api", None)?.unwrap();
            assert_eq!(full, Full { words: format!("A is at its limit of 1: {} is working.", t.id), close: None }, "working: nothing to close");
            assert_eq!(check(tx, Some(&agent("a", 2)), "api", None)?, None, "two at once");
            assert_eq!(check(tx, Some(&agent("b", 1)), "api", None)?, None, "another agent's slots are its own");
            Ok(())
        }).unwrap();
    }

    #[test]
    fn ac141_a_run_of_several_tasks_is_one_holder() {
        store("pool-several").with(|tx| {
            project(tx, "api", None)?;
            let (id, t) = working(tx, "a", "api");
            let other = ops::create(tx, HUMAN, NewTask { title: "its second".into(), project: Some("api".into()), ..Default::default() }, 1)?;
            tx.execute("INSERT INTO run_tasks (run, task) VALUES (?1, ?2)", (&id, &other.key)).map_err(ops::sql)?;
            assert_eq!(check(tx, Some(&agent("a", 1)), "api", None)?.unwrap().words, format!("A is at its limit of 1: {}, {} are working.", t.id, other.id));
            Ok(())
        }).unwrap();
    }

    #[test]
    fn ac141_the_project_s_limit_counts_every_agent_and_only_its_own_runs() {
        store("pool-project").with(|tx| {
            project(tx, "api", Some(2))?;
            project(tx, "web", None)?;
            let (_, one) = working(tx, "a", "api");
            let (_, two) = working(tx, "b", "api");
            let full = check(tx, Some(&agent("c", 1)), "api", None)?.unwrap();
            assert_eq!(full.words, format!("api runs 2 agents at once, its limit: {} is working; {} is working.", one.id, two.id));
            assert_eq!(check(tx, Some(&agent("c", 1)), "web", None)?, None, "another project's");
            Ok(())
        }).unwrap();
    }

    #[test]
    fn ac141_an_idle_holder_can_be_closed_for_it_and_only_one() {
        store("pool-idle").with(|tx| {
            project(tx, "api", None)?;
            let (old, t) = idle(tx, "a", "api");
            let full = check(tx, Some(&agent("a", 1)), "api", None)?.unwrap();
            assert_eq!(full.close, Some(Holder { run: old.clone(), tasks: t.id.clone() }));
            assert_eq!(full.words, format!("A is at its limit of 1: {} is idle.", t.id));
            let (_, w) = working(tx, "a", "api");
            assert_eq!(check(tx, Some(&agent("a", 1)), "api", None)?.unwrap().words, format!("A is at its limit of 1: {} is idle; {} is working.", t.id, w.id), "each holder with what it does");
            ops::move_to(tx, HUMAN, &w.id, "WAIT FOR HUMAN", 3)?;
            assert!(check(tx, Some(&agent("a", 1)), "api", None)?.unwrap().words.ends_with(&format!("{} waits for a human.", w.id)));
            assert_eq!(check(tx, Some(&agent("a", 1)), "api", None)?.unwrap().close, None, "over its limit: one closing would not do, a human may have typed");
            assert!(check(tx, Some(&agent("a", 2)), "api", None)?.unwrap().close.is_some(), "at its limit of two, one idle");
            Ok(())
        }).unwrap();
    }

    #[test]
    fn ac141_both_limits_need_one_holder_of_both() {
        store("pool-both").with(|tx| {
            project(tx, "api", Some(1))?;
            project(tx, "web", None)?;
            idle(tx, "a", "web");
            working(tx, "b", "api");
            assert_eq!(check(tx, Some(&agent("a", 1)), "api", None)?.unwrap().close, None, "closing web's frees nothing of api's");
            Ok(())
        }).unwrap();
    }

    #[test]
    fn ac141_a_session_with_another_task_open_is_not_idle() {
        store("pool-other").with(|tx| {
            project(tx, "api", None)?;
            let (id, _) = idle(tx, "a", "api");
            let key = runs::change(tx, &id, |_| {})?.pane_key.unwrap();
            let taken = ops::create(tx, HUMAN, NewTask { title: "taken".into(), project: Some("api".into()), session: Some(key), ..Default::default() }, 1)?;
            assert_eq!(check(tx, Some(&agent("a", 1)), "api", None)?.unwrap().close, None, "{} stands on it by take_task", taken.id);
            Ok(())
        }).unwrap();
    }

    #[test]
    fn ac141_what_it_goes_on_from_does_not_count_and_a_deleted_agent_has_no_slot() {
        store("pool-but").with(|tx| {
            project(tx, "api", None)?;
            let (id, _) = working(tx, "a", "api");
            assert_eq!(check(tx, Some(&agent("a", 1)), "api", Some(&id))?, None);
            assert_eq!(check(tx, None, "api", None)?, None, "only the project's limit could stop it");
            Ok(())
        }).unwrap();
    }

    #[test]
    fn ac141_what_is_over_or_gone_holds_nothing() {
        store("pool-over").with(|tx| {
            project(tx, "api", None)?;
            for state in ["closed", "left", "gone"] {
                let (id, _) = working(tx, "a", "api");
                assert!(check(tx, Some(&agent("a", 1)), "api", None)?.is_some());
                runs::change(tx, &id, |r| r.state = runs::Phase::of(state).unwrap())?;
                assert_eq!(check(tx, Some(&agent("a", 1)), "api", None)?, None, "{state}");
            }
            Ok(())
        }).unwrap();
    }
}
