// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A RUN at a slot's limit waits for it (AC-158): kept in order, called off by what changed, ahead of newer presses.

use super::*;
use crate::dispatcher::runs::{self, Phase, Run};
use crate::tasks::ops::{NewTask, Task};
use crate::tasks::store::{Store, HUMAN};
use crate::tasks::tools::{self, Who};
use serde_json::json;

fn store(name: &str) -> Store {
    let s = Store::open(crate::testutil::fresh_dir(name).join("kanban.sqlite"));
    s.with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place, agent_limit) VALUES ('api', 'api', 0, '/srv', 0, NULL), ('web', 'web', 1, '/srv', 1, 1)", []).map_err(sql)?;
        tx.execute("INSERT INTO agents (id, project, name, kind, command, color, at_once) VALUES ('a', 'api', 'Smith', 'claude', 'claude', 0, 1), ('b', 'api', 'Jones', 'claude', 'claude', 1, 1)", []).map(drop).map_err(sql)
    }).unwrap();
    s
}

fn task(tx: &Transaction, project: &str) -> Task {
    ops::create(tx, HUMAN, NewTask { title: format!("t in {project}"), project: Some(project.into()), ..Default::default() }, 1).unwrap()
}

/// A run of agent `a` in `project` holding its slot.
fn holding(tx: &Transaction, a: &str, project: &str) -> String {
    let t = task(tx, project);
    let id = runs::add(tx, &Run { agent_id: a.into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: project.into(), ..Default::default() }, std::slice::from_ref(&t.key), Default::default(), 1).unwrap();
    runs::change(tx, &id, |r| { r.state = Phase::Working; r.pane_key = Some(format!("tmux:local:t:%{id}")); }).unwrap();
    id
}

#[test]
fn ac158_a_wait_keeps_its_place_and_its_newest_press() {
    store("waits-place").with(|tx| {
        let (one, two) = (task(tx, "api"), task(tx, "api"));
        add(tx, &one, "a", "api", &json!({"comment": "first"}), 10)?;
        add(tx, &two, "a", "api", &json!({}), 20)?;
        add(tx, &one, "b", "api", &json!({"comment": "again"}), 30)?;
        let all = all(tx)?;
        assert_eq!(all.iter().map(|w| w.task.as_str()).collect::<Vec<_>>(), [one.key.as_str(), two.key.as_str()], "its place in the line stays");
        assert_eq!((all[0].agent.as_str(), all[0].body["comment"].as_str(), all[0].at, all[0].status.as_str()), ("b", Some("again"), 10, "TODO"));
        assert_eq!(on_cards(tx, &[&one.key, &two.key])?.get(&one.key).map(|w| w.agent.as_str()), Some("Jones"), "a card names the agent it waits for");
        assert!(remove(tx, &one.key)? && !remove(tx, &one.key)?);
        Ok(())
    }).unwrap();
}

#[test]
fn ac169_cards_share_the_press_and_full_agent_queue_position() {
    store("waits-cards").with(|tx| {
        let (first, second, member, other, child) = (task(tx, "api"), task(tx, "api"), task(tx, "api"), task(tx, "api"), task(tx, "api"));
        crate::tasks::family::set_parent(tx, HUMAN, &child.id, Some(&second.id), 2)?;
        add(tx, &first, "a", "api", &json!({}), 10)?;
        add(tx, &other, "b", "api", &json!({}), 11)?;
        add(tx, &second, "a", "api", &json!({"also": [member.id]}), 12)?;
        let shown = on_cards(tx, &[&second.key, &member.key, &other.key, &child.key])?;
        for key in [&second.key, &member.key, &child.key] {
            let w = &shown[key];
            assert_eq!((w.agent.as_str(), w.position, w.task.as_str()), ("Smith", 2, second.id.as_str()), "filtered first card still takes a place; several tasks share one press");
        }
        assert_eq!(shown[&other.key].position, 1, "each agent's own line");
        add(tx, &member, "b", "api", &json!({}), 13)?;
        assert_eq!(protecting(tx, &member.key)?.unwrap().task, member.key);
        assert_eq!(on_cards(tx, &[&member.key])?[&member.key].task, member.id, "its own wait comes first");
        take_out(tx, &member.id)?;
        assert_eq!(protecting(tx, &member.key)?.unwrap().task, second.key, "the group still protects it");
        reorder(tx, "a", &[second.id.clone(), first.id.clone()])?;
        assert_eq!(on_cards(tx, &[&member.key])?[&member.key].position, 1);
        take_out(tx, &second.id)?;
        assert!(on_cards(tx, &[&second.key, &member.key, &child.key])?.is_empty(), "cancelling clears every member");
        Ok(())
    }).unwrap();
}

