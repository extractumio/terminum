// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A schedule's task among the slots and the queue (terminum-schedules.md AC-161, AC-162, AC-182): where it starts, and
//! a newer run superseding a waiting one.
use super::tests::{every_15, made, open, schedule, working, NOW};
use super::*;
use crate::dispatcher::waits;
use crate::tasks::store::{Store, HUMAN};

fn agent_a(tx: &rusqlite::Transaction) -> crate::tasks::agents::Agent {
    agents::in_project(tx, "a", "default").unwrap().unwrap()
}

/// Task `id` waits for agent `a`, pressed with `press`; its firing started, so only the wait stands behind it.
fn waiting(tx: &rusqlite::Transaction, id: &str, press: serde_json::Value) -> i64 {
    let t = ops::get(tx, id).unwrap();
    waits::add(tx, &t, "a", "default", &press, NOW).unwrap();
    tx.execute("UPDATE schedule_fires SET state = 'started' WHERE task = ?1", [&t.key]).unwrap();
    waits::of_task(tx, &t.key).unwrap().unwrap().pos
}

#[test]
fn ac161_a_task_of_a_schedule_starts_in_a_kept_slot_then_an_ordinary_one() {
    open("fire-ordinary").with(|tx| {
        let one = schedule(tx, 3, every_15());
        let two = schedule(tx, 3, every_15());
        let a = agent_a(tx);
        let (first, _) = made(decide(tx, &one, NOW + 10 * 60, false)?);
        working(tx, &ops::get(tx, &first)?);
        let (second, _) = made(decide(tx, &two, NOW + 10 * 60, false)?);
        let key = ops::key_of(tx, &second)?;
        assert_eq!(pool::check_for(tx, &a, "default", Some(&key), None)?, None, "the one kept slot is taken: an ordinary one is free");
        let run = working(tx, &ops::get(tx, &second)?);
        let slots = pool::Slots::read(tx)?;
        assert_eq!((slots.slot_of(&run), slots.free_of(&a)), (None, 1), "counted as an ordinary run: 2 of 3 used");
        let other = ops::create(tx, HUMAN, NewTask { title: "API-12".into(), ..Default::default() }, NOW)?;
        working(tx, &other);
        let full = pool::check_for(tx, &a, "default", Some(&other.key), None)?.expect("full");
        assert_eq!(full.words, format!("Opus reviewer is at its limit of 3: {second} is working; {} is working; 1 kept for 2 schedules.", other.id));
        Ok(())
    }).unwrap();
}

#[test]
fn ac161_another_schedule_s_idle_session_is_never_closed_for_it() {
    open("fire-not-another").with(|tx| {
        let one = schedule(tx, 1, every_15());
        let two = schedule(tx, 1, every_15());   // two schedules keep one slot of its one
        let (first, _) = made(decide(tx, &one, NOW + 10 * 60, false)?);
        let run = working(tx, &ops::get(tx, &first)?);
        ops::move_to(tx, DISPATCHER, &first, "DONE", NOW + 11 * 60)?;
        runs::change(tx, &run, |r| { r.turn_ended = true; r.closing_for = 60; })?;
        assert!(pool::closable(tx, &runs::by_id(tx, &run)?)?);
        assert!(matches!(decide(tx, &two, NOW + 25 * 60, false)?, Next::Run(..)), "SCH-2's task is made and waits: SCH-1's idle session stays");
        assert!(matches!(decide(tx, &one, NOW + 25 * 60, false)?, Next::Close(r) if r == run), "SCH-1's own is closed for it");
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_a_newer_firing_supersedes_a_waiting_task_and_takes_its_place() {
    open("fire-supersede").with(|tx| {
        let s = schedule(tx, 1, every_15());
        let (old, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        let pos = waiting(tx, &old, json!({"agent": "a", "how": "schedule"}));
        let Next::Run(new, _, place) = decide(tx, &s, NOW + 25 * 60, false)? else { panic!("no task made") };
        assert_eq!(place, Some(pos), "its wait's place goes to the newer task");
        let t = ops::get(tx, &old)?;
        assert_eq!(t.status, "FAILED");
        assert!(waits::of_task(tx, &t.key)?.is_none());
        let words = format!("Superseded by {new}, a newer run of SCH-1, while it waited for room.");
        assert!(ops::history(tx, &old)?.as_array().unwrap().iter().any(|e| e["text"] == words), "{words}");
        // a manager's run_task of a task of it: superseded the same way; one waiting in WAIT FOR HUMAN stays there
        ops::move_to(tx, DISPATCHER, &new, "WAIT FOR HUMAN", NOW + 26 * 60)?;
        waiting(tx, &new, json!({"agent": "a", "how": "run", "manager": "m"}));
        let (newer, _) = made(decide(tx, &s, NOW + 40 * 60, false)?);
        assert_eq!(ops::get(tx, &new)?.status, "WAIT FOR HUMAN");
        assert!(waits::of_task(tx, &ops::key_of(tx, &new)?)?.is_none(), "{newer} took its place");
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_a_human_s_run_waiting_is_not_superseded() {
    open("fire-human-wait").with(|tx| {
        let s = schedule(tx, 1, every_15());
        let (old, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        ops::move_to(tx, DISPATCHER, &old, "WAIT FOR HUMAN", NOW + 11 * 60)?;
        waiting(tx, &old, json!({"agent": "a", "how": "run"}));
        assert!(matches!(decide(tx, &s, NOW + 25 * 60, false)?, Next::Skipped));
        let fires = crate::tasks::schedulesapi::fires_of(tx, Some("SCH-1"), None, 10)?;
        assert_eq!(fires[0]["why"].as_str(), Some(format!("Skipped: {old} waits for room, run by a human.").as_str()));
        assert!(waits::of_task(tx, &ops::key_of(tx, &old)?)?.is_some(), "it waits on");
        Ok(())
    }).unwrap();
}

/// Run now (AC-182): refused behind a human's RUN waiting; a wait of its own presses superseded.
#[tokio::test]
async fn ac182_run_now_supersedes_a_waiting_task_but_not_a_human_s() {
    let (root, mut app) = crate::testutil::app("fire-now-wait");
    std::sync::Arc::get_mut(&mut app).unwrap().tasks = std::sync::Arc::new(Store::open(root.join("kanban.sqlite")));
    let (sid, old) = store(&app, |tx| {
        let s = schedule(tx, 1, every_15());
        let (old, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        ops::move_to(tx, DISPATCHER, &old, "WAIT FOR HUMAN", NOW + 11 * 60)?;
        waiting(tx, &old, json!({"agent": "a", "how": "run"}));
        Ok((s.id, old))
    }).await.unwrap();
    assert_eq!(now(&app, &sid, HUMAN, None).await.unwrap_err(), format!("Not run: {old} waits for room, run by a human."));
    let task = old.clone();
    store(&app, move |tx| {
        ops::move_to(tx, DISPATCHER, &task, "IN PROGRESS", NOW + 12 * 60)?;
        waiting(tx, &task, json!({"agent": "a", "how": "schedule"}));
        Ok(())
    }).await.unwrap();
    let new = now(&app, &sid, HUMAN, None).await.unwrap();
    let (status, history) = store(&app, move |tx| Ok((ops::get(tx, &old)?.status, ops::history(tx, &old)?))).await.unwrap();
    assert_eq!(status, "FAILED");
    let words = format!("Superseded by {new}, a newer run of SCH-1, while it waited for room.");
    assert!(history.as_array().unwrap().iter().any(|e| e["text"] == words), "{history}");
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}
