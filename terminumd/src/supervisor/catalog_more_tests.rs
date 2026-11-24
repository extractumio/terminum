// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-212: firing and non-firing fixtures for the review-gate, owner-choice and Bug-cluster queries.

use serde_json::json;

use super::engine_tests::{Fixture, NOW, PROJECT};
use super::probe_arch::{Architecture, Changed};
use super::{DAY, Evidence, evaluate, probes, settings};

fn run(f: &Fixture, name: &str) -> (Vec<(String, f64)>, Vec<&'static str>) {
    let evaluation = evaluate(&f.store, PROJECT, NOW, &[Fixture::detector(name)]).unwrap();
    let checked = &evaluation.detectors[0];
    (checked.findings.iter().map(|found| (found.subject_id.clone(), found.measure.value)).collect(), checked.unavailable.clone())
}

fn configured(f: &Fixture, integration: bool) {
    f.write("UPDATE projects SET dir = ?1 WHERE id = ?2", (f.path.parent().unwrap().to_str().unwrap(), PROJECT));
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)", (PROJECT, NOW - 9 * DAY));
    let pack = json!({"charter": {"surfaces": {"t2": ["src/store/**"], "t3": ["src/auth/**"]}}});
    f.store.with(|tx| settings::write(tx, PROJECT, 0, serde_json::from_value(pack).unwrap(), false).map(drop)).unwrap();
    f.integration(integration);
}

fn gate(f: &Fixture, task: &str, tier: u8, opened: i64) {
    f.task(task, Some("Feature"), "P2", "IN PROGRESS", opened - 10);
    f.write("INSERT INTO supervisor_design_gates (task, project, tier, evidence, scope, existing_review, opened, deadline)
        VALUES (?1, ?2, ?3, 'src/store/kept.rs', '[]', 0, ?4, 0)", (task, PROJECT, tier, opened));
}

#[test]
fn ac212_surface_touches_fire_on_tasks_matching_t2_or_t3_in_the_window() {
    let f = Fixture::new("surface-unmapped");
    assert_eq!(run(&f, "arch.surface_touched").1, ["No T2 or T3 surface is configured."]);

    let f = Fixture::new("surface-touched");
    configured(&f, true);
    assert!(run(&f, "arch.surface_touched").0.is_empty(), "no gate, no touch");
    gate(&f, "old-touch", 3, NOW - 30 * DAY);
    gate(&f, "store-touch", 2, NOW - DAY);
    gate(&f, "auth-touch", 3, NOW - 2 * DAY);
    let (fired, unavailable) = run(&f, "arch.surface_touched");
    assert!(unavailable.is_empty());
    assert_eq!(fired, [("auth-touch".into(), 3.0), ("store-touch".into(), 2.0)], "an older touch is outside the window");
}

#[test]
fn ac212_review_missing_fires_on_verify_notices_only() {
    let f = Fixture::new("review-missing-off");
    configured(&f, false);
    assert_eq!(run(&f, "arch.review_missing").1, ["Project integration is off."]);

    let f = Fixture::new("review-missing");
    configured(&f, true);
    f.task("reviewed", Some("Feature"), "P2", "INTEGRATION", NOW - 3 * DAY);
    f.event("reviewed", NOW - DAY, "dispatcher", "comment", "design first",
        Some(json!({"notice": true, "design_gate": "design_first"})));
    assert!(run(&f, "arch.review_missing").0.is_empty(), "a design-first notice is not a Verify miss");
    f.task("unreviewed", Some("Feature"), "P2", "INTEGRATION", NOW - 3 * DAY);
    let notice = f.event("unreviewed", NOW - DAY, "dispatcher", "comment", "T3 Verify needs a review",
        Some(json!({"notice": true, "design_gate": "review_missing", "review_condition": "T3: a review"})));
    let evaluation = evaluate(&f.store, PROJECT, NOW, &[Fixture::detector("arch.review_missing")]).unwrap();
    let found = &evaluation.detectors[0].findings;
    assert_eq!((found.len(), found[0].subject_id.as_str()), (1, "unreviewed"));
    assert_eq!(found[0].evidence, [Evidence::Events(notice)]);
}

fn ask(f: &Fixture, task: &str, at: i64, kind: &str, chosen: &str) {
    let question = f.event(task, at, "agent:pool:agent", "comment", "Which way?", Some(json!({"questions": true, "question": {"kind": kind,
        "options": [{"id": "keep", "label": "Keep"}, {"id": "drop", "label": "Drop"}], "recommended": "keep"}})));
    f.event(task, at + 1, "human", "comment", "Selected option.", Some(json!({"choice": {"question": question, "option": chosen}})));
}

#[test]
fn ac212_rubber_stamp_counts_a_kind_s_latest_recommended_choices_in_a_row() {
    let f = Fixture::new("rubber-stamp");
    assert_eq!(run(&f, "owner.rubber_stamp").1, ["No answered question with options is retained."]);
    f.agent("agent");
    f.task("asking", Some("Feature"), "P2", "WAIT FOR HUMAN", NOW - 20 * DAY);
    for i in 0..4 { ask(&f, "asking", NOW - 10 * DAY + i * 10, "operational", "keep"); }
    ask(&f, "asking", NOW - 9 * DAY, "strategic", "keep");
    assert!(run(&f, "owner.rubber_stamp").0.is_empty(), "four in a row is under the streak of five");
    ask(&f, "asking", NOW - 8 * DAY, "operational", "keep");
    assert_eq!(run(&f, "owner.rubber_stamp").0, [("operational".into(), 5.0)], "the other kind keeps its own streak");
    ask(&f, "asking", NOW - 7 * DAY, "operational", "drop");
    assert!(run(&f, "owner.rubber_stamp").0.is_empty(), "choosing another option ends the streak");
}

#[test]
fn ac212_bug_cluster_counts_distinct_fix_commits_per_folder() {
    let f = Fixture::new("bug-cluster");
    configured(&f, true);
    assert_eq!(run(&f, "arch.bug_cluster").1, ["Git architecture observations are unavailable."]);
    let change = |at, path: &str, fix| Changed { at, path: path.into(), churn: 4, fix, commit: format!("{at}") };
    let arch = Architecture { since: NOW - 90 * DAY, changes: vec![
        change(NOW - DAY, "src/store/a.rs", true), change(NOW - DAY, "src/store/b.rs", true),
        change(NOW - 2 * DAY, "src/store/a.rs", true), change(NOW - 3 * DAY, "src/store/c.rs", true),
        change(NOW - 30 * DAY, "src/ui/a.js", true), change(NOW - DAY, "src/ui/a.js", true),
        Changed { commit: "same-second".into(), ..change(NOW - DAY, "src/store/d.rs", true) },
        change(NOW - 2 * DAY, "src/ui/b.js", false), change(NOW - 3 * DAY, "src/ui/c.js", false),
    ], ..Architecture::default() };
    probes::tests::publish_sample(&f, probes::ARCH, NOW - 10, serde_json::to_string(&arch).unwrap());
    let (fired, unavailable) = run(&f, "arch.bug_cluster");
    assert!(unavailable.is_empty(), "{unavailable:?}");
    assert_eq!(fired, [("src/store".into(), 4.0)],
        "a commit's files count once, another commit in its second counts; feature commits and older fixes do not count");
}
