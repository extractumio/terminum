// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-212, AC-213, AC-224: the red target's named query, its tie to a never line and its critical incident.

use serde_json::json;

use super::engine_tests::{Fixture, NOW, PROJECT};
use super::{DAY, Evaluation, Evidence, Rule, SCHEMA, incidents_api, signals};

const NEVER: &str = "Leave the target red.";

fn integrated(f: &Fixture, on: bool, ties: serde_json::Value) {
    f.integration(on);
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)", (PROJECT, NOW - 2 * DAY));
    let pack = json!({"charter": {"never": [NEVER], "never_checks": ties}});
    f.save_settings(pack).unwrap();
}

/// The target red on a Verify's failed steps: Dispatcher's Bug task for it, and, `confirmed`, its re-run's red notice.
fn red(f: &Fixture, bug: &str, confirmed: bool) -> Option<String> {
    f.task(bug, Some("Bug"), "P1", "TODO", NOW - 2000);
    f.write("UPDATE tasks SET slug = 'API-' || number WHERE id = ?1", [bug]);
    f.write("UPDATE project_integration SET red_bug = ?2 WHERE project = ?1", (PROJECT, bug));
    confirmed.then(|| confirm(f, bug))
}

fn confirm(f: &Fixture, bug: &str) -> String {
    f.event(bug, NOW - 100, "dispatcher", "comment", "The steps fail there too.",
        Some(json!({"notice": true, "target_red": true, "target_recheck": true})))
}

fn sweep(f: &Fixture, at: i64) -> Evaluation {
    f.live(Rule::MainRed, at)
}

#[test]
fn ac212_target_red_fires_once_its_re_run_confirms_it_and_while_its_bug_task_is_open() {
    let f = Fixture::new("main-red-off");
    integrated(&f, false, json!({}));
    red(&f, "red-bug", true);
    let off = sweep(&f, NOW);
    assert!(off.detectors[0].findings.is_empty());
    assert_eq!(off.detectors[0].unavailable, ["Project integration is off."]);

    let f = Fixture::new("main-red");
    integrated(&f, true, json!({}));
    assert!(sweep(&f, NOW).detectors[0].findings.is_empty(), "a green target stays quiet");
    red(&f, "red-bug", false);
    assert!(sweep(&f, NOW).detectors[0].findings.is_empty(), "a red not yet run again may be a busy moment's");
    let notice = confirm(&f, "red-bug");
    let evaluation = sweep(&f, NOW);
    let found = &evaluation.detectors[0].findings[0];
    assert_eq!((found.subject_kind, found.subject_id.as_str(), found.metric), ("target", "main", "target_red"),
        "the subject is the target, as AC-255's miss check names it");
    assert_eq!(found.evidence, [Evidence::Tasks("red-bug".into()), Evidence::Events(notice)]);
    assert!(!found.charter_breach() && found.scores.level() < 3, "an untied red target is an ordinary signal");
    assert_eq!((found.scores.impact, found.scores.blast, found.scores.reversibility), (0, 2, 1),
        "AC-213: without a share, a confirmed red target still reaches the target with hours of recovery");
    evaluation.record(&f.store).unwrap();
    let signal = signals(&f.store, PROJECT).unwrap().remove(0);
    assert_eq!((signal.level, signal.response), (2, "R4"));
    f.transition("red-bug", NOW + 10, "TODO", "DONE");
    assert!(sweep(&f, NOW + 20).detectors[0].findings.is_empty(), "the Bug task's end clears it");

    let unavailable = f.store.snapshot(SCHEMA, |tx| super::measure_detector(tx, PROJECT, &Fixture::detector("flow.main_red"),
        super::Window::week(NOW), false)).unwrap().unavailable;
    assert_eq!(unavailable, ["Red-target history keeps only the open Bug task."], "replay says why it has no history");
}

#[test]
fn ac224_a_red_target_tied_to_a_never_line_opens_one_incident_and_clears_it() {
    let f = Fixture::new("main-red-incident");
    integrated(&f, true, json!({"flow.main_red": NEVER}));
    red(&f, "red-bug", true);
    let evaluation = sweep(&f, NOW);
    let found = &evaluation.detectors[0].findings[0];
    assert_eq!((found.charter_breach(), found.charter_never.as_deref(), found.scores.level()), (true, Some(NEVER), 3));
    evaluation.record(&f.store).unwrap();
    sweep(&f, NOW + 60).record(&f.store).unwrap();
    let incidents = f.store.snapshot(SCHEMA, incidents_api::pending).unwrap();
    assert_eq!(incidents.len(), 1, "a standing red target keeps one incident");
    assert_eq!((incidents[0].project.as_str(), incidents[0].closed_at), (PROJECT, None));
    let signal = signals(&f.store, PROJECT).unwrap().into_iter().find(|s| s.detector == "flow.main_red").unwrap();
    assert!(signal.charter_breach, "quiet hours pass it (AC-228)");

    f.transition("red-bug", NOW + 90, "TODO", "DONE");
    sweep(&f, NOW + 120).record(&f.store).unwrap();
    assert_eq!(f.store.snapshot(SCHEMA, incidents_api::pending).unwrap()[0].closed_at, Some(NOW + 120), "green again: all-clear");
}
