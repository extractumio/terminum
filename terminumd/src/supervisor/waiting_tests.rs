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
    assert_eq!(held(&f), "Auto-Dispatch is on and has not started it.");

    f.write("INSERT INTO auto_starts (task, round, project, at) VALUES ('open-0', NULL, ?1, ?2)", (PROJECT, NOW - 60));
    assert_eq!(held(&f), "Auto-Dispatch started its 1 tasks of 24 hours.");
    f.write("UPDATE projects SET auto_day_most = 10 WHERE id = ?1", [PROJECT]);

    f.run("open-1", "busy", "agent", NOW - 120);
    f.write("UPDATE runs SET state = 'working' WHERE id = 'busy'", []);
    assert_eq!(held(&f), "No agent that may run it has a free slot.");
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
