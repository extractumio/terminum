// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213: `p0_waiting` raises a P0 task waiting in TODO past its age limit to level 2, and to 3 a day past it,
//! naming what held it; a P1 task, replay and weekly samples keep the share rule.

use serde_json::json;

use super::super::engine_tests::{Fixture, NOW, PROJECT};
use super::super::signals::State;
use super::super::{DAY, Rule, WEEK, Window, measure_detector, signals};

const OFF: &str = "Auto-Dispatch is off for this project.";

/// 17 P1 tasks open all week, so one task's hour past its limit is a negligible share; an agent with free slots.
fn project(name: &str) -> Fixture {
    let f = Fixture::new(name);
    f.agent("agent");
    for n in 0..17 {
        f.task(&format!("open-{n}"), None, "P1", "IN PROGRESS", NOW - 2 * WEEK);
    }
    f
}

/// The only finding of `rule` at `at` on task `id`: its impact and the raises' (score, from, to, rule, reason).
fn scored(f: &Fixture, rule: Rule, at: i64, id: &str) -> (u8, serde_json::Value) {
    let evaluation = f.live(rule, at);
    let found = evaluation.detectors[0].findings.iter().find(|found| found.subject_id == id)
        .unwrap_or_else(|| panic!("{} did not fire on {id}", rule.id()));
    (found.scores.impact, serde_json::to_value(&found.score_raises).unwrap())
}

fn raise(from: u8, to: u8, reason: &str) -> serde_json::Value {
    json!([{"score": "impact", "from": from, "to": to, "rule": "p0_waiting", "reason": reason}])
}

#[test]
fn ac213_a_waiting_p0_task_reaches_the_owner_and_resolves_when_it_leaves_todo() {
    let f = project("waiting-p0");
    f.task("p0", Some("Bug"), "P0", "TODO", NOW - DAY - 3600);
    assert_eq!(scored(&f, Rule::Starved, NOW, "p0"), (2, raise(0, 2, OFF)), "one hour past its limit");
    f.live(Rule::Starved, NOW).record(&f.store).unwrap();
    let signal = signals(&f.store, PROJECT).unwrap().remove(0);
    assert_eq!((signal.level, signal.response, signal.charter_breach), (2, "R4", false));
    assert_eq!(serde_json::to_value(&signal).unwrap()["score_raises"], raise(0, 2, OFF), "the record keeps the raise");

    let later = NOW + DAY;
    assert_eq!(scored(&f, Rule::Starved, later, "p0"), (3, raise(0, 3, OFF)), "a full day past its limit");
    f.live(Rule::Starved, later).record(&f.store).unwrap();
    let signal = signals(&f.store, PROJECT).unwrap().remove(0);
    assert_eq!((signal.level, signal.response, signal.charter_breach), (3, "R5", false));
    let reports: Vec<String> = f.store.with(|tx| crate::tasks::ops::rows(tx,
        "SELECT body FROM reports WHERE incident IS NOT NULL", [], |r| r.get(0))).unwrap();
    let [report] = reports.as_slice() else { panic!("one incident report: {reports:?}") };
    assert!(report.contains(OFF), "the incident names what held the task: {report}");
    let events = || f.store.with(|tx| tx.query_row("SELECT count(*) FROM supervisor_incident_events", [], |r| r.get::<_, i64>(0))
        .map_err(crate::tasks::ops::sql)).unwrap();
    let opened = events();
    f.live(Rule::Starved, later + 30).record(&f.store).unwrap();
    assert_eq!(events(), opened, "an age that only grew adds no timeline event");

    f.transition("p0", later + 60, "TODO", "IN PROGRESS");
    let (impact, raises) = scored(&f, Rule::UserBugAge, later + 120, "p0");
    assert_eq!((impact, raises), (0, json!([])), "a Bug in progress keeps its measured impact");
    f.live(Rule::Starved, later + 120).record(&f.store).unwrap();
    let signal = signals(&f.store, PROJECT).unwrap().remove(0);
    assert_eq!(signal.state, State::Resolved, "the starved signal resolves once the task leaves TODO");
}

