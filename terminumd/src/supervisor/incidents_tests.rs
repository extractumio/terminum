// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use crate::tasks::store::{HUMAN, Store};

use super::{Checked, Evaluation, Evidence, Finding, Mode, Reading, Rule, SCHEMA, Scores, Threshold, Window};
use super::engine_tests::{Fixture, NOW, OTHER, PROJECT};
use super::incidents::{self, Action};
use super::incidents_api::{self, Event, Incident};

fn finding(subject: &str, level: u8, n: i64) -> Finding {
    let mut found = Reading::new("project", subject, "failed_deliveries", n as f64, vec![Evidence::Inputs(n)], super::impact::Source::Present)
        .finding(Threshold::Absolute(0.0), Window { before: NOW - 2, start: NOW - 1, end: NOW }).unwrap();
    found.scores = Scores { impact: level, ..Scores::default() };
    found
}

fn record(store: &Store, project: &str, at: i64, mode: Mode, findings: Vec<Finding>, unavailable: bool) {
    Evaluation { project: project.into(), at, detectors: vec![Checked {
        rule: Rule::LineDead, mode, findings,
        unavailable: if unavailable { vec!["Pending reading"] } else { vec![] },
        thresholds: Default::default(), complete_metrics: vec![], measured: None, observations: Default::default(),
    }] }.record(store).unwrap();
}

fn all(store: &Store) -> Vec<Incident> {
    store.snapshot(SCHEMA, incidents_api::pending).unwrap()
}

fn events(store: &Store, id: i64) -> Vec<Event> {
    store.snapshot(SCHEMA, |tx| incidents_api::events(tx, id, None)).unwrap()
}

#[test]
fn ac224_incident_grouping_and_clear() {
    let f = Fixture::new("incident-grouping");
    let store = &f.store;
    let live = |at, findings| record(store, PROJECT, at, Mode::Live, findings, false);
    live(NOW, vec![finding("target", 3, 1)]);
    let first = all(store)[0].id;
    live(NOW + 600, vec![finding("target", 3, 2)]);
    live(NOW + 1200, vec![finding("target", 3, 2)]);
    assert_eq!(all(store).len(), 1);
    assert_eq!(events(store, first).iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["signal", "signal"]);
    // AC-213: a human reads the level's word, the page its number only in a tooltip
    assert!(events(store, first)[0].words.starts_with("Incident level. "), "{}", events(store, first)[0].words);
    let shown = serde_json::to_value(&all(store)[0]).unwrap();
    assert_eq!((&shown["level"], &shown["level_word"]), (&serde_json::json!(3), &serde_json::json!("Incident")));

    live(NOW + 3000, vec![finding("target", 3, 3)]);
    let rows = all(store);
    let second = rows[0].id;
    assert_eq!(rows.len(), 2);
    assert_ne!(second, first);
    assert_eq!(rows[1].closed_at, Some(NOW + 3000));
    assert_eq!(events(store, first)[0].words, format!("Continued in incident {second}."));

    live(NOW + 3001, vec![finding("target", 2, 3)]);
    record(store, PROJECT, NOW + 3002, Mode::Live, vec![], true);
    assert_eq!(all(store)[0].closed_at, None, "missing data cannot clear a signal");
    live(NOW + 3003, vec![]);
    assert_eq!(all(store)[0].closed_at, Some(NOW + 3003));
    let timeline = events(store, second);
    assert_eq!(timeline.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["all_clear", "cleared", "updated", "signal"]);
    assert_eq!(timeline[2].level, Some(2));

    live(NOW + 3004, vec![finding("target", 3, 3)]);
    assert_ne!(all(store)[0].id, second, "a new firing after all-clear opens an incident");
}

#[test]
fn ac224_distinct_fingerprints_share_subject_and_clear_independently() {
    let f = Fixture::new("incident-subject");
    let store = &f.store;
    let mut another_metric = finding("target", 3, 2);
    another_metric.metric = "another_metric";
    record(store, PROJECT, NOW, Mode::Live, vec![finding("target", 3, 1), another_metric], false);
    record(store, OTHER, NOW, Mode::Live, vec![finding("target", 3, 1)], false);
    record(store, PROJECT, NOW + 1, Mode::Live, vec![finding("target", 3, 1)], false);
    let all = all(store);
    assert_eq!(all.len(), 2, "the project is part of the subject");
    assert!(all.iter().all(|i| i.closed_at.is_none()));
    let ours = all.iter().find(|i| i.project == PROJECT).unwrap();
    assert_eq!(events(store, ours.id).iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["cleared", "signal", "signal"]);
    assert!(ours.happened.contains("failed 1 deliveries"));
    assert!(!ours.happened.contains("another metric"));
    assert!(!ours.happened.contains("target"));
}

#[test]
fn ac224_shadow_and_digest_findings_do_not_page() {
    let f = Fixture::new("incident-shadow");
    for (at, mode) in [(NOW, Mode::Shadow), (NOW + 1, Mode::Digest)] {
        record(&f.store, PROJECT, at, mode, vec![finding("target", 3, 1)], false);
        assert!(all(&f.store).is_empty());
    }
    record(&f.store, PROJECT, NOW + 2, Mode::Live, vec![finding("target", 3, 1)], false);
    assert_eq!(all(&f.store).len(), 1);
}

#[test]
fn ac226_owner_keys_are_audited_once_and_do_not_clear_the_cause() {
    let f = Fixture::new("incident-actions");
    let store = &f.store;
    record(store, PROJECT, NOW, Mode::Live, vec![finding("target", 3, 1)], false);
    let id = all(store)[0].id;
    let act = |action, at| store.with(|tx| incidents::act(tx, id, action, at)).unwrap();
    act(Action::Snooze, NOW + 1);
    act(Action::Snooze, NOW + 2);
    assert_eq!(all(store)[0].snoozed_until, Some(NOW + 3601));
    act(Action::Acknowledge, NOW + 3);
    act(Action::Acknowledge, NOW + 4);
    let shown = &all(store)[0];
    assert_eq!((shown.acknowledged_at, shown.snoozed_until, shown.closed_at), (Some(NOW + 3), None, None));
    assert_eq!(events(store, id).iter().filter(|e| e.actor == HUMAN).map(|e| (e.kind.as_str(), e.at)).collect::<Vec<_>>(),
        [("acknowledged", NOW + 3), ("snoozed", NOW + 1)]);
    record(store, PROJECT, NOW + 5, Mode::Live, vec![], false);
    assert!(all(store).is_empty());
    assert_eq!(events(store, id)[0].kind, "all_clear");
}
