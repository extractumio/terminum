// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-236, AC-211: an owner's Accept holds until its signal's count takes a repeat with evidence the ruling did not
//! see. Each test drives the real detector and signal write, so a retaken sample or a window's new input changes the
//! evidence the way the live collector does. Counts grown on every evaluation pass keep one occurrence (`schema65.sql`).

use crate::tasks::ops::{rows, sql};

use super::engine_tests::{Fixture, NOW, PROJECT};
use super::register_tests::draft;
use super::{DAY, Rule, Signal, evaluate, probe_arch::Architecture, probes, register, signals};

fn signal(f: &Fixture, pick: impl Fn(&Signal) -> bool) -> Signal {
    signals(&f.store, PROJECT).unwrap().into_iter().find(pick).expect("the detector fired")
}

/// The owner accepts as is a problem opened on the signal `key`.
fn accepted(f: &Fixture, key: &str) -> i64 {
    f.store.with(|tx| {
        let id = register::open(tx, PROJECT, draft("Accepted as is", &[key]), NOW)?;
        register::rule(tx, PROJECT, id, "The owner accepts this condition as is.", NOW)?;
        Ok(id)
    }).unwrap()
}

/// The problem's state after the register's sweep at `at`.
fn swept(f: &Fixture, id: i64, at: i64) -> String {
    f.store.with(|tx| {
        register::advance(tx, PROJECT, at)?;
        Ok(register::get(tx, PROJECT, id, at)?.state)
    }).unwrap()
}

#[test]
fn ac236_a_retaken_probe_sample_keeps_an_accepted_problem_accepted() {
    let f = Fixture::new("accept-probe");
    super::breach::tests::attached(&f, serde_json::json!({}));
    let arch = Architecture { since: NOW - 90 * DAY, migrations: vec![(NOW - 4 * DAY, "schema1.sql".into())], ..Architecture::default() };
    let data = serde_json::to_string(&arch).unwrap();
    let beat = |at| {
        probes::tests::publish_sample(&f, probes::ARCH, at, data.clone());
        f.live(Rule::MigrationEdited, at).record(&f.store).unwrap();
    };
    beat(NOW - 3 * DAY);
    let ruled = signal(&f, |s| s.detector == "arch.migration_edited");
    let id = accepted(&f, &ruled.fingerprint);
    for (n, at) in [NOW - 2 * DAY, NOW - DAY, NOW].into_iter().enumerate() {
        beat(at);
        let now = signal(&f, |s| s.fingerprint == ruled.fingerprint);
        assert_ne!(now.evidence, ruled.evidence, "beat {n} retook the sample the ruling saw");
        assert_eq!(now.count, 1, "the same edited migration is one occurrence");
        assert_eq!(swept(&f, id, at + 1), "accepted", "beat {n}");
    }
}

#[test]
fn ac236_a_window_input_keeps_an_accepted_spend_problem_accepted() {
    let f = Fixture::new("accept-spend");
    let old = NOW - 5 * super::WEEK;
    f.task("cheap", Some("Feature"), "P2", "IN PROGRESS", old - 1);
    f.run("cheap", "cheap-run", "agent", old);
    f.cost("cheap-run", old + 1, 10_000);
    f.task("expensive", Some("Feature"), "P2", "IN PROGRESS", NOW - 110);
    f.failed_run("expensive", "expensive-run", NOW - 90);
    f.cost("expensive-run", NOW - 89, 3_000_000);
    let d = Fixture::detector("flow.cost_outlier");
    let beat = |at| evaluate(&f.store, PROJECT, at, std::slice::from_ref(&d)).unwrap().record(&f.store).unwrap();
    beat(NOW);
    let ruled = signal(&f, |s| s.metric == "task_dollars");
    let id = accepted(&f, &ruled.fingerprint);
    for (n, at) in [NOW + 10, NOW + 20].into_iter().enumerate() {
        f.cost("expensive-run", at - 5, 3_000_000);
        beat(at);
        let now = signal(&f, |s| s.fingerprint == ruled.fingerprint);
        assert!(now.evidence.len() > ruled.evidence.len(), "usage row {n} joined the evidence");
        assert_eq!(now.count, 1, "a growing spend total is one occurrence");
        assert_eq!(swept(&f, id, at + 1), "accepted", "usage row {n}");
    }
}