#[test]
fn ac213_the_raise_names_what_held_a_waiting_p0_bug() {
    let f = project("waiting-held");
    f.task("p0", Some("Bug"), "P0", "TODO", NOW - 2 * DAY);
    // Auto-Dispatch names tasks by slug, as the store gives every task one.
    f.write("UPDATE tasks SET slug = 'API-7' WHERE id = 'p0'", []);
    let held = |f: &Fixture| scored(f, Rule::UserBugAge, NOW, "p0").1[0]["reason"].as_str().unwrap().to_owned();
    assert_eq!(held(&f), OFF);

    f.task("blocker", None, "P2", "TODO", NOW - DAY);
    f.write("INSERT INTO task_links (task, kind, other, place) VALUES ('p0', 'blocked_by', 'blocker', 0)", []);
    assert_eq!(held(&f), "It waits on blocker, which is not DONE.");
    f.write("DELETE FROM task_links", []);

    f.write("UPDATE projects SET auto_off_why = 'it could not start API-12' WHERE id = ?1", [PROJECT]);
    assert_eq!(held(&f), "Auto-Dispatch turned itself off: it could not start API-12");

    f.write("UPDATE projects SET auto_on = 1, auto_off_why = NULL, auto_severities = '[\"P1\"]', auto_day_most = 1 WHERE id = ?1", [PROJECT]);
    assert_eq!(held(&f), "Auto-Dispatch takes no P0 tasks.");

    f.write("UPDATE projects SET auto_severities = '[\"P0\",\"P1\"]' WHERE id = ?1", [PROJECT]);
    assert!(held(&f).starts_with("Auto-Dispatch passes over it: "), "no agent of the pool is named: {}", held(&f));

    f.write("UPDATE projects SET auto_default = 'agent' WHERE id = ?1", [PROJECT]);
    assert_eq!(held(&f), "Auto-Dispatch is on and has not started it.");

    f.write("INSERT INTO auto_starts (task, round, project, at) VALUES ('open-0', NULL, ?1, ?2)", (PROJECT, NOW - 60));
    assert_eq!(held(&f), "Auto-Dispatch started its 1 tasks of 24 hours.");
    f.write("UPDATE projects SET auto_day_most = 10 WHERE id = ?1", [PROJECT]);

    f.run("open-1", "busy", "agent", NOW - 120);
    f.write("UPDATE runs SET state = 'working' WHERE id = 'busy'", []);
    assert_eq!(held(&f), "No agent that may run it has a free slot.");
}

#[test]
fn ac213_a_waiting_p0_task_is_critical_from_a_full_day_past_its_limit() {
    let f = project("waiting-day");
    f.task("p0", None, "P0", "TODO", NOW - DAY);
    let limit = NOW;
    assert!(f.live(Rule::Starved, limit).detectors[0].findings.is_empty(), "at its limit it does not fire");
    assert_eq!(scored(&f, Rule::Starved, limit + DAY - 60, "p0").0, 2, "a minute short of a day past it");
    assert_eq!(scored(&f, Rule::Starved, limit + DAY, "p0").0, 3, "a full day past it");
}

#[test]
fn ac213_a_waiting_p1_task_and_replay_keep_the_share_rule() {
    let f = project("waiting-p1");
    f.task("p1", Some("Bug"), "P1", "TODO", NOW - DAY - 3600);
    f.task("p0", Some("Bug"), "P0", "TODO", NOW - 3 * DAY);
    for rule in [Rule::Starved, Rule::UserBugAge] {
        assert_eq!(scored(&f, rule, NOW, "p1"), (0, json!([])), "{}", rule.id());
        let detector = Fixture::detector(rule.id());
        // Present-only: a replay reads no waiting task, so it never raises one.
        let historical = f.store.with(|tx| measure_detector(tx, PROJECT, &detector, Window::of(NOW, 28), false)).unwrap();
        assert!(historical.readings.is_empty() && !historical.unavailable.is_empty(), "{}", rule.id());
        assert_eq!(scored(&f, rule, NOW, "p0").0, 3, "{}: the same P0 task is raised on the present", rule.id());
    }
}