#[test]
fn ac158_an_older_wait_with_room_goes_first_one_blocked_by_its_project_does_not() {
    store("waits-ahead").with(|tx| {
        let (old, new) = (task(tx, "api"), task(tx, "api"));
        add(tx, &old, "a", "api", &json!({}), 10)?;
        assert_eq!(ahead(tx, "a", "api", &new.key)?, Some(old.id.clone()), "the slot is free: the older wait takes it, a new press goes behind");
        assert_eq!(ahead(tx, "a", "api", &old.key)?, None, "never behind itself");
        assert_eq!(ahead(tx, "b", "web", &new.key)?, None, "another agent and project");
        remove(tx, &old.key)?;
        // web holds one at once: a wait there is blocked by its project, and blocks no wait for that agent elsewhere
        holding(tx, "b", "web");
        let blocked = task(tx, "web");
        add(tx, &blocked, "a", "web", &json!({}), 10)?;
        assert_eq!(ahead(tx, "a", "api", &new.key)?, None, "{} waits for web's room, not for Smith's", blocked.id);
        Ok(())
    }).unwrap();
}

#[test]
fn ac158_a_wait_stays_while_full_and_goes_when_its_task_moved_was_archived_or_its_agent_left() {
    store("waits-due").with(|tx| {
        let t = task(tx, "api");
        add(tx, &t, "a", "api", &json!({"agent": "a"}), 10)?;
        let run = holding(tx, "a", "api");
        assert!(matches!(due(tx, &of_task(tx, &t.key)?.unwrap())?, Due::Stays), "Smith is at its limit");
        runs::change(tx, &run, |r| r.state = Phase::Closed)?;
        assert!(matches!(due(tx, &of_task(tx, &t.key)?.unwrap())?, Due::Press(id, body) if id == t.id && body["agent"] == "a"), "room: pressed again as kept");
        ops::move_to(tx, HUMAN, &t.id, "DONE", 20)?;
        assert!(matches!(due(tx, &of_task(tx, &t.key)?.unwrap())?, Due::Off(_, w) if w.contains("moved to DONE")));
        let u = task(tx, "api");
        add(tx, &u, "a", "api", &json!({}), 10)?;
        ops::archive(tx, HUMAN, &u.id, true, 30)?;
        assert!(matches!(due(tx, &of_task(tx, &u.key)?.unwrap())?, Due::Off(_, w) if w.contains("archived")));
        let v = task(tx, "api");
        add(tx, &v, "gone", "api", &json!({}), 10)?;
        assert!(matches!(due(tx, &of_task(tx, &v.key)?.unwrap())?, Due::Off(_, w) if w.contains("no longer in the pool")));
        Ok(())
    }).unwrap();
}

