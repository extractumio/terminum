// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-169, AC-179: explicit queue promotion preserves the press and ordinary arrival fairness.

use super::*;
use crate::dispatcher::{pool, waits};

fn queue(tx: &Transaction, id: &str, agent: &str, body: Value, at: i64) {
    let t = ops::get(tx, id).unwrap();
    waits::add(tx, &t, agent, &t.project, &body, at).unwrap();
}

fn order(app: &crate::app::Shared) -> Vec<String> {
    app.tasks.with(|tx| waits::all(tx).map(|ws| ws.into_iter().map(|w| ops::get(tx, &w.task).unwrap().id).collect())).unwrap()
}

#[tokio::test]
async fn ac169_a_manager_moves_the_whole_press_preserving_metadata_and_other_agents_places() {
    let (app, _) = app("roles-queue");
    let (one, other, two, urgent, member, child, saved, occupied) = app.tasks.with(|tx| {
        let ids: Vec<String> = ["first", "other agent", "second", "urgent", "member", "child"].iter().map(|title| task_in(tx, "api", title)).collect();
        tx.execute("INSERT INTO agents (id, project, name, kind, command, color, at_once) VALUES ('a2', 'api', 'Second', 'claude', 'claude', 1, 1)", []).map_err(sql)?;
        tx.execute("UPDATE tasks SET parent = ?1 WHERE slug = ?2", (ops::key_of(tx, &ids[3])?, &ids[5])).map_err(sql)?;
        for (id, agent) in [(&ids[0], "a1"), (&ids[1], "a2"), (&ids[2], "a1"), (&ids[3], "a1")] {
            queue(tx, id, agent, if id == &ids[3] { json!({"comment": "saved answer", "also": [ids[4]]}) } else { json!({}) }, 10);
        }
        tx.execute("UPDATE tasks SET severity = 'P0' WHERE slug = ?1", [&ids[3]]).map_err(sql)?;
        let saved = waits::of_task(tx, &ops::key_of(tx, &ids[3])?)?.unwrap();
        let occupied = pool::Slots::read(tx)?.check_for(tx, &crate::tasks::agents::in_project(tx, "a1", "api")?.unwrap(), "api", None, None)?;
        Ok((ids[0].clone(), ids[1].clone(), ids[2].clone(), ids[3].clone(), ids[4].clone(), ids[5].clone(), saved, occupied))
    }).unwrap();
    assert_eq!(order(&app), [one.clone(), other.clone(), two.clone(), urgent.clone()], "severity edits do not sort the queue");
    let said = call(&app, BOSS, "promote_queued_task", json!({"id": urgent})).await.unwrap();
    assert!(said.contains("is now at the front of Opus's queue"), "{said}");
    assert_eq!(order(&app), [urgent.clone(), other.clone(), one.clone(), two.clone()], "only this agent's occupied positions are redistributed");
    let path: String = app.tasks.with(|tx| {
        let mut after = waits::of_task(tx, &saved.task)?.unwrap();
        assert!(after.pos < saved.pos);
        after.pos = saved.pos;
        assert_eq!(after, saved, "body, group, arrival time, project, status and agent survive unchanged");
        let marks = waits::on_cards(tx, &[&saved.task, &ops::key_of(tx, &member)?, &ops::key_of(tx, &child)?])?;
        for mark in marks.values() { assert_eq!((mark.position, mark.task.as_str()), (1, urgent.as_str())); }
        assert_eq!(marks.len(), 3, "the grouped and carried tasks share the primary's position");
        assert_eq!(pool::Slots::read(tx)?.check_for(tx, &crate::tasks::agents::in_project(tx, "a1", "api")?.unwrap(), "api", None, None)?, occupied, "promotion creates no run and frees no capacity");
        runs::change(tx, &runs::in_pane(tx, WORKER)?.unwrap().id, |r| r.state = Phase::Closed)?;
        assert_eq!(waits::ahead(tx, "a1", "api", &ops::key_of(tx, &one)?)?, Some(urgent.clone()), "at a vacancy, the promoted wait goes first");
        let history = ops::history(tx, &urgent)?.to_string();
        assert!(history.contains("Opus · manager moved it to the front of Opus's queue."), "{history}");
        tx.query_row("PRAGMA database_list", [], |r| r.get(2)).map_err(sql)
    }).unwrap();
    let listed = call(&app, BOSS, "list_runs", json!({})).await.unwrap();
    let listed: Value = serde_json::from_str(listed.split_once('\n').unwrap().1).unwrap();
    assert_eq!(listed["waiting_for_a_slot"].as_array().unwrap().iter().map(|w| w["task"].as_str().unwrap()).collect::<Vec<_>>(), [urgent.as_str(), other.as_str(), one.as_str(), two.as_str()]);
    let history = app.tasks.with(|tx| ops::history(tx, &urgent)).unwrap();
    assert!(call(&app, BOSS, "promote_queued_task", json!({"id": urgent})).await.unwrap().contains("is already"));
    assert_eq!(app.tasks.with(|tx| ops::history(tx, &urgent)).unwrap(), history, "idempotent promotion adds no event");
    let expected = order(&app);
    let reopened = crate::tasks::store::Store::open(path.into());
    reopened.with(|tx| {
        assert_eq!(waits::all(tx)?.iter().map(|w| ops::get(tx, &w.task).unwrap().id).collect::<Vec<_>>(), expected, "the persisted order survives reopening");
        Ok(())
    }).unwrap();
}

