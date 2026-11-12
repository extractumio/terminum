// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;
use crate::tasks::schedules::Given;
use crate::tasks::store::{Store, HUMAN};

/// 2026-11-09 20:35:00 UTC.
const NOW: i64 = 1_794_256_500;

fn open(name: &str) -> Store {
    Store::open(crate::testutil::fresh_dir(name).join("kanban.sqlite"))
}

/// Agent `a` with `at_once`, and a schedule of it every 15 minutes, its body `text`.
fn schedule(tx: &rusqlite::Transaction, at_once: i64, v: serde_json::Value) -> Schedule {
    tx.execute("INSERT OR IGNORE INTO agents (id, name, kind, command, color, at_once) VALUES ('a', 'Opus reviewer', 'claude', 'claude', 0, ?1)", [at_once]).unwrap();
    let (id, _) = schedules::add(tx, HUMAN, &Given::of(tx, &v, NOW).unwrap(), NOW).unwrap();
    schedules::by_id(tx, &id).unwrap()
}

fn every_15() -> serde_json::Value {
    json!({"title": "Nightly deps check", "body": "Check the dependencies.", "agent": "a", "cron": "*/15 * * * *", "budget_minutes": 30, "severity": "P1"})
}

fn made(next: Next) -> (String, i64) {
    match next {
        Next::Run(task, fire) => (task, fire),
        _ => panic!("no task made"),
    }
}