#[test]
fn ac169_a_queue_is_its_agents_waits_in_the_order_the_human_gave() {
    store("waits-queue").with(|tx| {
        let (one, two, three, other) = (task(tx, "api"), task(tx, "api"), task(tx, "api"), task(tx, "api"));
        add(tx, &one, "a", "api", &json!({}), 10)?;
        add(tx, &other, "b", "api", &json!({}), 10)?;
        add(tx, &two, "a", "api", &json!({}), 10)?;
        add(tx, &three, "a", "api", &json!({}), 10)?;
        let ids = |tx: &Transaction, agent: &str| -> Vec<String> { queues(tx).unwrap().remove(agent).unwrap_or_default().iter().map(|t| t["id"].as_str().unwrap().to_string()).collect() };
        assert_eq!(ids(tx, "a"), [one.id.clone(), two.id.clone(), three.id.clone()], "pressed in the same second: in the order they came");
        assert_eq!(queues(tx)?["a"][2], json!({"id": three.id, "title": "t in api", "project": "api", "also": []}));
        reorder(tx, "a", &[three.id.clone(), one.id.clone(), two.id.clone()])?;
        assert_eq!(ids(tx, "a"), [three.id.clone(), one.id.clone(), two.id.clone()]);
        assert_eq!(all(tx)?.iter().map(|w| w.task.as_str()).collect::<Vec<_>>(), [three.key.as_str(), other.key.as_str(), one.key.as_str(), two.key.as_str()], "another agent's wait keeps its place among them");
        assert_eq!(ahead(tx, "a", "api", &one.key)?, Some(three.id.clone()), "the one moved first goes first");
        assert_eq!(ahead(tx, "a", "api", &three.key)?, None);
        // a new press on a queued task keeps the place the human gave it
        add(tx, &three, "a", "api", &json!({"comment": "again"}), 99)?;
        assert_eq!(ids(tx, "a")[0], three.id);
        // a queue that changed meanwhile is not ordered by an older list
        assert_eq!(reorder(tx, "a", &[one.id.clone(), two.id.clone()]).unwrap_err(), "The queue changed meanwhile: look at it again.");
        assert_eq!(reorder(tx, "a", &[one.id.clone(), one.id.clone(), two.id.clone()]).unwrap_err(), "The queue changed meanwhile: look at it again.");
        assert_eq!(reorder(tx, "a", &[one.id.clone(), two.id.clone(), other.id.clone()]).unwrap_err(), "The queue changed meanwhile: look at it again.", "another agent's task");
        assert_eq!(ids(tx, "a"), [three.id.clone(), one.id.clone(), two.id.clone()], "nothing moved");
        // taken out: its wait alone, said on it; a task that started meanwhile is not touched
        take_out(tx, &one.id)?;
        assert_eq!(ids(tx, "a"), [three.id.clone(), two.id.clone()]);
        assert!(ops::history(tx, &one.id)?.to_string().contains("The human called off its wait for Smith."));
        let run = holding(tx, "b", "api");
        let started = runs::by_id(tx, &run)?.ids[0].clone();
        assert_eq!(take_out(tx, &started).unwrap_err(), format!("{started} waits no more: it started, or its wait was called off."));
        assert!(runs::by_id(tx, &run)?.holds_slot(), "its run goes on");
        // a press for several tasks names the others
        add(tx, &one, "a", "api", &json!({"also": [other.id]}), 10)?;
        assert_eq!(queues(tx)?["a"][2]["also"], json!([other.id]));
        assert_eq!(offer("Smith is at its limit of 1: API-1 is working."), "Smith is at its limit of 1: API-1 is working. Add it to the queue: it starts when there is room.");
        Ok(())
    }).unwrap();
}

fn other_agent() -> Who {
    Who { key: "tmux:local:t:%5".into(), agent: "codex".into(), title: "other".into(), project: Some("api".into()), dir: None, supervisor: false }
}

#[test]
fn ac158_ac169_a_queued_press_protects_each_member_until_it_is_cancelled_or_starts() {
    let (s, dir) = (store("waits-group-held"), crate::testutil::fresh_dir("waits-group-dir"));
    let (main, explicit, child) = s.with(|tx| {
        let (main, explicit, child) = (task(tx, "api"), task(tx, "api"), task(tx, "api"));
        crate::tasks::family::set_parent(tx, HUMAN, &child.id, Some(&explicit.id), 1)?;
        Ok((main, explicit, child))
    }).unwrap();
    let body = json!({"also": [explicit.id]});
    let queue = || s.with(|tx| add(tx, &main, "a", "api", &body, 2)).unwrap();
    queue();
    let who = other_agent();
    let call = |tool, args| tools::run(&s, &dir, &who, tool, &args).map(|(text, _)| text);
    let change = |t: &Task| [("move_task", json!({"id": t.id, "to": "IN PROGRESS"})), ("finish_task", json!({"id": t.id, "result": "A result."}))];
    for t in [&explicit, &child, &main] {
        let before = s.with(|tx| ops::history(tx, &t.id)).unwrap();
        for (tool, args) in change(t) {
            let got = call(tool, args);
            assert_eq!(got, Err(format!("{} waits in Smith's queue: it starts when there is room. You can read it and comment.", t.id)), "{tool}");
        }
        s.with(|tx| {
            assert_eq!(ops::get(tx, &t.id)?.status, "TODO");
            assert_eq!(ops::history(tx, &t.id)?, before, "refusals save no result or status change");
            assert_eq!(on_cards(tx, &[&t.key])?[&t.key].task, main.id);
            assert!(of_task(tx, &main.key)?.is_some());
            Ok(())
        }).unwrap();
        assert!(call("get_task", json!({"id": t.id})).is_ok());
        assert!(call("add_comment", json!({"id": t.id, "text": "A comment."})).is_ok());
    }
    s.with(|tx| take_out(tx, &main.id)).unwrap();
    for t in [&explicit, &child] {
        assert!(call("move_task", json!({"id": t.id, "to": "IN PROGRESS"})).is_ok());
    }

    queue();
    let pane = "tmux:local:t:%6";
    s.with(|tx| {
        remove(tx, &main.key)?;
        let run = runs::add(tx, &Run { agent_id: "a".into(), project: "api".into(), ..Default::default() },
            &[main.key.clone(), explicit.key.clone(), child.key.clone()], Default::default(), 3)?;
        runs::change(tx, &run, |r| { r.state = Phase::Working; r.pane_key = Some(pane.into()); }).map(drop)
    }).unwrap();
    for t in [&explicit, &child] {
        for (tool, args) in change(t) {
            assert_eq!(call(tool, args), Err(format!("{} runs in session %6 at a human's RUN. You can read it and comment.", t.id)), "{tool}");
        }
    }
    let own = Who { key: pane.into(), ..who };
    assert!(tools::run(&s, &dir, &own, "move_task", &json!({"id": child.id, "to": "FAILED"})).is_ok());
}

