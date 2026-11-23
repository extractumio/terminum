// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213, AC-214, AC-215: private, migrated fixtures exercise the shipped CLI path.

use std::{collections::HashMap, fs};

use rusqlite::Connection;

use super::engine_tests::{Fixture, NOW, OTHER, PROJECT};
use super::impact::{Assessment, Share, Unit};
use super::{WEEK, Scores, evaluate, signals};

#[test]
fn ac213_impact_floors_and_ladder() {
    let assess = |fraction, weeks, events, noise, ranked| Assessment {
        ranked, no_share: None, shares: (0..weeks).map(|week| Share { week, complete: true, unit: Unit::FailureEvents,
            amount: events, total: events / fraction, noise }).collect(),
    };
    for (share, weeks, events, noise, ranked, impact) in [
        (0.14, 3, 8.0, 0.08, true, 2),
        (0.14, 3, 2.0, 0.08, true, 1),
        (0.14, 3, 8.0, 0.08, false, 0),
        (0.14, 1, 8.0, 0.08, true, 1),
        (0.12, 3, 8.0, 0.15, true, 1),
        (0.25, 3, 8.0, 0.28, true, 2),
        (0.15, 3, 8.0, 0.15, true, 1),
        (0.10, 3, 8.0, 0.10, true, 2),
        (0.02, 3, 8.0, 0.08, true, 0),
    ] {
        assert_eq!(assess(share, weeks, events, noise, ranked).score(false, false), impact,
            "{share}, {weeks}, {events}, {noise}, {ranked}");
    }
    let mut pair = Assessment { ranked: true, no_share: None, shares: [Unit::OwnerHours, Unit::Tokens].into_iter()
        .map(|unit| Share { week: 1, complete: true, unit, amount: 6.0, total: 100.0, noise: 0.0 }).collect() };
    assert_eq!(pair.score(false, false), 2);
    assert_eq!(pair.score(true, false), 1);
    pair.shares[1].week = 2;
    assert_eq!(pair.score(false, false), 1);
    assert_eq!(assess(0.01, 1, 1.0, 0.2, true).score(true, true), 3);
    let scores = Scores { impact: 1, blast: 2, reversibility: 0, confidence: 1.0 };
    assert_eq!(scores.level(), 2);
    assert_eq!(scores.response(false), "R4");
    assert!(scores.check_proposed_level(1).is_err());
    assert!(scores.check_proposed_level(3).is_ok());
    assert_eq!(Scores::default().response(true), "R1");
}

