// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;
use crate::tasks::store::{Store, HUMAN};

/// 2026-11-09 20:35:00 UTC.
const NOW: i64 = 1_794_256_500;

fn open(name: &str) -> Store {
    Store::open(crate::testutil::fresh_dir(name).join("kanban.sqlite"))
}

fn agent(tx: &Transaction, id: &str, at_once: i64) {
    tx.execute("INSERT INTO agents (id, project, name, kind, command, color, at_once) VALUES (?1, 'default', upper(?1), 'claude', 'claude', 0, ?2)", (id, at_once)).unwrap();
}

fn body(agent: &str, cron: &str) -> Value {
    json!({"title": "Nightly deps check", "body": "Check the dependencies.", "agent": agent, "cron": cron, "budget_minutes": 30})
}

fn given(tx: &Transaction, v: Value) -> Result<Given, String> {
    Given::of(tx, &v, NOW)
}

#[test]
fn ac160_kept_listed_changed_and_deleted() {
    open("sched-kept").with(|tx| {
        agent(tx, "a", 2);
        let (id, key) = add(tx, HUMAN, &given(tx, body("a", "0 3 * * *"))?, NOW)?;
        assert_eq!(key, "SCH-1");
        let s = by_id(tx, &id)?;
        assert_eq!((s.title.as_str(), s.cron.as_deref(), s.budget_seconds, s.last_due, s.severity.as_str()), ("Nightly deps check", Some("0 3 * * *"), 1800, NOW, "P2"));
        let shown = listed(tx, NOW)?;
        assert_eq!(shown[0]["next"].as_array().map(Vec::len), Some(3), "its next three runs");
        assert!(shown[0]["words"].as_str().unwrap().starts_with("Every day at "));
        let paused = change(tx, HUMAN, &id, &given(tx, json!({"title": "Weekly", "body": "x", "agent": "a", "cron": "0 9 * * 1", "budget_minutes": 10, "paused": true}))?, NOW + 60)?;
        assert!(paused, "paused by this save");
        let s = by_id(tx, &id)?;
        assert_eq!((s.title.as_str(), s.paused, s.last_due), ("Weekly", true, NOW + 60), "a save fires nothing it passed");
        assert!(listed(tx, NOW)?[0]["next"].as_array().unwrap().is_empty(), "paused: no next run");
        remove(tx, &id, NOW + 120)?;
        assert!(all(tx)?.is_empty());
        assert_eq!(by_id(tx, &id).unwrap_err(), "That schedule is gone.");
        Ok(())
    }).unwrap();
}

#[test]
fn ac160_refused_in_the_field_s_words() {
    open("sched-refused").with(|tx| {
        agent(tx, "a", 5);
        let says = |v: Value| given(tx, v).err().unwrap_or_default();
        let mut b = body("a", "0 3 * * *");
        b["title"] = json!("");
        assert_eq!(says(b), "Title: 1–200 characters");
        assert_eq!(says(body("", "0 3 * * *")), "Choose the agent that runs it.");
        let mut b = body("a", "0 3 * * *");
        b["project"] = json!("");
        assert_eq!(says(b), SCHEDULE_NEEDS_PROJECT, "a window's project not chosen is never Default");
        let mut b = body("a", "0 3 * * *");
        b.as_object_mut().unwrap().remove("budget_minutes");
        assert_eq!(says(b), "A scheduled run needs a time budget: no human watches it.");
        let mut b = body("a", "0 3 * * *");
        b["budget_minutes"] = json!(1441);
        assert_eq!(says(b), "Time budget: 1 to 1440 minutes.");
        assert_eq!(says(body("a", "0 24 * * *")), "The hour \"24\" is not 0 to 23.");
        assert!(given(tx, body("a", "* * * * *")).is_ok(), "a minute apart, as cron's fields are");
        assert_eq!(given(tx, body("a", "*/1 * * * *"))?.as_schedule().words(), "Every minute");
        assert_eq!(says(body("a", "0 0 31 2 *")), "This cadence never comes.");
        assert_eq!(says(json!({"title": "t", "agent": "a", "once_at": NOW - 60, "budget_minutes": 5})), "Once: a time still to come.");
        assert_eq!(says(json!({"title": "t", "agent": "a", "budget_minutes": 5})), "When: choose how often it runs.");
        let mut b = body("a", "0 3 * * *");
        b["project"] = json!("gone");
        assert_eq!(says(b), "That project is gone.");
        Ok(())
    }).unwrap();
}

#[test]
fn ac161_an_agent_keeps_one_slot_for_every_two_schedules() {
    open("sched-count").with(|tx| {
        agent(tx, "a", 5);
        assert_eq!((0..=6).map(slots_for).collect::<Vec<_>>(), vec![0, 1, 1, 2, 2, 3, 3], "ceil(N / 2)");
        for (n, slots) in [(1, 1), (2, 1), (3, 2), (4, 2), (5, 3)] {
            add(tx, HUMAN, &given(tx, body("a", "0 3 * * *"))?, NOW)?;
            assert_eq!(kept_by_agent(tx)?["a"], Kept { slots, schedules: n, integration: false, supervisor: false }, "{n} schedules");
        }
        let mut b = body("a", "0 4 * * *");
        b["paused"] = json!(true);
        add(tx, HUMAN, &given(tx, b)?, NOW)?;
        assert_eq!(kept_by_agent(tx)?["a"].schedules, 5, "a paused one counts in no agent's N");
        Ok(())
    }).unwrap();
}