#[test]
fn ac158_ac169_saved_ids_use_the_same_members_as_run() {
    store("waits-also-ids").with(|tx| {
        let (main, explicit) = (task(tx, "api"), task(tx, "api"));
        crate::tasks::slugs::set_prefix(tx, "api", "NEW")?;
        let current = ops::get(tx, &explicit.key)?.id;
        for name in [&current, &format!("\u{2003}{}\t", explicit.id.to_ascii_lowercase()),
            &explicit.key[..8].to_ascii_uppercase(), &explicit.key] {
            assert_eq!(ops::get(tx, name)?.key, explicit.key);
            add(tx, &main, "a", "api", &json!({"also": [name]}), 2)?;
            assert_eq!(protecting(tx, &explicit.key)?.unwrap().task, main.key);
            assert_eq!(on_cards(tx, &[&explicit.key])?[&explicit.key].task, ops::get(tx, &main.key)?.id);
        }
        Ok(())
    }).unwrap();
}

/// Task `t` made a schedule's: a firing of a schedule on agent `agent` names it (terminum-schedules.md AC-162).
fn scheduled(tx: &Transaction, t: &Task, agent: &str) {
    let v = json!({"title": "Nightly", "project": t.project, "agent": agent, "cron": "0 3 * * *", "budget_minutes": 5});
    let (id, _) = crate::tasks::schedules::add(tx, HUMAN, &crate::tasks::schedules::Given::of(tx, &v, 1).unwrap(), 1).unwrap();
    let s = crate::tasks::schedules::by_id(tx, &id).unwrap();
    crate::tasks::schedules::fired(tx, &s, (1, 0), Some(&t.key), "started", None, 1).unwrap();
}

#[test]
fn ac169_a_scheduled_wait_leaves_its_subtask_free() {
    let (s, dir) = (store("waits-scheduled-subtask"), crate::testutil::fresh_dir("waits-scheduled-dir"));
    let child = s.with(|tx| {
        let (main, child) = (task(tx, "api"), task(tx, "api"));
        crate::tasks::family::set_parent(tx, HUMAN, &child.id, Some(&main.id), 1)?;
        scheduled(tx, &main, "a");
        add(tx, &main, "a", "api", &json!({}), 2)?;
        assert_eq!(protecting(tx, &main.key)?.unwrap().task, main.key);
        assert!(on_cards(tx, &[&child.key])?.is_empty());
        Ok(child)
    }).unwrap();
    assert!(tools::run(&s, &dir, &other_agent(), "move_task", &json!({"id": child.id, "to": "IN PROGRESS"})).is_ok());
}