#[test]
fn ac213_records_per_finding_impact_and_reads_legacy_scores() {
    let f = Fixture::new("record-impact");
    f.task("earliest", None, "P2", "DONE", NOW - 9 * WEEK);
    f.task("repeat", Some("Bug"), "P2", "IN PROGRESS", NOW - 4 * WEEK);
    f.task("short", Some("Bug"), "P2", "IN PROGRESS", NOW - 4 * WEEK);
    for (week, base) in [0, 1, 2].into_iter().map(|w| (w, NOW - w * WEEK - 60)) {
        for n in 0..3 {
            f.failed_run("repeat", &format!("repeat-{week}-{n}"), base + 5 * n);
        }
    }
    for n in 0..2 {
        f.failed_run("short", &format!("short-{n}"), NOW - 24 + 5 * n);
    }
    let d = Fixture::detector("repeated_failures");
    evaluate(&f.store, PROJECT, NOW, &[d]).unwrap().record(&f.store).unwrap();
    let result = signals(&f.store, PROJECT).unwrap();
    assert_eq!(result.iter().map(|s| (s.subject_id.as_str(), s.scores.impact, s.level, s.response))
        .collect::<Vec<_>>(), [("repeat", 2, 2, "R4"), ("short", 1, 1, "R2")]);
    assert_eq!(result[0].impact_basis.shares.len(), 3);
    assert!(result[0].impact_basis.shares.iter().all(|s| s.amount == 3.0));
    f.write("UPDATE supervisor_signals SET scores = ?1 WHERE subject_id = 'short'",
        [r#"{"impact":0,"blast":0,"reversibility":0,"confidence":1.0}"#]);
    assert_eq!(signals(&f.store, PROJECT).unwrap()[1].response, "R0");
}

#[test]
fn ac213_one_week_with_two_additive_quantities_is_high() {
    let f = Fixture::new("paired-impact");
    f.task("earliest", None, "P2", "DONE", NOW - 9 * WEEK);
    f.task("repeat", None, "P2", "IN PROGRESS", NOW - WEEK);
    for n in 0..3 {
        let run = format!("failed-{n}");
        let at = NOW - 30 + 5 * n;
        f.failed_run("repeat", &run, at);
        f.cost(&run, at + 1, 200);
    }
    let d = Fixture::detector("repeated_failures");
    evaluate(&f.store, PROJECT, NOW, &[d]).unwrap().record(&f.store).unwrap();
    let signals = signals(&f.store, PROJECT).unwrap();
    let signal = &signals[0];
    assert_eq!(signal.scores.impact, 2);
    assert!(signal.impact_basis.shares.iter().any(|s| s.unit == Unit::FailureEvents && s.amount == 3.0));
    assert!(signal.impact_basis.shares.iter().any(|s| s.unit == Unit::Tokens && s.amount == 600.0));

    f.task("shared", None, "P2", "IN PROGRESS", NOW - WEEK);
    f.write("INSERT INTO run_tasks (run, task) SELECT id, 'shared' FROM runs", []);
    let d = Fixture::detector("repeated_failures");
    evaluate(&f.store, PROJECT, NOW + 1, &[d]).unwrap().record(&f.store).unwrap();
    let shared = super::signals(&f.store, PROJECT).unwrap();
    assert_eq!(shared[0].scores.impact, 1);
    assert!(shared[0].impact_basis.shares.iter().all(|s| s.unit == Unit::FailureEvents));
}

fn replay_args() -> Vec<String> {
    ["supervisor", "replay", "--project", PROJECT].map(str::to_owned).to_vec()
}

fn cli(path: &std::path::Path, args: &[String]) -> Result<String, String> {
    let mut context = crate::cli::Ctx::new(HashMap::new(), path.to_owned()).unwrap();
    context.app_dir = path.to_owned();
    context.captured = Some(Default::default());
    let cmd = crate::cli::parse(args)?;
    let status = crate::cli::run(&context, cmd)?;
    assert_eq!(status, 0);
    let captured = context.captured.take().unwrap().into_inner();
    assert_eq!(captured.len(), 1);
    Ok(captured.into_iter().next().unwrap())
}

#[test]
fn ac215_replay_rank_and_schema() {
    let f = Fixture::new("replay");
    let dir = f.path.parent().unwrap();
    f.task("history", Some("Story"), "P2", "DONE", NOW - 9 * WEEK);
    f.task("waiting", Some("Feature"), "P2", "IN PROGRESS", NOW - 4 * WEEK);
    f.transition("waiting", NOW - 3 * WEEK, "IN PROGRESS", "WAIT FOR HUMAN");
    f.task("resolved", None, "P2", "IN PROGRESS", NOW - 4 * WEEK);
    f.transition("resolved", NOW - 3 * WEEK, "IN PROGRESS", "WAIT FOR HUMAN");
    f.transition("resolved", NOW - 2 * WEEK, "WAIT FOR HUMAN", "DONE");
    f.task("budget", Some("Feature"), "P2", "IN PROGRESS", NOW - 14);
    f.task("reference", Some("Feature"), "P2", "IN PROGRESS", NOW - 5 * WEEK);
    f.run("budget", "spike", "worker", NOW - 10);
    f.run("reference", "normal", "worker", NOW - 5 * WEEK);
    f.cost("spike", NOW - 9, 3_000_000);
    f.cost("normal", NOW - 5 * WEEK + 1, 5_000);
    f.task_in(OTHER, "private-other", Some("Feature"), "P0", "WAIT FOR HUMAN", NOW - 5000);
    f.event("private-other", NOW + 10, "dispatcher", "move", "", Some(serde_json::json!({"to": "WAIT FOR HUMAN"})));

    // The store is in WAL mode already. Its database and WAL bytes must remain unchanged.
    let wal = f.path.with_extension("sqlite-wal");
    let before = (fs::read(&f.path).unwrap(), fs::read(&wal).unwrap());
    let report = cli(dir, &replay_args()).unwrap();
    assert_eq!(report, cli(dir, &replay_args()).unwrap());
    let wait = report.find("flow.wait_human_age waiting wait_minutes:").unwrap();
    let spend = report.find("flow.cost_outlier budget task_dollars:").unwrap();
    assert!(wait < spend, "{report}");
    assert!(report[wait..].lines().next().unwrap().contains("impact 2"));
    assert!(report[spend..].lines().next().unwrap().contains("impact 1"));
    assert!(report.contains("flow.main_red: CI probe history is unavailable."));
    assert!(report.contains("flow.queue_growth: Not enough retained snapshot points for this period."));
    assert!(!report.contains("private-other"));
    assert!(!report.contains("flow.wait_human_age resolved"));
    let mut past = replay_args();
    past.extend(["--at".into(), (NOW - 2 * WEEK - 1).to_string()]);
    let previous = cli(dir, &past).unwrap();
    assert!(previous.contains("flow.wait_human_age resolved wait_minutes:"));
    assert!(previous.contains("Historical queued presses and blocking links are unavailable."));
    assert!(report.contains("Release cadence: not measured yet"));
    assert_eq!(before, (fs::read(&f.path).unwrap(), fs::read(&wal).unwrap()));
    assert!(signals(&f.store, PROJECT).unwrap().is_empty());

    let old_dir = crate::testutil::fresh_dir("supervisor-replay-schema");
    let old = old_dir.join("kanban.sqlite");
    let conn = Connection::open(&old).unwrap();
    conn.pragma_update(None, "user_version", 1).unwrap();
    drop(conn);
    let before = fs::read(&old).unwrap();
    let message = cli(&old_dir, &replay_args()).unwrap_err();
    assert!(message.contains("schema is 1; Supervisor queries require"), "{message}");
    assert_eq!(before, fs::read(&old).unwrap());
    assert_eq!(fs::read_dir(&old_dir).unwrap().count(), 1);

    let empty = crate::testutil::fresh_dir("supervisor-replay-no-file");
    assert!(cli(&empty, &replay_args()).is_err());
    assert_eq!(fs::read_dir(empty).unwrap().count(), 0);
}