#[test]
fn ac161_a_schedule_is_refused_when_its_agent_cannot_keep_the_slots() {
    open("sched-slot").with(|tx| {
        agent(tx, "a", 1);
        agent(tx, "b", 1);
        let (one, _) = add(tx, HUMAN, &given(tx, body("a", "0 3 * * *"))?, NOW)?;
        add(tx, HUMAN, &given(tx, body("a", "0 3 * * *"))?, NOW)?;
        assert_eq!(kept_on(tx, "a", None)?, vec!["SCH-1", "SCH-2"], "two keep one slot of its one");
        assert_eq!(add(tx, HUMAN, &given(tx, body("a", "0 4 * * *"))?, NOW).unwrap_err(),
                   "A keeps 1 slot for 2 schedules: SCH-1, SCH-2; with this one it would keep 2 of its 1 at once: raise its at once, or choose another agent.");
        let mut b = body("a", "0 4 * * *");
        b["paused"] = json!(true);
        let (three, _) = add(tx, HUMAN, &given(tx, b)?, NOW)?;
        assert_eq!(kept_on(tx, "a", None)?.len(), 2, "a paused one keeps none");
        change(tx, HUMAN, &one, &given(tx, body("a", "0 5 * * *"))?, NOW)?;
        assert_eq!(kept_on(tx, "a", Some(&one))?, vec!["SCH-2"], "its own save does not count it");
        assert_eq!(change(tx, HUMAN, &three, &given(tx, body("a", "0 4 * * *"))?, NOW).unwrap_err(),
                   "A keeps 1 slot for 2 schedules: SCH-1, SCH-2; with SCH-3 it would keep 2 of its 1 at once: raise its at once, or choose another agent.", "unpausing needs a slot");
        let (other, _) = add(tx, HUMAN, &given(tx, body("b", "0 4 * * *"))?, NOW)?;
        assert_eq!(change(tx, HUMAN, &other, &given(tx, body("a", "0 4 * * *"))?, NOW).unwrap_err(),
                   "A keeps 1 slot for 2 schedules: SCH-1, SCH-2; with SCH-4 it would keep 2 of its 1 at once: raise its at once, or choose another agent.", "moving one onto it");
        remove(tx, &one, NOW)?;
        assert_eq!(kept_on(tx, "a", None)?, vec!["SCH-2"], "a deleted one keeps none");
        change(tx, HUMAN, &other, &given(tx, body("a", "0 4 * * *"))?, NOW)?;
        assert_eq!(kept_by_agent(tx)?.get("b"), None, "moved: its old agent keeps nothing for it");
        Ok(())
    }).unwrap();
}

#[test]
fn ac161_integration_s_kept_slot_counts_against_its_at_once() {
    open("sched-slot-integration").with(|tx| {
        agent(tx, "a", 2);
        let s = crate::tasks::integration::read(tx, "default")?;
        crate::tasks::integration::keep(tx, "default", &crate::tasks::integration::Settings { on: true, agent: Some("a".into()), ..s })?;
        assert_eq!(kept_by_agent(tx)?["a"], Kept { slots: 0, schedules: 0, integration: true, supervisor: false });
        add(tx, HUMAN, &given(tx, body("a", "0 3 * * *"))?, NOW)?;
        add(tx, HUMAN, &given(tx, body("a", "0 3 * * *"))?, NOW)?;
        assert_eq!(kept_by_agent(tx)?["a"].total(), 2);
        assert_eq!(add(tx, HUMAN, &given(tx, body("a", "0 4 * * *"))?, NOW).unwrap_err(),
                   "A keeps 1 slot for 2 schedules: SCH-1, SCH-2, and 1 for integration; with this one it would keep 3 of its 2 at once: raise its at once, or choose another agent.");
        Ok(())
    }).unwrap();
}

#[test]
fn ac162_due_by_the_wall_clock_and_missed_firings_come_once() {
    open("sched-due").with(|tx| {
        agent(tx, "a", 1);
        let (id, _) = add(tx, HUMAN, &given(tx, body("a", "*/15 * * * *"))?, NOW)?;
        let s = by_id(tx, &id)?;
        assert_eq!(s.due(NOW + 5 * 60), None, "20:40: not yet");
        assert_eq!(s.due(NOW + 10 * 60), Some((NOW + 10 * 60, 0)), "20:45");
        assert_eq!(s.due(NOW + 8 * 3600), Some((NOW + 8 * 3600 - 5 * 60, 31)), "eight hours asleep: the last of 32, once");
        let mut paused = s.clone();
        paused.paused = true;
        assert_eq!(paused.due(NOW + 8 * 3600), None);
        let once = Schedule { cron: None, once_at: Some(NOW + 3600), ..s };
        assert_eq!(once.due(NOW + 3600), Some((NOW + 3600, 0)));
        assert_eq!(Schedule { last_due: NOW + 3600, ..once }.due(NOW + 7200), None, "once fired, never again");
        Ok(())
    }).unwrap();
}

#[test]
fn ac160_where_it_runs_and_what_its_agent_reaches() {
    open("sched-place").with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('b', 'Billing', 0, '/srv/billing', 0), ('r', 'Remote', 1, '~/api', 1), ('n', 'Nowhere', 2, '', 2)", []).map_err(sql)?;
        tx.execute("UPDATE projects SET host = 'devbox.example' WHERE id = 'r'", []).map_err(sql)?;
        assert_eq!(place_words(tx, "b")?, "/srv/billing, with the board of Billing");
        assert_eq!(place_words(tx, "n")?, "Nowhere has no folder: set it in its window.", "as its firing would be refused");
        assert_eq!(place_words(tx, "r")?, "devbox.example: ~/api, with the board of Remote");
        assert_eq!(place_words(tx, crate::tasks::store::DEFAULT_PROJECT)?, "~, in no project: the agent reaches only this run's own task, no board. Choose a project for work on its board or folder.");
        Ok(())
    }).unwrap();
}