/// Saves `pack` as the project's Supervisor settings, answering the save's result.
fn save(f: &Fixture, pack: serde_json::Value) -> Result<(), String> {
    f.store.with(|tx| super::super::settings::write(tx, PROJECT, 0, serde_json::from_value(pack).unwrap(), false).map(drop))
}

#[test]
fn ac213_the_projects_p0_waiting_hours_set_its_levels() {
    let f = project("waiting-hours");
    let defaults = serde_json::to_value(super::super::settings::Settings::default()).unwrap();
    assert_eq!(defaults["p0_waiting"], json!({"level2_hours": 24.0, "level3_hours": 24.0}), "unset, a day and a further day");
    save(&f, json!({"p0_waiting": {"level2_hours": 1, "level3_hours": 4}})).unwrap();
    f.task("p0", Some("Bug"), "P0", "TODO", NOW - 3600);
    f.task("p1", Some("Bug"), "P1", "TODO", NOW - 3600);
    for rule in [Rule::Starved, Rule::UserBugAge] {
        assert!(f.live(rule, NOW).detectors[0].findings.is_empty(), "{}: at one hour it does not fire", rule.id());
        assert_eq!(scored(&f, rule, NOW + 60, "p0"), (2, raise(0, 2, OFF)), "{}: past one hour, below the detector's day", rule.id());
        assert_eq!(scored(&f, rule, NOW + 4 * 3600 - 60, "p0").0, 2, "{}: a minute short of five hours", rule.id());
        assert_eq!(scored(&f, rule, NOW + 4 * 3600, "p0"), (3, raise(0, 3, OFF)), "{}: five hours", rule.id());
        let found = f.live(rule, NOW + 4 * 3600).detectors[0].findings.iter().map(|found| found.subject_id.clone()).collect::<Vec<_>>();
        assert_eq!(found, ["p0"], "{}: a P1 task keeps the detector's own limit", rule.id());
    }
}

#[test]
fn ac213_a_detector_limit_below_the_p0_hours_fires_with_the_measured_share() {
    let f = project("waiting-low-limit");
    save(&f, json!({"detectors": {"flow.starved": {"threshold": {"absolute": 60.0}}}})).unwrap();
    f.task("p0", None, "P0", "TODO", NOW - 2 * 3600);
    assert_eq!(scored(&f, Rule::Starved, NOW, "p0"), (0, json!([])), "past the detector's hour, short of the P0 day");
    assert_eq!(scored(&f, Rule::Starved, NOW + DAY, "p0").0, 2, "past the P0 day");
}

#[test]
fn ac213_p0_waiting_hours_out_of_range_are_refused() {
    let f = project("waiting-invalid");
    for hours in [json!({"level2_hours": 0}), json!({"level3_hours": 721}), json!({"level2_hours": 1.5}), json!({"level3_hours": -2})] {
        assert_eq!(save(&f, json!({"p0_waiting": hours})).unwrap_err(), "P0 waiting escalation: use whole hours from 1 to 720.", "{hours}");
    }
    let saved = f.store.with(|tx| super::super::settings::read(tx, PROJECT)).unwrap();
    assert_eq!((saved.revision, serde_json::to_value(&saved.settings).unwrap()["p0_waiting"].clone()),
        (0, json!({"level2_hours": 24.0, "level3_hours": 24.0})), "the saved settings stay as they were");
    save(&f, json!({"p0_waiting": {"level2_hours": 720, "level3_hours": 1}})).unwrap();
}