#[test]
fn ac161_a_schedule_s_task_waits_ahead_of_the_agent_s_other_waits() {
    store("waits-scheduled").with(|tx| {
        tx.execute("UPDATE agents SET at_once = 3 WHERE id = 'a'", []).map_err(sql)?;
        let (ordinary, mine, newer) = (task(tx, "api"), task(tx, "api"), task(tx, "api"));
        scheduled(tx, &mine, "a");
        holding(tx, "a", "api");
        holding(tx, "a", "api");   // with the one kept slot, its at once of 3 is full for ordinary runs
        add(tx, &ordinary, "a", "api", &json!({}), 10)?;
        add(tx, &mine, "a", "api", &json!({"how": "schedule"}), 20)?;
        assert_eq!(all(tx)?.iter().map(|w| (w.task.as_str(), w.scheduled)).collect::<Vec<_>>(), [(mine.key.as_str(), true), (ordinary.key.as_str(), false)], "came later, stands first");
        assert_eq!(queues(tx)?["a"].iter().map(|w| w["id"].as_str().unwrap()).collect::<Vec<_>>(), [mine.id.as_str(), ordinary.id.as_str()]);
        assert_eq!((on_cards(tx, &[&mine.key])?[&mine.key].position, on_cards(tx, &[&ordinary.key])?[&ordinary.key].position), (1, 2), "the cards say the same places");
        assert_eq!(ahead(tx, "a", "api", &newer.key)?, Some(mine.id.clone()), "its kept slot is free: a newer press goes behind it");
        assert_eq!(reorder(tx, "a", &[ordinary.id.clone(), mine.id.clone()]).unwrap_err(), format!("{} is a scheduled run: it stays ahead of the other waits.", mine.id));
        assert_eq!(all(tx)?[0].task, mine.key, "a refused reorder moves nothing");
        reorder(tx, "a", &[mine.id.clone(), ordinary.id.clone()])?;
        Ok(())
    }).unwrap();
}

