// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;
use crate::tasks::ops::{sql, NewTask};
use crate::tasks::store::{Store, HUMAN};

fn store(name: &str) -> Store {
    Store::open(crate::testutil::fresh_dir(name).join("kanban.sqlite"))
}

fn agent(id: &str, at_once: i64) -> Agent {
    Agent { id: id.into(), project: "default".into(), name: id.to_uppercase(), kind: "claude".into(), command: "claude".into(), effort: String::new(), color: 0, line: "claude".into(), at_once, week_limit: None, month_limit: None }
}

fn project(tx: &Transaction, id: &str, limit: Option<i64>) -> Result<(), String> {
    tx.execute("INSERT INTO projects (id, name, color, dir, place, agent_limit) VALUES (?1, ?1, 0, '/srv', 0, ?2)", (id, limit)).map(drop).map_err(sql)
}

/// A run on `a` in `p`, working, its task open.
fn working(tx: &Transaction, a: &str, p: &str) -> (String, ops::Task) {
    let t = ops::create(tx, HUMAN, NewTask { title: format!("t in {p}"), project: Some(p.into()), ..Default::default() }, 1).unwrap();
    let id = runs::add(tx, &Run { agent_id: a.into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: p.into(), ..Default::default() }, std::slice::from_ref(&t.key), Default::default(), 1).unwrap();
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

/// AC-169, AC-198: integration waits must leave room for their own Verify prerequisites.
#[test]
fn ac169_four_integration_waits_free_slots_and_a_return_counts_again() {
    let root = crate::testutil::fresh_dir("pool-integration");
    let s = Store::open(root.join("kanban.sqlite"));
    let returned = s.with(|tx| {
        project(tx, "api", Some(3))?;
        tx.execute("INSERT INTO agents (id, project, name, kind, command, color, at_once) VALUES ('a', 'api', 'A', 'codex', 'codex', 0, 3)", []).map_err(sql)?;
        let mut queued = Vec::new();
        for _ in 0..4 {
            let (id, t) = working(tx, "a", "api");
            ops::place(tx, HUMAN, &t, ops::INTEGRATION, 2)?;
            // No Stop is needed: integrate itself hands the work off.
            assert!(runs::by_id(tx, &id)?.holds_slot(), "session ownership and task protection remain");
            queued.push(t);
        }
        let slots = Slots::read(tx)?;
        assert_eq!((slots.free_of(&agent("a", 3)), slots.free_in(tx, "api")?), (3, Some(3)));
        assert_eq!(check(tx, Some(&agent("a", 3)), "api", None)?, None);
        let blocker = ops::create(tx, HUMAN, NewTask { title: "Verify prerequisite".into(), project: Some("api".into()), ..Default::default() }, 3)?;
        super::super::waits::add(tx, &blocker, "a", "api", &json!({"agent": "a"}), 3)?;
        assert_eq!(super::super::waits::ahead(tx, "a", "api", "newer-task")?, Some(blocker.id), "an already queued blocker has room ahead of new presses");
        let shown = board(tx)?;
        assert!(shown["full"].as_array().unwrap().is_empty() && shown["full_projects"].as_array().unwrap().is_empty());
        let visible = shown["agents"]["a"].as_array().unwrap();
        assert_eq!(visible.len(), 4, "AC-138: all retained sessions stay visible");
        assert!(visible.iter().all(|r| r["integration"] == true && r["list"][0]["status"] == ops::INTEGRATION));
        for _ in 0..3 { working(tx, "a", "api"); }
        Ok(queued[0].id.clone())
    }).unwrap();
    // A restart derives occupancy from the same tasks; nothing transient owns the exemption.
    Store::open(root.join("kanban.sqlite")).with(|tx| {
        assert_eq!(Slots::read(tx)?.holding.len(), 3);
        let t = ops::get(tx, &returned)?;
        crate::integrate::store::give_back(tx, &t.key, "returned: red check", crate::integrate::store::Left::Red, 4)?;
        assert_eq!(Slots::read(tx)?.holding.len(), 4, "a live return may exceed the limit under AC-141");
        assert_eq!(runs::live(tx)?.len(), 7, "the integration sessions stayed watched and visible");
        let shown = board(tx)?;
        let visible = shown["agents"]["a"].as_array().unwrap();
        assert_eq!(visible.len(), 7, "all sessions remain openable after a return");
        assert_eq!(visible.iter().filter(|r| r["integration"] == true).count(), 3, "only the returned wait counts again");
        let full = check(tx, Some(&agent("a", 3)), "api", None)?.unwrap();
        assert!(full.words.contains(&returned) && full.close.is_none(), "later starts wait, no integration pane closed");
        Ok(())
    }).unwrap();
}

#[test]
fn ac169_only_runs_with_every_unfinished_task_in_integration_free_room() {
    store("pool-integration-mixed").with(|tx| {
        project(tx, "api", Some(1))?;
        let (id, queued) = working(tx, "a", "api");
        ops::place(tx, HUMAN, &queued, ops::INTEGRATION, 2)?;
        for state in [runs::Phase::Starting, runs::Phase::Asking, runs::Phase::Typed] {
            runs::change(tx, &id, |r| r.state = state)?;
            assert!(check(tx, Some(&agent("a", 1)), "api", None)?.is_some(), "{state:?}: a new process keeps its reservation before accepting the task");
        }
        runs::change(tx, &id, |r| r.state = runs::Phase::Working)?;
        assert!(check(tx, Some(&agent("a", 1)), "api", None)?.is_none(), "the handed-off work frees room after startup");
        let other = ops::create(tx, HUMAN, NewTask { title: "another task".into(), project: Some("api".into()), ..Default::default() }, 1)?;
        tx.execute("INSERT INTO run_tasks (run, task) VALUES (?1, ?2)", (&id, &other.key)).map_err(sql)?;
        for status in ["IN PROGRESS", "WAIT FOR HUMAN", "TODO", "DONE", "FAILED"] {
            ops::move_to(tx, HUMAN, &other.id, status, 3)?;
            let finished = matches!(status, "DONE" | "FAILED");
            assert_eq!(check(tx, Some(&agent("a", 1)), "api", None)?.is_none(), finished, "{status}");
            assert_eq!(runs::by_id(tx, &id)?.status == ops::INTEGRATION, finished, "{status}: unfinished status wins");
        }
        Ok(())
    }).unwrap();
}

/// A schedule of agent `a`; its id.
fn schedule_on(tx: &Transaction, a: &str) -> String {
    let v = json!({"title": "Nightly", "project": "api", "agent": a, "cron": "0 3 * * *", "budget_minutes": 5});
    schedules::add(tx, HUMAN, &schedules::Given::of(tx, &v, 1).unwrap(), 1).unwrap().0
}

/// A run of a task of schedule `s` on `a`, working.
fn scheduled_run(tx: &Transaction, a: &str, s: &str) -> String {
    let (id, t) = working(tx, a, "api");
    schedules::fired(tx, &schedules::by_id(tx, s).unwrap(), (1, 0), Some(&t.key), "started", None, 1).unwrap();
    id
}

#[test]
fn ac161_kept_slots_are_counted_and_shown_by_count() {
    store("pool-kept-count").with(|tx| {
        project(tx, "api", None)?;
        tx.execute("INSERT INTO agents (id, project, name, kind, command, color, at_once) VALUES ('a', 'api', 'A', 'claude', 'claude', 0, 3)", []).map_err(sql)?;
        let a = agents::in_project(tx, "a", "api")?.unwrap();
        let (one, two, three) = (schedule_on(tx, "a"), schedule_on(tx, "a"), schedule_on(tx, "a"));
        assert_eq!(Slots::read(tx)?.free_of(&a), 1, "three schedules keep two of its three");
        let shown = board(tx)?;
        assert_eq!(shown["kept"]["a"], json!({"slots": 2, "schedules": 3, "integration": false}), "the card's rings: a count, no schedule named");
        let (first, second, third) = (scheduled_run(tx, "a", &one), scheduled_run(tx, "a", &two), scheduled_run(tx, "a", &three));
        let slots = Slots::read(tx)?;
        assert_eq!((slots.slot_of(&first), slots.slot_of(&second), slots.slot_of(&third)), (Some(Slot::Schedule), Some(Slot::Schedule), None), "the two oldest stand in the kept slots, the third counts as ordinary");
        assert_eq!(slots.free_of(&a), 0);
        assert_eq!(check(tx, Some(&a), "api", None)?.unwrap().words, format!("A is at its limit of 3: {} is working; 2 kept for 3 schedules.", runs::by_id(tx, &third)?.names()));
        schedules::pause(tx, &three, "paused", 2)?;
        schedules::pause(tx, &two, "paused", 2)?;
        let slots = Slots::read(tx)?;
        assert_eq!((slots.slot_of(&first), slots.slot_of(&second), slots.free_of(&a)), (Some(Slot::Schedule), None, 0), "K falls to one under three live runs: nothing moves, the agent is full until they end");
        assert_eq!(board(tx)?["full"], json!(["a"]));
        Ok(())
    }).unwrap();
}