#[tokio::test]
async fn ac169_promotion_refuses_foreign_stale_and_non_primary_waits_without_changes() {
    let (app, working) = app("roles-queue-no");
    let (foreign, moved, primary, secondary) = app.tasks.with(|tx| {
        let (foreign, moved, primary, secondary) = (task_in(tx, "web", "foreign"), task_in(tx, "web", "moved"), task_in(tx, "api", "primary"), task_in(tx, "api", "secondary"));
        queue(tx, &foreign, "w1", json!({}), 1);
        queue(tx, &moved, "w1", json!({}), 2);
        queue(tx, &primary, "a1", json!({"also": [secondary]}), 3);
        tx.execute("UPDATE tasks SET project = 'api' WHERE slug = ?1", [&moved]).map_err(sql)?;
        Ok((foreign, moved, primary, secondary))
    }).unwrap();
    let before = order(&app);
    for (id, words) in [(&foreign, "is outside your project"), (&moved, "wait is outside your project"), (&working, "no waiting run of its own"), (&secondary, "no waiting run of its own")] {
        let why = call(&app, BOSS, "promote_queued_task", json!({"id": id})).await.unwrap_err();
        assert!(why.contains(words), "{id}: {why}");
        assert_eq!(order(&app), before);
    }
    assert!(call(&app, WORKER, "promote_queued_task", json!({"id": primary})).await.unwrap_err().contains("it is a worker's"));
    assert_eq!(in_pane(&app, BOSS, true, true, "promote_queued_task", &json!({"id": primary})).await.unwrap_err(), SUBAGENT);
    app.tasks.with(|tx| waits::remove(tx, &ops::key_of(tx, &primary)?).map(drop)).unwrap();
    assert!(call(&app, BOSS, "promote_queued_task", json!({"id": primary})).await.unwrap_err().contains("no waiting run of its own"));
}

#[tokio::test]
async fn ac161_a_promotion_stops_behind_a_schedule_s_task() {
    let (app, _) = app("roles-queue-scheduled");
    let (first, second, mine) = app.tasks.with(|tx| {
        let ids: Vec<String> = ["first", "second", "scheduled"].iter().map(|title| task_in(tx, "api", title)).collect();
        let v = json!({"title": "Nightly", "project": "api", "agent": "a1", "cron": "0 3 * * *", "budget_minutes": 5});
        let (sid, _) = crate::tasks::schedules::add(tx, crate::tasks::store::HUMAN, &crate::tasks::schedules::Given::of(tx, &v, 1)?, 1)?;
        let s = crate::tasks::schedules::by_id(tx, &sid)?;
        crate::tasks::schedules::fired(tx, &s, (1, 0), Some(&ops::key_of(tx, &ids[2])?), "started", None, 1)?;
        queue(tx, &ids[0], "a1", json!({}), 10);
        queue(tx, &ids[1], "a1", json!({}), 11);
        queue(tx, &ids[2], "a1", json!({"how": "schedule"}), 12);
        Ok((ids[0].clone(), ids[1].clone(), ids[2].clone()))
    }).unwrap();
    assert_eq!(order(&app), [mine.clone(), first.clone(), second.clone()], "a schedule's task waits ahead");
    let said = call(&app, BOSS, "promote_queued_task", json!({"id": second})).await.unwrap();
    assert!(said.contains("is now at the front of Opus's queue, behind its schedules' tasks"), "{said}");
    assert_eq!(order(&app), [mine.clone(), second.clone(), first.clone()]);
    assert!(call(&app, BOSS, "promote_queued_task", json!({"id": second})).await.unwrap().contains("is already"));
    assert!(call(&app, BOSS, "promote_queued_task", json!({"id": mine})).await.unwrap().contains("is already at the front of Opus's queue."));
}
