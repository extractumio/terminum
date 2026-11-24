// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-212, AC-215: firing, quiet, missing-input, isolation and replay fixtures for
//! `product.external` and `product.feedback_age`.

use serde_json::json;

use super::engine_tests::{Fixture, NOW, OTHER, PROJECT};
use super::probes::tests::publish_sample;
use super::{DAY, Evidence, Threshold, WEEK, evaluate, replay, settings};

const BASE: &str = "https://example.com/issues/";

fn run(f: &Fixture, project: &str, name: &str, threshold: Option<Threshold>) -> (Vec<(String, f64)>, Vec<&'static str>, Vec<Evidence>) {
    let mut detector = Fixture::detector(name);
    if let Some(threshold) = threshold { detector.threshold = threshold; }
    let evaluation = evaluate(&f.store, project, NOW, &[detector]).unwrap();
    let checked = &evaluation.detectors[0];
    (checked.findings.iter().map(|found| (found.subject_id.clone(), found.measure.value)).collect(),
        checked.unavailable.clone(), checked.findings.iter().flat_map(|found| found.evidence.clone()).collect())
}

/// An enabled project in the fixture's folder, its approved probes and its pack.
fn configured(f: &Fixture, project: &str, probes: &[&str], pack: serde_json::Value) {
    f.write("UPDATE projects SET dir = ?1 WHERE id = ?2", (f.path.parent().unwrap().to_str().unwrap(), project));
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)", (project, NOW - 20 * WEEK));
    for probe in probes {
        f.write("INSERT INTO supervisor_probe_commands VALUES (?1, ?2, 'printf probe')", (project, *probe));
    }
    f.store.with(|tx| settings::write(tx, project, 0, serde_json::from_value(pack).unwrap(), false).map(drop)).unwrap();
}

fn indicators() -> serde_json::Value {
    json!({"charter": {"outside_indicators": {
        "Downloads": {"probe": "store", "metric": "downloads", "higher_is_better": true, "guard": "errors", "guard_higher_is_better": false},
        "Errors": {"probe": "store", "metric": "errors", "higher_is_better": false, "guard": "downloads", "guard_higher_is_better": true},
    }}})
}

/// One sample a week, each with that week's points: downloads 10 → 20, errors 1 → 5 from week `turn` on.
fn weekly_store_samples(f: &Fixture, weeks: i64, turn: i64) {
    for week in 0..weeks {
        let at = NOW - (weeks - 1 - week) * WEEK - 1;
        let (downloads, errors) = if week < turn { (10, 1) } else { (20, 5) };
        let points = json!([
            {"metric": "downloads", "value": downloads, "unit": "count", "at": at, "dims": {}},
            {"metric": "errors", "value": errors, "unit": "count", "at": at, "dims": {}},
        ]);
        publish_sample(f, "custom:store", at, points.to_string());
    }
}

#[test]
fn ac212_outside_metric_fires_only_against_its_direction_beyond_its_band() {
    let f = Fixture::new("external-unset");
    configured(&f, PROJECT, &["store"], json!({}));
    assert_eq!(run(&f, PROJECT, "product.external", None).1, ["No outside indicator is set in the charter."]);

    let f = Fixture::new("external-short");
    configured(&f, PROJECT, &["store"], indicators());
    weekly_store_samples(&f, 6, 2);
    let (fired, unavailable, _) = run(&f, PROJECT, "product.external", None);
    assert!(fired.is_empty());
    assert_eq!(unavailable, ["The probe needs a reading in each week of the comparison."; 2]);

    let f = Fixture::new("external-moves");
    configured(&f, PROJECT, &["store"], indicators());
    weekly_store_samples(&f, 8, 4);
    let (fired, unavailable, evidence) = run(&f, PROJECT, "product.external", None);
    assert!(unavailable.is_empty(), "{unavailable:?}");
    assert_eq!(fired, [("Errors".into(), 5.0)], "downloads rose, their better direction; errors rose against theirs");
    assert_eq!(evidence.len(), 8, "each week's retained sample is evidence");
    assert!(evidence.iter().all(|e| matches!(e, Evidence::SupervisorProbes(_))));
    let margin = |by| run(&f, PROJECT, "product.external", Some(Threshold::Change { by })).0.len();
    assert_eq!(margin(4.0), 0, "a move of 4 equal to noise 0 plus a margin of 4 stays inside the band");
    assert_eq!(margin(3.9), 1, "the same move crosses a narrower band");

    f.write("UPDATE supervisor_probe_commands SET command = 'printf replaced' WHERE project = ?1", [PROJECT]);
    assert_eq!(run(&f, PROJECT, "product.external", None).1, ["The probe needs a reading in each week of the comparison."; 2],
        "a replaced probe's old readings measure nothing");
}

