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
        let s = schedule(tx, 1, every_15());
        assert!(matches!(decide(tx, &s, NOW + 60, false)?, Next::Nothing), "20:36: not due");
        let (id, fire) = made(decide(tx, &s, NOW + 10 * 60 + 14, false)?);
        let t = ops::get(tx, &id)?;
        assert_eq!((t.title.as_str(), t.severity.as_str(), t.status.as_str(), t.budget_seconds, t.by_agent), ("Nightly deps check", "P1", "TODO", Some(1800), false));
        assert_eq!(ops::text_of(tx, &id)?, "Check the dependencies.");
        let history = ops::history(tx, &id)?;
        let history = history.as_array().unwrap();
        assert!(history.iter().any(|e| e["text"].as_str().is_some_and(|w| w.starts_with("Dispatcher made it from schedule SCH-1, due ") && !w.contains("asleep"))), "{history:?}");
        assert_eq!(schedules::of_task(tx, &t.key)?.map(|o| o.key), Some("SCH-1".into()), "its task carries it");
        assert_eq!(schedules::unstarted(tx)?, vec![(fire, id.clone())], "made, its run not yet asked for: a restart's walk asks once");
        assert_eq!(schedules::by_id(tx, &s.id)?.last_due, NOW + 10 * 60);
        assert!(matches!(decide(tx, &s, NOW + 11 * 60, false)?, Next::Nothing), "not due again within the quarter");
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_its_last_task_open_skips_the_firing_and_says_so() {
    open("fire-skip").with(|tx| {
        let s = schedule(tx, 1, every_15());
        let (id, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        ops::move_to(tx, DISPATCHER, &id, "WAIT FOR HUMAN", NOW + 11 * 60)?;
        assert!(matches!(decide(tx, &s, NOW + 25 * 60, false)?, Next::Nothing));
        let listed = schedules::listed(tx, NOW + 25 * 60)?;
        assert_eq!(listed[0]["skipped"], 1);
        assert_eq!(listed[0]["last"]["status"], "WAIT FOR HUMAN");
        let fires = crate::tasks::schedulesapi::fires_of(tx, Some("SCH-1"), None, 10)?;
        assert_eq!((fires[0]["state"].as_str(), fires[0]["why"].as_str()), (Some("skipped"), Some(format!("Skipped: {id} is still open.").as_str())));
        ops::move_to(tx, DISPATCHER, &id, "DONE", NOW + 26 * 60)?;
        let (next, _) = made(decide(tx, &s, NOW + 40 * 60, false)?);
        assert_ne!(next, id, "its last one done: a new task");
        assert_eq!(schedules::listed(tx, NOW + 40 * 60)?[0]["skipped"], 0, "skips count since the last task made");
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_missed_firings_come_once_and_say_so() {
    open("fire-missed").with(|tx| {
        let s = schedule(tx, 1, every_15());
        let (id, _) = made(decide(tx, &s, NOW + 8 * 3600, false)?);
        let fires = crate::tasks::schedulesapi::fires_of(tx, None, None, 10)?;
        assert_eq!((fires.len(), fires[0]["missed"].as_i64()), (1, Some(31)));
        assert!(ops::history(tx, &id)?.as_array().unwrap().iter().any(|e| e["text"].as_str().is_some_and(|w| w.contains(": the Mac was asleep or terminumd was not running."))));
        Ok(())
    }).unwrap();
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
        let p = prompt(&s, "TERM-41", "Nightly deps check", "Check them.");
        assert!(p.starts_with("Task TERM-41: Nightly deps check. A scheduled run of terminum's schedule SCH-1 \"Nightly deps check\" (every day at 03:00), set by a human; no human started this run or watches it now.\n\nCheck them.\n\nYou have 30 minutes"), "{p}");
        assert!(p.contains("call finish_task with the result in this form, each on its own line:\nOutcome: ok, attention or failed\nSummary: one or two sentences\nDetails: what was checked or done\nFor the human: what they should do, or nothing"), "{p}");
        assert!(p.ends_with(ASK), "the words on asking a human");
        let (id, _) = made(decide(tx, &s, NOW + DAY, false)?);
        let t = ops::get(tx, &id)?;
        assert_eq!(super::super::press::prompt_of(tx, std::slice::from_ref(&t), None)?, prompt(&s, &id, &t.title, "Check them."), "RUN of its task goes as the schedule's run");
        Ok(())
    }).unwrap();
}

const DAY: i64 = 86_400;

#[test]
fn ac161_its_task_takes_the_kept_slot_and_others_have_the_rest() {
    open("fire-slot").with(|tx| {
        let s = schedule(tx, 1, every_15());
        let a = crate::tasks::agents::by_id(tx, "a")?.unwrap();
        let other = ops::create(tx, HUMAN, NewTask { title: "API-12".into(), ..Default::default() }, NOW)?;
        let full = pool::check_for(tx, &a, "default", &other.key, None)?.expect("its one slot is kept");
        assert_eq!(full.words, "Opus reviewer is at its limit of 1: SCH-1 (kept).");
        let (id, _) = made(decide(tx, &s, NOW + 10 * 60, false)?);
        let key = ops::key_of(tx, &id)?;
        assert_eq!(pool::check_for(tx, &a, "default", &key, None)?, None, "its own task takes the kept slot");
        schedules::pause(tx, &s.id, "paused", NOW)?;
        assert_eq!(pool::check_for(tx, &a, "default", &other.key, None)?, None, "paused: the slot is free");
        Ok(())
    }).unwrap();
}