#[test]
fn ac236_a_new_failed_run_reopens_an_accepted_problem() {
    let f = Fixture::new("accept-failures");
    f.task("repeat", Some("Bug"), "P2", "IN PROGRESS", NOW - 200);
    f.failed_run("repeat", "run-a", NOW - 50);
    f.failed_run("repeat", "run-b", NOW - 20);
    let d = Fixture::detector("repeated_failures");
    let beat = |at| evaluate(&f.store, PROJECT, at, std::slice::from_ref(&d)).unwrap().record(&f.store).unwrap();
    beat(NOW);
    let key = signal(&f, |s| s.detector == "flow.failed_repeat").fingerprint;
    let id = accepted(&f, &key);
    beat(NOW + 10);
    assert_eq!(swept(&f, id, NOW + 11), "accepted", "a sweep of the same failed runs is no repeat");

    f.failed_run("repeat", "run-c", NOW + 15);
    beat(NOW + 20);
    assert_eq!(signal(&f, |s| s.fingerprint == key).count, 2);
    assert_eq!(swept(&f, id, NOW + 21), "recurred", "a new failed run is a new counted event");
    f.store.with(|tx| {
        let detail: String = tx.query_row("SELECT new_evidence FROM supervisor_problems WHERE id = ?1", [id], |r| r.get(0)).map_err(sql)?;
        assert_eq!(detail, format!("New evidence from signal {key}."));
        Ok(())
    }).unwrap();
}

#[test]
fn ac236_a_repeat_counted_while_its_signal_is_held_reopens_once_it_is_active() {
    let f = Fixture::new("accept-held");
    super::register_tests::signal(&f, "held", "A", 1, 0.1, false);
    let id = accepted(&f, "held");
    f.write("UPDATE supervisor_signals SET state = 'held', count = 2, evidence = ?1 WHERE fingerprint = 'held'",
        [r#"[{"table":"events","id":"event-a"},{"table":"events","id":"event-b"}]"#]);
    assert_eq!(swept(&f, id, NOW + 1), "accepted", "a held signal reopens nothing");
    f.write("UPDATE supervisor_signals SET state = 'new' WHERE fingerprint = 'held'", []);
    assert_eq!(swept(&f, id, NOW + 2), "recurred", "the repeat counted while held was not absorbed");
}

#[test]
fn ac211_schema65_resets_counts_that_counted_evaluation_passes() {
    let f = Fixture::new("count-reset");
    for (key, count, state) in [("passes", 146, "new"), ("back", 404, "recurred"), ("once", 1, "raised"), ("ruled", 23, "new")] {
        super::register_archive_tests::backlog(&f, key);
        f.write("UPDATE supervisor_signals SET count = ?2, state = ?3 WHERE fingerprint = ?1", (key, count, state));
    }
    f.store.with(|tx| {
        let id = register::open(tx, PROJECT, draft("Ruled", &["ruled"]), NOW)?;
        register::rule(tx, PROJECT, id, "The owner accepts this as is.", NOW)?;
        tx.execute_batch(include_str!("../tasks/schema65.sql")).map_err(sql)?;
        let counts = rows(tx, "SELECT fingerprint, count FROM supervisor_signals ORDER BY fingerprint", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        assert_eq!(counts, [("back".into(), 2), ("once".into(), 1), ("passes".into(), 1), ("ruled".into(), 1)]);
        let ruling: i64 = tx.query_row("SELECT ruling_count FROM supervisor_problem_signals WHERE fingerprint = 'ruled'", [], |r| r.get(0)).map_err(sql)?;
        assert_eq!(ruling, 1, "a ruling's count is no higher than its signal's, so its next repeat reopens it");
        Ok(())
    }).unwrap();
}