#[test]
fn ac162_a_due_schedule_makes_its_task_as_it_is_then() {
    open("fire-made").with(|tx| {
        let human = ops::create(tx, HUMAN, NewTask { title: "A human's".into(), ..Default::default() }, NOW)?;
        let s = schedule(tx, 1, every_15());
        assert!(matches!(decide(tx, &s, NOW + 60, false)?, Next::Nothing), "20:36: not due");
        let (id, fire) = made(decide(tx, &s, NOW + 10 * 60 + 14, false)?);
        let t = ops::get(tx, &id)?;
        assert_eq!((t.title.as_str(), t.severity.as_str(), t.status.as_str(), t.budget_seconds, t.by_agent), ("Nightly deps check", "P1", "IN PROGRESS", Some(1800), false), "made IN PROGRESS, never in TODO");
        let (letters, number) = human.id.split_once('-').unwrap();
        assert_eq!(id, format!("S_{letters}-{}", number.parse::<i64>().unwrap() + 1), "its ID says a schedule made it, in its project's count");
        assert_eq!(ops::get(tx, &id.to_lowercase())?.key, t.key, "found as any ID is");
        assert_eq!(ops::move_to(tx, HUMAN, &id, "TODO", NOW + 11 * 60).unwrap_err(), format!("{id} is a run of schedule SCH-1: it does not go to TODO."));
        assert_eq!(ops::text_of(tx, &id)?, "Check the dependencies.");
        let history = ops::history(tx, &id)?;
        let history = history.as_array().unwrap();
        assert!(history.iter().any(|e| e["text"].as_str().is_some_and(|w| w.starts_with("Dispatcher made it from schedule SCH-1, due "))), "{history:?}");
        assert!(!history.iter().any(|e| e["text"].as_str().is_some_and(|w| w.contains("TODO"))), "{history:?}");
        assert_eq!(schedules::of_task(tx, &t.key)?.map(|o| o.key), Some("SCH-1".into()), "its task carries it");
        assert_eq!(schedules::unstarted(tx)?, vec![(fire, id.clone(), NOW + 10 * 60 + 14)], "made, its run not yet asked for: a restart's walk asks once");
        // AC-166: its own ID in its result is a task the board holds, and what its session made counts from its making
        assert!(crate::tasks::closeout::unknown_ids(tx, &format!("See {id}, and {}.", human.id))?.is_empty());
        ops::link(tx, DISPATCHER, &id, Some("tmux:local:t:%7"), NOW + 11 * 60)?;
        let left = ops::create(tx, HUMAN, NewTask { title: "Debt: left".into(), by_agent: true, made_in: Some("tmux:local:t:%7".into()), ..Default::default() }, NOW + 12 * 60)?;
        assert_eq!(crate::tasks::closeout::made_line(tx, &ops::get(tx, &id)?)?, Some(format!("Tasks made in its session: {}.", left.id)));
        assert_eq!(schedules::by_id(tx, &s.id)?.last_due, NOW + 10 * 60);
        assert!(matches!(decide(tx, &s, NOW + 11 * 60, false)?, Next::Nothing), "not due again within the quarter");
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_a_task_of_it_in_progress_skips_the_firing_and_one_waiting_for_a_human_does_not() {
    open("fire-skip").with(|tx| {
        let s = schedule(tx, 2, every_15());
        let (id, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        assert!(matches!(decide(tx, &s, NOW + 25 * 60, false)?, Next::Skipped), "IN PROGRESS: it runs, or waits for room");
        let listed = schedules::listed(tx, NOW + 25 * 60)?;
        assert_eq!(listed[0]["skipped"], 1);
        assert_eq!(listed[0]["last"]["status"], "IN PROGRESS");
        let fires = crate::tasks::schedulesapi::fires_of(tx, Some("SCH-1"), None, 10)?;
        assert_eq!((fires[0]["state"].as_str(), fires[0]["why"].as_str()), (Some("skipped"), Some(format!("Skipped: {id} is still in progress.").as_str())));
        assert!(matches!(decide(tx, &s, NOW + 40 * 60, false)?, Next::Skipped));
        let fires = crate::tasks::schedulesapi::fires_of(tx, Some("SCH-1"), None, 10)?;
        assert_eq!((fires.len(), fires[0]["missed"].as_i64(), schedules::listed(tx, NOW + 40 * 60)?[0]["skipped"].as_i64()), (2, Some(1), Some(2)), "skips one after another are one row, each counted");
        ops::move_to(tx, DISPATCHER, &id, "WAIT FOR HUMAN", NOW + 41 * 60)?;
        let (next, _) = made(decide(tx, &s, NOW + 55 * 60, false)?);
        assert_ne!(next, id, "its last one waits for a human: the schedule goes on");
        assert_eq!(schedules::listed(tx, NOW + 55 * 60)?[0]["skipped"], 0, "skips count since the last task made");
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_a_firing_not_run_in_its_time_is_missed_and_never_run_later() {
    open("fire-missed").with(|tx| {
        let s = schedule(tx, 1, every_15());
        // eight hours asleep, awake five minutes after the last due time
        assert!(matches!(decide(tx, &s, NOW + 8 * 3600, false)?, Next::Skipped));
        let fires = crate::tasks::schedulesapi::fires_of(tx, None, None, 10)?;
        assert_eq!((fires.len(), fires[0]["missed"].as_i64(), fires[0]["task"].as_str(), fires[0]["state"].as_str()), (1, Some(31), None, Some("skipped")));
        assert_eq!(schedules::listed(tx, NOW + 8 * 3600)?[0]["skipped"], 32, "each firing that passed is counted");
        let why = fires[0]["why"].as_str().unwrap();
        assert!(why.starts_with("Missed, due ") && why.ends_with(": the Mac was asleep or terminumd was not running."), "{why}");
        assert!(matches!(decide(tx, &s, NOW + 8 * 3600 + 60, false)?, Next::Nothing), "nothing of it is run later");
        made(decide(tx, &s, NOW + 8 * 3600 + 10 * 60 + LATE, false)?);
        let fires = crate::tasks::schedulesapi::fires_of(tx, None, None, 10)?;
        assert_eq!((fires.len(), fires[0]["missed"].as_i64()), (2, Some(0)), "the next by its cadence, in its time");
        Ok(())
    }).unwrap();
    open("fire-missed-awake").with(|tx| {
        let s = schedule(tx, 1, every_15());
        made(decide(tx, &s, NOW + 8 * 3600 + 10 * 60, false)?);
        assert_eq!(crate::tasks::schedulesapi::fires_of(tx, None, None, 10)?[0]["missed"], 32, "awake at a due time: that one runs, those before it are counted");
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_a_once_that_missed_its_time_pauses_itself() {
    open("fire-once-missed").with(|tx| {
        let s = schedule(tx, 1, json!({"title": "Once", "agent": "a", "once_at": NOW + 3600, "budget_minutes": 5}));
        assert!(matches!(decide(tx, &s, NOW + 2 * 3600, false)?, Next::Skipped));
        let after = schedules::by_id(tx, &s.id)?;
        assert_eq!((after.paused, after.paused_why.as_deref()), (true, Some("missed its time")));
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_a_wait_for_room_called_off_fails_a_task_that_never_ran() {
    open("fire-wait-off").with(|tx| {
        let s = schedule(tx, 1, every_15());
        let (id, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        let t = ops::get(tx, &id)?;
        super::super::waits::add(tx, &t, "a", "default", &json!({}), NOW)?;
        super::super::waits::call_off(tx, &t.key, "Its wait was called off: its agent is no longer in the pool. Run it again.")?;
        assert_eq!(ops::get(tx, &id)?.status, "FAILED", "nothing works on it");
        // one left IN PROGRESS with no run, no wait and its start asked for stops no firing
        let (left, fire) = made(decide(tx, &s, NOW + 25 * 60, false)?);
        schedules::fire_started(tx, fire, "its prompt")?;
        assert_eq!(ops::get(tx, &left)?.status, "IN PROGRESS");
        let (next, _) = made(decide(tx, &s, NOW + 40 * 60, false)?);
        assert_ne!(next, left);
        Ok(())
    }).unwrap();
}

/// A project with no folder starts nothing: the task fails, the schedule goes on.
#[tokio::test]
async fn ac162_a_start_that_fails_fails_its_task_and_the_next_firing_tries_again() {
    let (root, mut app) = crate::testutil::app("fire-nostart");
    std::sync::Arc::get_mut(&mut app).unwrap().tasks = std::sync::Arc::new(Store::open(root.join("kanban.sqlite")));
    let (s, task, fire) = store(&app, |tx| {
        let s = schedule(tx, 1, every_15());
        let (task, fire) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        Ok((s, task, fire))
    }).await.unwrap();
    start(&app, &s, &task, fire).await;
    let (sid, id) = (s.id.clone(), task.clone());
    let (status, after, listed, said) = store(&app, move |tx| {
        let listed = schedules::listed(tx, NOW + 11 * 60)?;
        let fires = crate::tasks::schedulesapi::fires_of(tx, None, None, 10)?;
        let again = matches!(decide(tx, &schedules::by_id(tx, &sid)?, NOW + 25 * 60, false)?, Next::Run(..)) && ops::get(tx, &id)?.archived_at.is_some();
        Ok((ops::get(tx, &id)?.status, schedules::by_id(tx, &sid)?, listed, (fires[0]["state"].clone(), fires[0]["why"].clone(), again)))
    }).await.unwrap();
    assert_eq!(status, "FAILED");
    assert!(!after.paused, "the schedule is not paused");
    let why = listed[0]["failed_why"].as_str().unwrap_or_default().to_string();
    assert!(why.starts_with("It did not start: "), "{listed:?}");
    assert_eq!((said.0.as_str(), said.1.as_str(), said.2), (Some("failed"), Some(why.as_str()), true), "the firing says why; the next one makes a task, and the one that never started leaves the board");
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ac162_once_pauses_itself_after_it_fires() {
    open("fire-once").with(|tx| {
        let s = schedule(tx, 1, json!({"title": "Once", "agent": "a", "once_at": NOW + 3600, "budget_minutes": 5}));
        assert!(schedules::kept_on(tx, "a", None)?.len() == 1);
        made(decide(tx, &s, NOW + 3600, false)?);
        let after = schedules::by_id(tx, &s.id)?;
        assert_eq!((after.paused, after.paused_why.as_deref()), (true, Some("ran once")));
        assert!(schedules::kept_on(tx, "a", None)?.is_empty(), "its slot is free");
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_the_prompt_says_a_scheduled_run_and_the_result_s_form() {
    open("fire-prompt").with(|tx| {
        let s = schedule(tx, 1, json!({"title": "Nightly deps check", "body": "Check them.", "agent": "a", "cron": "0 3 * * *", "budget_minutes": 30}));
        let p = prompt(&s, "TERM-41", "Nightly deps check", "Check them.", false);
        assert!(p.starts_with("Task TERM-41: Nightly deps check. A scheduled run of terminum's schedule SCH-1 \"Nightly deps check\" (every day at 03:00), set by a human; no human started this run or watches it now.\n\nCheck them.\n\nYou have 30 minutes"), "{p}");
        assert!(p.contains("call execute_tool(\"finish_task\", {id, result}) with the result in this form, each on its own line:\nOutcome: ok, attention or failed\nSummary: one or two sentences\nDetails: what was checked or done\nFor the human: what they should do, or nothing"), "{p}");
        assert!(p.ends_with(ASK), "the words on asking a human");
        let due = s.when()?.next_after(NOW).unwrap();
        let (id, _) = made(decide(tx, &s, due, false)?);
        let t = ops::get(tx, &id)?;
        assert_eq!(super::super::press::prompt_of(tx, std::slice::from_ref(&t), None)?, prompt(&s, &id, &t.title, "Check them.", false), "the firing's RUN goes as the schedule's run");
        // AC-158: what the human saw holds the schedule's cadence as kept, not terminum's words for it
        let own = json!([[[t.id, t.title, "Check them.", t.by_agent, t.parent, t.budget_seconds]], [], ["SCH-1", "Nightly deps check", "0 3 * * *", null, 1800, false], "English"]).to_string();
        assert_eq!(super::super::press::facts(tx, std::slice::from_ref(&t))?.rev(), crate::auth::crypto::sha256_hex(own.as_bytes()));
        assert!(prompt(&s, &id, &t.title, "Check them.", true).starts_with(&format!("The human ran task {id} again: Nightly deps check. A task of terminum's schedule SCH-1")), "a human's RUN after says so");
        assert!(prompt(&s, &id, &t.title, &"x".repeat(20_001), false).contains(&format!("Its text, by execute_tool(\"get_task\", {{id}}) for {id}, is your assignment")), "a long text by get_task");
        Ok(())
    }).unwrap();
}

#[test]
fn ac161_its_task_takes_the_kept_slot_and_others_have_the_rest() {
    open("fire-slot").with(|tx| {
        let s = schedule(tx, 1, every_15());
        let a = crate::tasks::agents::by_id(tx, "a")?.unwrap();
        let other = ops::create(tx, HUMAN, NewTask { title: "API-12".into(), ..Default::default() }, NOW)?;
        let full = pool::check_for(tx, &a, "default", Some(&other.key), None)?.expect("its one slot is kept");
        assert_eq!(full.words, "Opus reviewer is at its limit of 1: SCH-1 (kept).");
        let (id, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        let key = ops::key_of(tx, &id)?;
        assert_eq!(pool::check_for(tx, &a, "default", Some(&key), None)?, None, "its own task takes the kept slot");
        schedules::pause(tx, &s.id, "paused", NOW)?;
        assert_eq!(pool::check_for(tx, &a, "default", Some(&other.key), None)?, None, "paused: the slot is free");
        Ok(())
    }).unwrap();
}

/// A run of `task` on agent `a`, working.
fn working(tx: &rusqlite::Transaction, task: &ops::Task) -> String {
    let id = runs::add(tx, &runs::Run { agent_id: "a".into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: task.project.clone(), ..Default::default() }, std::slice::from_ref(&task.key), Default::default(), NOW).unwrap();
    runs::change(tx, &id, |r| { r.state = runs::Phase::Working; r.pane_key = Some(format!("tmux:local:t:%{id}")); }).unwrap();
    id
}

#[test]
fn ac161_one_run_stands_in_a_kept_slot() {
    open("fire-one-kept").with(|tx| {
        let s = schedule(tx, 1, every_15());
        let a = crate::tasks::agents::by_id(tx, "a")?.unwrap();
        let (old, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        ops::move_to(tx, DISPATCHER, &old, "FAILED", NOW + 11 * 60)?;
        let (new, _) = made(decide(tx, &s, NOW + 25 * 60, false)?);
        let (old_key, new_key) = (ops::key_of(tx, &old)?, ops::key_of(tx, &new)?);
        let run = working(tx, &ops::get(tx, &new)?);
        let full = pool::check_for(tx, &a, "default", Some(&old_key), None)?.expect("the kept slot holds the new run");
        assert_eq!(full.words, "Opus reviewer is at its limit of 1: SCH-1 (kept).");
        assert_eq!(pool::check_for(tx, &a, "default", Some(&new_key), Some(&run))?, None, "its own run goes on from itself");
        assert!(pool::check_for(tx, &a, "default", None, None)?.is_some(), "a run of several tasks takes no kept slot");
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_any_task_of_it_in_progress_skips_the_firing() {
    open("fire-any-open").with(|tx| {
        let s = schedule(tx, 2, every_15());
        let (old, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        ops::move_to(tx, DISPATCHER, &old, "FAILED", NOW + 11 * 60)?;
        let (new, _) = made(decide(tx, &s, NOW + 25 * 60, false)?);
        ops::move_to(tx, DISPATCHER, &new, "DONE", NOW + 26 * 60)?;
        ops::move_to(tx, DISPATCHER, &old, "IN PROGRESS", NOW + 27 * 60)?;   // a human ran the older again
        assert!(matches!(decide(tx, &s, NOW + 40 * 60, false)?, Next::Skipped), "the older one runs again: skipped");
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_an_older_task_s_idle_session_in_the_kept_slot_is_closed_for_the_firing() {
    open("fire-older-idle").with(|tx| {
        let s = schedule(tx, 1, every_15());
        let (old, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        ops::move_to(tx, DISPATCHER, &old, "DONE", NOW + 11 * 60)?;
        let (new, _) = made(decide(tx, &s, NOW + 25 * 60, false)?);
        ops::move_to(tx, DISPATCHER, &new, "DONE", NOW + 26 * 60)?;   // its session closed: no run of it holds a slot
        // a human ran the older one again; it finished, its session stays open in the kept slot
        let run = working(tx, &ops::get(tx, &old)?);
        assert!(matches!(decide(tx, &s, NOW + 40 * 60, false)?, Next::Skipped), "working: skipped");
        let fires = crate::tasks::schedulesapi::fires_of(tx, Some("SCH-1"), None, 10)?;
        assert_eq!(fires[0]["why"].as_str(), Some(format!("Skipped: {old}'s session is still busy.").as_str()), "named by the task in the slot, not the last one");
        runs::change(tx, &run, |r| { r.turn_ended = true; r.closing_for = 60; })?;
        match decide(tx, &s, NOW + 55 * 60, false)? {
            Next::Close(r) => assert_eq!(r, run, "the older task's idle session is closed for the slot"),
            _ => panic!("its idle session was not closed"),
        }
        assert!(matches!(decide(tx, &s, NOW + 55 * 60, true)?, Next::Skipped), "closed once and still standing there: skipped, nothing closed again");
        runs::change(tx, &run, |r| r.state = runs::Phase::Gone)?;
        made(decide(tx, &s, NOW + 70 * 60, false)?);
        Ok(())
    }).unwrap();
}
