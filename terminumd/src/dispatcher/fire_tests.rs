// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;
use crate::tasks::schedules::Given;
use crate::tasks::store::{Store, HUMAN};

/// 2026-11-09 20:35:00 UTC.
pub(super) const NOW: i64 = 1_794_256_500;

pub(super) fn open(name: &str) -> Store {
    Store::open(crate::testutil::fresh_dir(name).join("kanban.sqlite"))
}

/// Agent `a` with `at_once`, and a schedule of it every 15 minutes, its body `text`.
pub(super) fn schedule(tx: &rusqlite::Transaction, at_once: i64, v: serde_json::Value) -> Schedule {
    tx.execute("INSERT OR IGNORE INTO agents (id, project, name, kind, command, color, at_once) VALUES ('a', 'default', 'Opus reviewer', 'claude', 'claude', 0, ?1)", [at_once]).unwrap();
    let (id, _) = schedules::add(tx, HUMAN, &Given::of(tx, &v, NOW).unwrap(), NOW).unwrap();
    schedules::by_id(tx, &id).unwrap()
}

pub(super) fn every_15() -> serde_json::Value {
    json!({"title": "Nightly deps check", "body": "Check the dependencies.", "agent": "a", "cron": "*/15 * * * *", "budget_minutes": 30, "severity": "P1"})
}

