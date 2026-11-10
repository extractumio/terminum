// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A RUN at a slot's limit waits for it (AC-158): kept in order, called off by what changed, ahead of newer presses.

use super::*;
use crate::dispatcher::runs::{self, Phase, Run};
use crate::tasks::ops::{NewTask, Task};
use crate::tasks::store::{Store, HUMAN};
use serde_json::json;

fn store(name: &str) -> Store {
    let s = Store::open(crate::testutil::fresh_dir(name).join("kanban.sqlite"));
    s.with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place, agent_limit) VALUES ('api', 'api', 0, '/srv', 0, NULL), ('web', 'web', 1, '/srv', 1, 1)", []).map_err(sql)?;
        tx.execute("INSERT INTO agents (id, name, kind, command, color, at_once) VALUES ('a', 'Smith', 'claude', 'claude', 0, 1), ('b', 'Jones', 'claude', 'claude', 1, 1)", []).map(drop).map_err(sql)
    }).unwrap();
    s
}

fn task(tx: &Transaction, project: &str) -> Task {
    ops::create(tx, HUMAN, NewTask { title: format!("t in {project}"), project: Some(project.into()), ..Default::default() }, 1).unwrap()
}

/// A run of agent `a` in `project` holding its slot.
fn holding(tx: &Transaction, a: &str, project: &str) -> String {
    let t = task(tx, project);
    let id = runs::add(tx, &Run { agent_id: a.into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: project.into(), ..Default::default() }, std::slice::from_ref(&t.key), 1).unwrap();
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
        assert_eq!(on_cards(tx, &[&one.key, &two.key])?.get(&one.key).map(String::as_str), Some("Jones"), "a card names the agent it waits for");
        assert!(remove(tx, &one.key)? && !remove(tx, &one.key)?);
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
        let (one, two, three, other) = (task(tx, "api"), task(tx, "api"), task(tx, "web"), task(tx, "api"));
        add(tx, &one, "a", "api", &json!({}), 10)?;
        add(tx, &other, "b", "api", &json!({}), 10)?;
        add(tx, &two, "a", "api", &json!({}), 10)?;
        add(tx, &three, "a", "web", &json!({}), 10)?;
        let ids = |tx: &Transaction, agent: &str| -> Vec<String> { queues(tx).unwrap().remove(agent).unwrap_or_default().iter().map(|t| t["id"].as_str().unwrap().to_string()).collect() };
        assert_eq!(ids(tx, "a"), [one.id.clone(), two.id.clone(), three.id.clone()], "pressed in the same second: in the order they came");
        assert_eq!(queues(tx)?["a"][2], json!({"id": three.id, "title": "t in web", "project": "web"}));
        reorder(tx, "a", &[three.id.clone(), one.id.clone(), two.id.clone()])?;
        assert_eq!(ids(tx, "a"), [three.id.clone(), one.id.clone(), two.id.clone()]);
        assert_eq!(all(tx)?.iter().map(|w| w.task.as_str()).collect::<Vec<_>>(), [three.key.as_str(), other.key.as_str(), one.key.as_str(), two.key.as_str()], "another agent's wait keeps its place among them");
        assert_eq!(ahead(tx, "a", "api", &one.key)?, Some(three.id.clone()), "the one moved first goes first");
        assert_eq!(ahead(tx, "a", "web", &three.key)?, None);
        // a new press on a queued task keeps the place the human gave it
        add(tx, &three, "a", "web", &json!({"comment": "again"}), 99)?;
        assert_eq!(ids(tx, "a")[0], three.id);
        // a queue that changed meanwhile is not ordered by an older list
        assert_eq!(reorder(tx, "a", &[one.id.clone(), two.id.clone()]).unwrap_err(), "The queue changed meanwhile: look at it again.");
        assert_eq!(reorder(tx, "a", &[one.id.clone(), one.id.clone(), two.id.clone()]).unwrap_err(), "The queue changed meanwhile: look at it again.");
        assert_eq!(reorder(tx, "a", &[one.id.clone(), two.id.clone(), other.id.clone()]).unwrap_err(), "The queue changed meanwhile: look at it again.", "another agent's task");
        assert_eq!(ids(tx, "a"), [three.id.clone(), one.id.clone(), two.id.clone()], "nothing moved");
        remove(tx, &one.key)?;
        assert_eq!(ids(tx, "a"), [three.id.clone(), two.id.clone()]);
        assert_eq!(offer("Smith is at its limit of 1: API-1 is working."), "Smith is at its limit of 1: API-1 is working. Add it to the queue: it starts when there is room.");
        Ok(())
    }).unwrap();
}