#[test]
fn ac161_a_schedule_s_task_waits_ahead_of_its_project_s_other_waits() {
    store("waits-scheduled-project").with(|tx| {
        let (ordinary, mine, newer) = (task(tx, "api"), task(tx, "api"), task(tx, "api"));
        scheduled(tx, &mine, "a");
        add(tx, &ordinary, "b", "api", &json!({}), 10)?;   // Jones has room
        assert_eq!(ahead(tx, "a", "api", &newer.key)?, Some(ordinary.id.clone()), "an older wait of its project with room goes first");
        assert_eq!(ahead(tx, "a", "api", &mine.key)?, None, "but never before a schedule's task");
        add(tx, &mine, "a", "api", &json!({"how": "schedule"}), 20)?;
        assert!(all(tx)?[0].scheduled);
        take_place(tx, &mine.key, 7)?;
        assert_eq!(of_task(tx, &mine.key)?.unwrap().pos, 7, "a newer run of a schedule takes the place of the wait it superseded");
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_a_wait_knows_whether_a_human_pressed_it() {
    store("waits-who").with(|tx| {
        let t = task(tx, "api");
        for (press, human) in [(json!({}), true), (json!({"how": "run"}), true), (json!({"how": "drop"}), true), (json!({"how": "schedule"}), false),
                               (json!({"how": "run", "manager": "m"}), false), (json!({"how": "schedule", "manager": "m"}), false), (json!({"how": "auto"}), false)] {
            add(tx, &t, "a", "api", &press, 1)?;
            assert_eq!(of_task(tx, &t.key)?.unwrap().by_human(), human, "{press}");
        }
        Ok(())
    }).unwrap();
}

#[test]
fn ac161_a_reorder_deals_each_kind_its_own_places() {
    store("waits-reorder-kinds").with(|tx| {
        tx.execute("UPDATE agents SET at_once = 3", []).map_err(sql)?;
        let (other, mine, plain, late, again) = (task(tx, "api"), task(tx, "api"), task(tx, "api"), task(tx, "api"), task(tx, "api"));
        add(tx, &other, "b", "api", &json!({}), 1)?;
        for (t, at) in [(&mine, 2), (&plain, 3), (&late, 4), (&again, 5)] {
            add(tx, t, "a", "api", &json!({}), at)?;
        }
        scheduled(tx, &mine, "a");
        scheduled(tx, &again, "a");
        let pos = |tx: &Transaction, t: &Task| of_task(tx, &t.key).unwrap().unwrap().pos;
        let before = (pos(tx, &other), [pos(tx, &mine), pos(tx, &again)], [pos(tx, &plain), pos(tx, &late)]);
        reorder(tx, "a", &[again.id.clone(), mine.id.clone(), late.id.clone(), plain.id.clone()])?;
        assert_eq!((pos(tx, &again), pos(tx, &mine)), (before.1[0], before.1[1]), "the schedules' tasks swap their own places");
        assert_eq!((pos(tx, &late), pos(tx, &plain)), (before.2[0], before.2[1]), "the others theirs");
        assert_eq!(pos(tx, &other), before.0, "another agent's wait keeps its place");
        Ok(())
    }).unwrap();
}

#[test]
fn ac161_a_schedule_s_waiting_task_closes_its_own_idle_session() {
    store("waits-own-idle").with(|tx| {
        let (old, new) = (task(tx, "api"), task(tx, "api"));
        scheduled(tx, &old, "a");
        let s = crate::tasks::schedules::of_task(tx, &old.key)?.unwrap();
        crate::tasks::schedules::fired(tx, &s, (2, 0), Some(&new.key), "started", None, 2)?;
        let id = runs::add(tx, &Run { agent_id: "a".into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: "api".into(), ..Default::default() }, std::slice::from_ref(&old.key), Default::default(), 1)?;
        runs::change(tx, &id, |r| { r.state = Phase::Working; r.pane_key = Some(format!("tmux:local:t:%{id}")); })?;
        ops::move_to(tx, HUMAN, &old.id, "DONE", 2)?;
        add(tx, &new, "a", "api", &json!({"how": "schedule"}), 3)?;
        let w = of_task(tx, &new.key)?.unwrap();
        assert!(matches!(due(tx, &w)?, Due::Stays), "its agent still in its turn: it waits");
        runs::change(tx, &id, |r| { r.turn_ended = true; r.closing_for = 60; })?;
        assert!(matches!(due(tx, &w)?, Due::Close(run, key) if run == id && key == s.key), "idle: closed for it");
        Ok(())
    }).unwrap();
}

#[test]
fn ac210_a_wait_for_other_tasks_goes_on_once_they_are_done_and_holds_no_place_meanwhile() {
    store("waits-after").with(|tx| {
        let (waiter, blocker, later) = (task(tx, "api"), task(tx, "api"), task(tx, "api"));
        crate::tasks::family::set_parent(tx, HUMAN, &blocker.id, Some(&waiter.id), 1)?;
        ops::move_to(tx, HUMAN, &blocker.id, "IN PROGRESS", 1)?;
        add_after(tx, &waiter, "a", "api", &json!({"after": [blocker.id]}), std::slice::from_ref(&blocker.key), 2)?;
        add(tx, &later, "a", "api", &json!({}), 3)?;
        let w = of_task(tx, &waiter.key)?.unwrap();
        assert!(matches!(due(tx, &w)?, Due::Stays), "its blocker works: it waits, with room or not");
        assert_eq!(ahead(tx, "a", "api", &later.key)?, None, "a wait for other tasks holds no place before a later press");
        assert_eq!(queues(tx)?["a"].iter().map(|v| v["id"].as_str().unwrap()).collect::<Vec<_>>(), [later.id.as_str()], "nor in the agent's queue");
        assert_eq!(on_cards(tx, &[&later.key])?[&later.key].position, 1, "nor a queue place");
        assert!(protecting(tx, &waiter.key)?.unwrap().for_tasks(), "the primary still waits for its dependencies");
        assert!(protecting(tx, &blocker.key)?.is_none(), "the blocker can be finished");
        assert!(on_cards(tx, &[&blocker.key])?.is_empty(), "the blocker is not a member");
        ops::move_to(tx, HUMAN, &blocker.id, "DONE", 4)?;
        assert!(matches!(due(tx, &w)?, Due::Press(id, body) if id == waiter.id && body["after"] == json!([blocker.id])), "DONE: pressed with its words' IDs");
        assert!(of_task(tx, &waiter.key)?.unwrap().after.is_empty(), "from then on a wait for room alone, in its place in the line");
        Ok(())
    }).unwrap();
}

#[test]
fn ac210_a_task_waited_for_that_will_not_be_done_asks_a_human() {
    for (how, end) in [("FAILED", "FAILED"), ("archived", "archived"), ("deleted", "deleted")] {
        store(&format!("waits-after-{how}")).with(|tx| {
            let (waiter, blocker) = (task(tx, "api"), task(tx, "api"));
            add_after(tx, &waiter, "a", "api", &json!({"after": [blocker.id]}), std::slice::from_ref(&blocker.key), 2)?;
            let mut w = of_task(tx, &waiter.key)?.unwrap();
            match how {
                "FAILED" => ops::move_to(tx, HUMAN, &blocker.id, "FAILED", 3)?,
                "archived" => { tx.execute("UPDATE tasks SET archived_at = 3 WHERE id = ?1", [&blocker.key]).map_err(sql)?; }
                _ => w.after = vec!["no-such-key".into()],
            }
            let Due::Asks(id, words) = due(tx, &w)? else { panic!("{how}: a human decides") };
            assert_eq!(id, waiter.id);
            let named = if how == "deleted" { "a task that was deleted".to_string() } else { blocker.id.clone() };
            assert!(words.starts_with(&format!("Why you are needed: {} waited for {named}, which is {end}", waiter.id)), "{words}");
            assert_eq!(crate::tasks::closeout::unshaped(&id, &words), None, "in AC-157's three parts");
            super::super::after::ask(tx, &waiter.key, &id, &words)?;
            assert_eq!(ops::get(tx, &waiter.id)?.status, "WAIT FOR HUMAN");
            assert!(of_task(tx, &waiter.key)?.is_none(), "its wait ends");
            let history = ops::history(tx, &waiter.id)?;
            assert!(history.as_array().unwrap().iter().any(|e| e["author"] == "dispatcher" && e["questions"] == true && e["text"] == words.as_str()), "{history}");
            Ok(())
        }).unwrap();
    }
}

#[test]
fn ac210_a_human_s_move_calls_off_a_wait_for_other_tasks() {
    store("waits-after-moved").with(|tx| {
        let (waiter, blocker) = (task(tx, "api"), task(tx, "api"));
        add_after(tx, &waiter, "a", "api", &json!({}), std::slice::from_ref(&blocker.key), 2)?;
        ops::move_to(tx, HUMAN, &waiter.id, "IN PROGRESS", 3)?;
        let w = of_task(tx, &waiter.key)?.unwrap();
        assert!(matches!(due(tx, &w)?, Due::Off(id, words) if id == waiter.id && words.contains("moved to IN PROGRESS")));
        add(tx, &waiter, "a", "api", &json!({}), 4)?;
        assert!(of_task(tx, &waiter.key)?.unwrap().after.is_empty(), "a human's RUN that waits for room replaces it");
        Ok(())
    }).unwrap();
}

#[test]
fn ac210_a_wait_for_other_tasks_called_off_lets_go_of_its_run_and_the_task_stays_in_todo() {
    store("waits-after-off").with(|tx| {
        let (waiter, blocker) = (task(tx, "api"), task(tx, "api"));
        let id = runs::add(tx, &Run { agent_id: "a".into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: "api".into(), ..Default::default() }, std::slice::from_ref(&waiter.key), Default::default(), 1)?;
        runs::change(tx, &id, |r| { r.state = Phase::Working; r.pane_key = Some("tmux:local:t:%9".into()); r.asked = true; })?;
        add_after(tx, &waiter, "a", "api", &json!({}), std::slice::from_ref(&blocker.key), 2)?;
        // the manager's stop or the human's Stop waiting: the wait goes, and its run with it, the window left alone
        call_off(tx, &waiter.key, "The project's manager called off its wait for Smith.")?;
        assert!(of_task(tx, &waiter.key)?.is_none());
        assert!(runs::by_id(tx, &id)?.is_over(), "no walk judges its window again: a close later fails nothing");
        assert_eq!(ops::get(tx, &waiter.id)?.status, "TODO", "it stays where it stood (AC-158)");
        assert!(ops::history(tx, &waiter.id)?.to_string().contains("Dispatcher let go of its session"));
        // a wait for room alone is no run's: calling it off touches none
        let (queued, other) = (task(tx, "api"), holding(tx, "b", "api"));
        add(tx, &queued, "a", "api", &json!({}), 3)?;
        tx.execute("INSERT INTO run_tasks (run, task) VALUES (?1, ?2)", (&other, &queued.key)).map_err(sql)?;
        call_off(tx, &queued.key, "The human called off its wait for Smith.")?;
        assert!(!runs::by_id(tx, &other)?.is_over(), "a slot's wait leaves every run as it was");
        Ok(())
    }).unwrap();
}