pub(super) fn made(next: Next) -> (String, i64) {
    match next {
        Next::Run(task, fire, _) => (task, fire),
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
        let left = ops::create(tx, HUMAN, NewTask { title: "Debt: left".into(), by_agent: Some(10), made_in: Some("tmux:local:t:%7".into()), ..Default::default() }, NOW + 12 * 60)?;
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
    start(&app, &s, &task, fire, (None, None)).await;
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

/// terminum-reports.md AC-272: a schedule's task's result is a report, its task DONE; none supersedes another.
#[test]
fn ac272_a_schedule_s_task_s_result_is_a_report_and_none_supersedes_another() {
    let dir = crate::testutil::fresh_dir("fire-report");
    let store = Store::open(dir.join("kanban.sqlite"));
    let s = store.with(|tx| Ok(schedule(tx, 2, every_15()))).unwrap();
    // each firing's task made, linked to its pane and finished: DONE, its result a report in the Inbox (terminum-reports.md AC-272)
    let fire = |due: i64, pane: &str| {
        let key = format!("tmux:local:t:{pane}");
        let id = store.with(|tx| {
            let (id, _) = made(decide(tx, &s, due, false)?);
            ops::link(tx, DISPATCHER, &id, Some(&key), due)?;
            Ok(id)
        }).unwrap();
        let who = crate::tasks::tools::Who { key, agent: "claude".into(), title: "t".into(), project: None, dir: None, supervisor: false };
        let result = json!({"id": id, "result": "Outcome: ok\nSummary: s\nDetails: d\nFor the human: nothing"});
        let said = crate::tasks::tools::run(&store, &dir, &who, "finish_task", &result).unwrap().0;
        assert!(said.starts_with(&format!("{id} is DONE; its result went to the human's Inbox as R_")), "{said}");
        id
    };
    let older = fire(NOW + 10 * 60 + 14, "%7");
    let newer = fire(NOW + 25 * 60 + 14, "%8");
    store.with(|tx| {
        assert_eq!((ops::get(tx, &older)?.status, ops::get(tx, &newer)?.status), ("DONE".to_string(), "DONE".to_string()));
        assert_eq!(crate::reports::inbox::by_schedule(tx)?.get(&s.id).map(|r| r.2), Some(2), "every report stays, none superseded");
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_the_prompt_says_a_scheduled_run_and_the_result_s_form() {
    open("fire-prompt").with(|tx| {
        let s = schedule(tx, 1, json!({"title": "Nightly deps check", "body": "Check them.", "agent": "a", "cron": "0 3 * * *", "budget_minutes": 30}));
        let p = prompt(&s, "TERM-41", "Nightly deps check", "Check them.", false, Initiator::Due);
        assert!(p.starts_with("Task TERM-41: Nightly deps check. A scheduled run of terminum's schedule SCH-1 \"Nightly deps check\" (every day at 03:00), set by a human; no human started this run or watches it now.\n\nCheck them.\n\nYou have 30 minutes"), "{p}");
        assert!(p.contains("call execute_tool(\"finish_task\", {id, result}) with the result as a Markdown document: each section a \"## \" heading"), "{p}");
        assert!(p.contains("in this form:\n## Outcome\nok, attention or failed\n## Summary\none or two sentences\n## Details\nwhat was checked or done\n## For the human\nwhat they should do, or nothing"), "{p}");
        assert!(p.ends_with(ASK), "the words on asking a human");
        let due = s.when()?.next_after(NOW).unwrap();
        let (id, _) = made(decide(tx, &s, due, false)?);
        let t = ops::get(tx, &id)?;
        assert_eq!(super::super::press::prompt_of(tx, std::slice::from_ref(&t), None)?, prompt(&s, &id, &t.title, "Check them.", false, Initiator::Due), "the firing's RUN goes as the schedule's run");
        // AC-158: what the human saw holds the schedule's cadence as kept, not terminum's words for it
        let own = json!([[[t.id, t.title, "Check them.", t.by_agent, t.parent, t.budget_seconds]], [], ["SCH-1", "Nightly deps check", "0 3 * * *", null, 1800, false], "English"]).to_string();
        assert_eq!(super::super::press::facts(tx, std::slice::from_ref(&t), None)?.rev(), crate::auth::crypto::sha256_hex(own.as_bytes()));
        assert!(prompt(&s, &id, &t.title, "Check them.", true, Initiator::Human).starts_with(&format!("The human ran task {id} again: Nightly deps check. A task of terminum's schedule SCH-1")), "a human's RUN after says so");
        assert!(prompt(&s, &id, &t.title, &"x".repeat(20_001), false, Initiator::Due).contains(&format!("Its text, by execute_tool(\"get_task\", {{id}}) for {id}, is your assignment")), "a long text by get_task");
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_a_manager_s_schedule_and_runs_are_said_to_be_the_manager_s() {
    open("fire-manager").with(|tx| {
        let m = crate::tasks::users::agent_named(tx, "m".into(), "Opus · manager")?;
        let nightly = json!({"title": "Nightly deps check", "body": "Check them.", "agent": "a", "cron": "0 3 * * *", "budget_minutes": 30});
        let human = schedule(tx, 1, nightly.clone());
        assert!(!human.by_manager);
        let given = Given::of(tx, &nightly, NOW)?;
        schedules::change(tx, &m, &human.id, &given, NOW)?;
        let s = schedules::by_id(tx, &human.id)?;
        assert!(s.by_manager, "its last author is the manager");
        let schedule = "terminum's schedule SCH-1 \"Nightly deps check\" (every day at 03:00), set by the project's manager";
        // its firing
        let fired = prompt(&s, "TERM-41", "Nightly deps check", "Check them.", false, Initiator::Due);
        assert!(fired.starts_with(&format!("Task TERM-41: Nightly deps check. A scheduled run of {schedule}; no human started this run or watches it now.\n\nCheck them.")), "{fired}");
        // the manager's run_schedule
        let now = prompt(&s, "TERM-42", "Nightly deps check", "Check them.", false, Initiator::Manager);
        assert!(now.starts_with(&format!("Task TERM-42: Nightly deps check. A run of {schedule}, started now by the project's manager; no human watches it.\n\nCheck them.")), "{now}");
        // the manager's run_task of a task the schedule made
        let again = prompt(&s, "TERM-41", "Nightly deps check", "Check them.", true, Initiator::Manager);
        assert!(again.starts_with(&format!("The project's manager ran task TERM-41 again: Nightly deps check. A task of {schedule}.")), "{again}");
        for p in [&fired, &now, &again] {
            assert!(!p.contains("set by a human") && !p.contains("The human ran"), "a manager's words never stand as a human's: {p}");
        }
        // a human's RUN of a manager's schedule's task: the human ran it, the manager set it
        assert!(prompt(&s, "TERM-41", "Nightly deps check", "Check them.", true, Initiator::Human).starts_with(&format!("The human ran task TERM-41 again: Nightly deps check. A task of {schedule}.")));
        // a human who saves it again becomes its author
        schedules::change(tx, HUMAN, &s.id, &given, NOW)?;
        let s = schedules::by_id(tx, &s.id)?;
        assert!(prompt(&s, "TERM-43", "Nightly deps check", "Check them.", false, Initiator::Due).contains("(every day at 03:00), set by a human; no human started"));
        Ok(())
    }).unwrap();
}

#[test]
fn ac182_run_now_names_the_human_on_its_first_run() {
    open("fire-now-prompt").with(|tx| {
        let s = schedule(tx, 1, json!({"title": "Nightly deps check", "body": "Check them.", "agent": "a", "cron": "0 3 * * *", "budget_minutes": 30}));
        let (id, _, _) = super::made(tx, &s, (NOW, 0), "Alex ran schedule SCH-1 now.", Some(HUMAN), NOW)?;
        let t = ops::get(tx, &id)?;
        let expected = format!("Task {id}: Nightly deps check. A run of terminum's schedule SCH-1 \"Nightly deps check\" (every day at 03:00), set by a human, started now by a human.");
        // A first press can also be read back from a wait: its requester is on the task, not in the kept press.
        let press: super::super::api::Press = serde_json::from_value(json!({"agent": "a", "how": "schedule"})).unwrap();
        let sent = press.first_words(tx, &[t])?.prompt(None);
        assert_eq!(sent.lines().next(), Some(expected.as_str()));
        Ok(())
    }).unwrap();
}

#[test]
fn ac161_its_task_takes_the_kept_slot_and_others_have_the_rest() {
    open("fire-slot").with(|tx| {
        let s = schedule(tx, 1, every_15());
        let a = crate::tasks::agents::in_project(tx, "a", "default")?.unwrap();
        let other = ops::create(tx, HUMAN, NewTask { title: "API-12".into(), ..Default::default() }, NOW)?;
        let full = pool::check_for(tx, &a, "default", Some(&other.key), None)?.expect("its one slot is kept");
        assert_eq!(full.words, "Opus reviewer is at its limit of 1: 1 kept for 1 schedule.");
        let (id, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        let key = ops::key_of(tx, &id)?;
        assert_eq!(pool::check_for(tx, &a, "default", Some(&key), None)?, None, "its own task takes the kept slot");
        schedules::pause(tx, &s.id, "paused", NOW)?;
        assert_eq!(pool::check_for(tx, &a, "default", Some(&other.key), None)?, None, "paused: the slot is free");
        Ok(())
    }).unwrap();
}

/// A run of `task` on agent `a`, working.
pub(super) fn working(tx: &rusqlite::Transaction, task: &ops::Task) -> String {
    let id = runs::add(tx, &runs::Run { agent_id: "a".into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: task.project.clone(), ..Default::default() }, std::slice::from_ref(&task.key), Default::default(), NOW).unwrap();
    runs::change(tx, &id, |r| { r.state = runs::Phase::Working; r.pane_key = Some(format!("tmux:local:t:%{id}")); }).unwrap();
    id
}

#[test]
fn ac161_k_runs_stand_in_the_kept_slots_the_rest_are_ordinary() {
    open("fire-one-kept").with(|tx| {
        let s = schedule(tx, 1, every_15());
        let a = crate::tasks::agents::in_project(tx, "a", "default")?.unwrap();
        let (old, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        ops::move_to(tx, DISPATCHER, &old, "FAILED", NOW + 11 * 60)?;
        let (new, _) = made(decide(tx, &s, NOW + 25 * 60, false)?);
        let (old_key, new_key) = (ops::key_of(tx, &old)?, ops::key_of(tx, &new)?);
        let run = working(tx, &ops::get(tx, &new)?);
        let full = pool::check_for(tx, &a, "default", Some(&old_key), None)?.expect("the kept slot holds the new run, and no ordinary slot is left");
        assert_eq!(full.words, "Opus reviewer is at its limit of 1: 1 kept for 1 schedule.");
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
fn ac161_its_own_idle_session_is_closed_only_with_no_room_and_a_busy_one_never() {
    open("fire-older-idle").with(|tx| {
        let s = schedule(tx, 1, every_15());
        let (old, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        ops::move_to(tx, DISPATCHER, &old, "DONE", NOW + 11 * 60)?;
        let (new, _) = made(decide(tx, &s, NOW + 25 * 60, false)?);
        ops::move_to(tx, DISPATCHER, &new, "DONE", NOW + 26 * 60)?;   // its session closed: no run of it holds a slot
        // a human ran the older one again; it finished, its session stays open in the kept slot
        let run = working(tx, &ops::get(tx, &old)?);
        let (busy, _) = made(decide(tx, &s, NOW + 40 * 60, false)?);
        assert_eq!(runs::by_id(tx, &run)?.state, runs::Phase::Working, "a busy session is never closed: its task is made, and waits for room");
        ops::move_to(tx, DISPATCHER, &busy, "DONE", NOW + 41 * 60)?;
        runs::change(tx, &run, |r| { r.turn_ended = true; r.closing_for = 60; })?;
        match decide(tx, &s, NOW + 55 * 60, false)? {
            Next::Close(r) => assert_eq!(r, run, "with no room, the older task's idle session is closed for it"),
            _ => panic!("its idle session was not closed"),
        }
        let (closed, _) = made(decide(tx, &s, NOW + 55 * 60, true)?);
        ops::move_to(tx, DISPATCHER, &closed, "DONE", NOW + 56 * 60)?;
        tx.execute("UPDATE agents SET at_once = 2 WHERE id = 'a'", []).map_err(ops::sql)?;
        let room = made(decide(tx, &s, NOW + 70 * 60, false)?);
        assert_eq!(runs::by_id(tx, &run)?.state, runs::Phase::Working, "with room, nothing is closed: {room:?}");
        Ok(())
    }).unwrap();
}

#[test]
fn ac164_out_keeps_agent_comments_and_failure_reasons_past_session_bookkeeping() {
    open("fire-out").with(|tx| {
        let s = schedule(tx, 1, every_15());
        let (id, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        let key = ops::key_of(tx, &id)?;
        let agent = ops::pool_user(tx, "a", "Opus reviewer")?;
        let out = |tx: &rusqlite::Transaction| -> Result<serde_json::Value, String> {
            Ok(crate::tasks::schedulesapi::fires_of(tx, Some("SCH-1"), None, 10)?[0]["result"].clone())
        };
        assert!(out(tx)?.is_null(), "no agent comment or failure yet");
        ops::move_to(tx, HUMAN, &id, "FAILED", NOW + 10 * 60 + 1)?;
        assert!(out(tx)?.is_null(), "the schedule's creation reason is not a failure result");
        ops::move_to(tx, HUMAN, &id, "IN PROGRESS", NOW + 10 * 60 + 2)?;
        ops::comment(tx, &agent, &id, "Checking dependencies.", NOW + 11 * 60)?;
        assert_eq!(out(tx)?, "Checking dependencies.", "the agent's progress is readable");
        let result = "Outcome: ok\nSummary: All done.\nDetails: None\nFor the human: None";
        ops::comment(tx, &agent, &id, result, NOW + 12 * 60)?;
        ops::move_to(tx, &agent, &id, "DONE", NOW + 12 * 60)?;
        ops::log(tx, &key, DISPATCHER, "told", "Session ended: closed for SCH-1.", None, NOW + 13 * 60)?;
        ops::comment(tx, HUMAN, &id, "A later human comment", NOW + 14 * 60)?;
        assert_eq!(out(tx)?, result, "later lifecycle and human events leave the result");
        // A subsequent RUN fails: the failure reason, not an earlier result or a later close, is Out.
        ops::move_to(tx, HUMAN, &id, "IN PROGRESS", NOW + 15 * 60)?;
        ops::move_to(tx, DISPATCHER, &id, "FAILED", NOW + 16 * 60)?;
        ops::log(tx, &key, DISPATCHER, "why", "Its budget ended.", None, NOW + 16 * 60)?;
        ops::log(tx, &key, DISPATCHER, "told", "Session ended: closed 5 minutes after FAILED.", None, NOW + 17 * 60)?;
        assert_eq!(out(tx)?, "Its budget ended.", "the failure reason survives its close");
        Ok(())
    }).unwrap();
}