#[test]
fn ac212_outside_metric_names_each_missing_input_and_stays_in_its_project() {
    let f = Fixture::new("external-isolated");
    let pack = json!({"charter": {"outside_indicators": {
        "Errors": {"probe": "store", "metric": "errors", "higher_is_better": false, "guard": "downloads", "guard_higher_is_better": true},
        "Legacy": "store",
        "Unapproved": {"probe": "survey", "metric": "score", "higher_is_better": true, "guard": "replies", "guard_higher_is_better": true},
    }}});
    configured(&f, PROJECT, &["store"], pack);
    weekly_store_samples(&f, 8, 4);
    configured(&f, OTHER, &["store"], indicators());
    let (fired, unavailable, _) = run(&f, PROJECT, "product.external", None);
    assert_eq!(fired, [("Errors".into(), 5.0)]);
    assert_eq!(unavailable, ["Choose a metric and a guard in project settings.", "The custom probe is not approved."]);

    f.write("UPDATE supervisor_probe_samples SET project = ?1 WHERE project = ?2", (OTHER, PROJECT));
    assert!(run(&f, PROJECT, "product.external", None).0.is_empty(), "another project's samples are not this project's");
    assert_eq!(run(&f, OTHER, "product.external", None).0, [("Errors".into(), 5.0)]);
}

fn feedback(open: &[(u64, i64)]) -> String {
    json!([{"unanswered": open.iter().map(|(reference, since)| json!({"reference": reference, "since": since})).collect::<Vec<_>>()}]).to_string()
}

#[test]
fn ac212_unanswered_feedback_fires_past_its_age_and_names_missing_input() {
    let f = Fixture::new("feedback-unset");
    configured(&f, PROJECT, &["feedback"], json!({}));
    assert_eq!(run(&f, PROJECT, "product.feedback_age", None).1, ["No approved probe has an outside feedback reference base."]);

    let f = Fixture::new("feedback-age");
    configured(&f, PROJECT, &["feedback"], json!({"probes": {"feedback": {"feedback": BASE}}}));
    assert_eq!(run(&f, PROJECT, "product.feedback_age", None).1, ["Feedback probe history is unavailable for this window."]);
    publish_sample(&f, "custom:feedback", NOW - 3 * DAY, feedback(&[(7, NOW - 9 * DAY)]));
    assert_eq!(run(&f, PROJECT, "product.feedback_age", None).1, ["Feedback probe history is unavailable for this window."],
        "a sample older than two days measures nothing");
    publish_sample(&f, "custom:feedback", NOW - 2 * DAY, "[]".into());
    assert_eq!(run(&f, PROJECT, "product.feedback_age", None).1, ["The feedback probe reports no unanswered list."]);

    let at = NOW - DAY;
    publish_sample(&f, "custom:feedback", at, feedback(&[(42, at - 3 * DAY - 60), (43, at - 3 * DAY), (44, at - DAY)]));
    let (fired, unavailable, evidence) = run(&f, PROJECT, "product.feedback_age", None);
    assert!(unavailable.is_empty(), "{unavailable:?}");
    assert_eq!(fired, [(format!("{BASE}42"), 4321.0)], "three days exactly is the limit, not past it");
    assert!(matches!(evidence[..], [Evidence::SupervisorProbes(_)]));
    assert_eq!(run(&f, PROJECT, "product.feedback_age", Some(Threshold::Absolute(1440.0))).0.len(), 2);

    configured(&f, OTHER, &["feedback"], json!({"probes": {"feedback": {"feedback": BASE}}}));
    assert!(run(&f, OTHER, "product.feedback_age", None).0.is_empty(), "this project's feedback is not another's");
}

#[test]
fn ac215_replay_counts_outside_firings_from_retained_samples() {
    let f = Fixture::new("outside-replay");
    let mut pack = indicators();
    pack["probes"] = json!({"feedback": {"feedback": BASE}});
    configured(&f, PROJECT, &["store", "feedback"], pack);
    weekly_store_samples(&f, 10, 4);
    publish_sample(&f, "custom:feedback", NOW - 3 * WEEK, feedback(&[(42, NOW - 4 * WEEK)]));
    publish_sample(&f, "custom:feedback", NOW - 2 * WEEK, feedback(&[(42, NOW - 4 * WEEK)]));

    let report = replay(&f.path, PROJECT, Some(NOW)).unwrap();
    assert_eq!(report, replay(&f.path, PROJECT, Some(NOW)).unwrap());
    assert!(report.contains("- product.external: 1\n"), "only the cutoff two weeks back has eight retained weeks: {report}");
    assert!(report.contains("- product.feedback_age: 2\n"), "{report}");
    assert!(report.contains("product.feedback_age: Feedback probe history is unavailable for this window."),
        "the current cutoff has no fresh feedback sample: {report}");
    assert!(!report.contains("No historical"), "{report}");
}
