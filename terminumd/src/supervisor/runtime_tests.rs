// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-216, AC-217, AC-218, AC-219: project scope, committed cursor, retained inputs and watchdog.

use std::sync::{Arc, mpsc};
use std::thread;

use rusqlite::Params;
use serde_json::json;

use super::*;
use crate::supervisor::{self, WEEK, replay, engine_tests::{Fixture, NOW, OTHER, PROJECT}, signals::State};

fn write(store: &Store, sql: &str, args: impl Params) {
    store.with(|tx| tx.execute(sql, args).map(drop).map_err(crate::tasks::ops::sql)).unwrap();
}

fn ready(store: &Store) -> Project {
    store.snapshot(SCHEMA, enabled).unwrap().remove(0)
}

fn value(store: &Store, sql: &str) -> i64 {
    store.snapshot(SCHEMA, |tx| tx.query_row(sql, [], |r| r.get(0)).map_err(crate::tasks::ops::sql)).unwrap()
}

fn failed(store: &Store, id: &str) {
    write(store, "INSERT INTO events (id, task, at, author, kind, data)
        VALUES (?1, 'work', ?2, 'human', 'move', '{\"to\":\"FAILED\"}')", (id, NOW));
}

#[tokio::test]
async fn ac216_project_off_is_inert_and_a_new_generation_rejects_old_work() {
    let (_, app) = crate::testutil::app_with_tasks("supervisor-off");
    let store = &app.tasks;
    assert_eq!(set_enabled(store, PROJECT, false, NOW).unwrap(), None);
    assert_eq!(value(store, "SELECT count(*) FROM project_supervisor"), 0);
    assert!(set_enabled(store, "missing", true, NOW).is_err());

    set_enabled(store, PROJECT, true, NOW).unwrap();
    let project = ready(store);
    let batch = store.snapshot(SCHEMA, |tx| read_batch(tx, &app, &project, NOW)).unwrap();
    let pending = app.supervisor.sensors.take(PROJECT, project.generation);
    write(store, "INSERT INTO supervisor_changes (project, source, source_id)
        VALUES (?1, 'tasks', 'pending-incident-card')", [PROJECT]);
    set_enabled(store, PROJECT, false, NOW).unwrap();
    assert_eq!(value(store, "SELECT count(*) FROM supervisor_changes"), 0);
    assert!(!commit(store, &project, batch, &pending, || NOW).unwrap().0.0);
    set_enabled(store, PROJECT, true, NOW).unwrap();
    let stale = Batch { at: NOW, cursor: 100, evaluation: None, points: vec![], sampled: false, baseline: false,
        settings_revision: 0, attachment: None, first_attachment: false };
    assert!(!commit(store, &project, stale, &pending, || NOW).unwrap().0.0);
    assert_eq!(value(store, "SELECT cursor FROM project_supervisor"), 0);
    assert_eq!(value(store, "SELECT count(*) FROM supervisor_points"), 0);
    assert_eq!(value(store, "SELECT count(*) FROM supervisor_evaluations"), 0);
}

#[tokio::test]
async fn ac217_cursor_restart_and_rollback_with_same_time_nonmonotonic_ids() {
    let (root, app) = crate::testutil::app_with_tasks("supervisor-cursor");
    let store = &app.tasks;
    write(store, "INSERT INTO tasks (id, number, project, status, severity, rank, owner, created_at, updated_at, title, body)
        VALUES ('work', 1, 'default', 'FAILED', 'P2', 0, 'human', ?1, ?1, 'Work', '')", [NOW]);
    failed(store, "before-enable");
    assert_eq!(value(store, "SELECT count(*) FROM supervisor_changes"), 0);
    // Use a distinct timestamp for the historic event so it is outside the failure window.
    write(store, "UPDATE events SET at = at - 60 * 86400 WHERE id = 'before-enable'", []);
    set_enabled(store, PROJECT, true, NOW).unwrap();
    failed(store, "z-first");
    let first = ready(store);
    collect_project(&app, &first, || NOW + 1).unwrap();
    let advanced = ready(store).cursor;
    assert!(advanced > first.cursor);
    assert!(supervisor::signals(store, PROJECT).unwrap().is_empty());

    let reopened = Store::open(root.join("state/kanban.sqlite"));
    let second = ready(&reopened);
    assert_eq!(second.cursor, advanced);
    failed(store, "a-second");
    failed(store, "m-third");
    let sources = store.snapshot(SCHEMA, |tx| rows(tx, "SELECT source_id FROM supervisor_changes
        WHERE project = ?1 AND seq > ?2 ORDER BY seq", (PROJECT, second.cursor), |r| r.get::<_, String>(0))).unwrap();
    assert_eq!(sources, ["a-second", "m-third"]);
    write(store, "CREATE TRIGGER fail_signal BEFORE INSERT ON supervisor_signals
        BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END", []);
    let err = collect_project(&app, &second, || NOW + 2).unwrap_err();
    assert!(err.contains("fixture write failure"), "{err}");
    assert_eq!(ready(&reopened).cursor, advanced);
    assert!(supervisor::signals(&reopened, PROJECT).unwrap().is_empty());
    write(store, "DROP TRIGGER fail_signal", []);
    collect_project(&app, &second, || NOW + 3).unwrap();

    let committed = ready(&reopened);
    assert!(committed.cursor > advanced);
    assert_eq!(value(store, "SELECT count(*) FROM supervisor_changes"), 0);
    collect_project(&app, &committed, || NOW + 4).unwrap();
    let got = supervisor::signals(&reopened, PROJECT).unwrap();
    let signal = got.iter().find(|s| s.detector == "flow.failed_no_reason").unwrap();
    assert_eq!((signal.measure.value, signal.count), (3.0, 1));
    for id in ["z-first", "a-second", "m-third"] {
        assert!(signal.evidence.contains(&Evidence::Events(id.into())));
    }

    let at = std::cell::Cell::new(NOW + 5);
    let clock = || {
        let current = at.get();
        if current == NOW + 5 {
            failed(store, "after-snapshot");
            at.set(NOW + 6);
        }
        current
    };
    collect_project(&app, &ready(store), clock).unwrap();
    assert_eq!(ready(store).cursor, committed.cursor);
    assert_eq!(ready(store).last_ok, Some(NOW + 6));
    collect_project(&app, &ready(store), || NOW + 7).unwrap();
    assert!(ready(store).cursor > committed.cursor);
    let got = supervisor::signals(store, PROJECT).unwrap();
    let signal = got.iter().find(|s| s.detector == "flow.failed_no_reason").unwrap();
    assert!(signal.evidence.contains(&Evidence::Events("after-snapshot".into())));
}

#[test]
fn ac217_open_detector_snapshot_does_not_take_the_store_write_turn() {
    let f = Fixture::new("concurrent");
    let store = Arc::new(f.store);
    let (opened, hold) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let reader = store.clone();
    let handle = thread::spawn(move || reader.snapshot(SCHEMA, |tx| {
        assert!(tx.execute("DELETE FROM events", []).is_err());
        opened.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(5)).unwrap();
        let seen: i64 = tx.query_row("SELECT count(*) FROM meta WHERE key = 'fixture'", [], |r| r.get(0)).unwrap();
        assert_eq!(seen, 0);
        Ok(())
    }).unwrap());
    hold.recv_timeout(Duration::from_secs(5)).unwrap();
    write(&store, "INSERT INTO meta VALUES ('fixture', 'made while the reader waited')", []);
    release.send(()).unwrap();
    handle.join().unwrap();
}

#[tokio::test]
async fn ac218_points_and_hook_counter_retention() {
    let (_, app) = crate::testutil::app_with_tasks("supervisor-samples");
    let s = &app.tasks;
    write(s, "INSERT INTO tasks (id, number, project, status, severity, rank, owner, created_at, updated_at, title, body)
        VALUES ('work', 1, 'default', 'TODO', 'P2', 0, 'human', ?1, ?1, 'Work', '')", [NOW - 1000]);
    write(s, "INSERT INTO project_members VALUES ('default', 'test-pane', 0)", []);
    write(s, "INSERT INTO run_waits (task, agent_id, project, status, body, at)
        VALUES ('work', 'fixture', 'default', 'TODO', '', ?1)", [NOW - 300]);
    set_enabled(s, PROJECT, true, NOW).unwrap();
    let projects = app.supervisor.read_scopes(s, NOW).unwrap();
    collect_project(&app, &projects[0], || NOW).unwrap();
    assert_eq!(value(s, "SELECT cast(value AS INTEGER) FROM supervisor_points WHERE metric = 'queue_oldest_minutes'"), 5);
    assert_eq!(value(s, "SELECT at FROM supervisor_points WHERE metric = 'queue_waits'"), NOW);
    write(s, "DELETE FROM run_waits WHERE task = 'work'", []);
    collect_project(&app, &ready(s), || NOW + samples::CADENCE - 1).unwrap();
    assert_eq!(value(s, "SELECT count(*) FROM supervisor_points WHERE metric = 'queue_waits'"), 1);
    assert_eq!(value(s, "SELECT count(*) FROM supervisor_points WHERE metric = 'queue_waits_p90'"), 1);

    for event in ["PreToolUse", "PreToolUse", "PostToolUseFailure", "SubagentStart", "SubagentStop", "PreCompact"] {
        app.supervisor.sensors.heard(PROJECT, "test-pane", &json!({"agent":"claude", "session":"counted", "event":event,
            "tool_name_hash":"abcdef1234567890", "tool":"Read: private fixture input", "output":"fixture output"}), NOW);
    }
    app.supervisor.sensors.heard(OTHER, "other-pane", &json!({"agent":"claude", "session":"ignored", "event":"Stop"}), NOW);
    app.supervisor.sensors.heard(PROJECT, "moved-pane", &json!({"agent":"claude", "session":"moved", "event":"Stop"}), NOW);
    let later = NOW + samples::CADENCE;
    let failed_flush = app.supervisor.sensors.take(PROJECT, projects[0].generation);
    drop(failed_flush);
    collect_project(&app, &ready(s), || later).unwrap();
    assert_eq!(value(s, "SELECT sum(count) FROM supervisor_hook_counts WHERE session = 'counted'"), 7);
    assert_eq!(value(s, "SELECT count(*) FROM supervisor_hook_counts WHERE session != 'counted'"), 0);
    assert_eq!(value(s, "SELECT sum(count) FROM supervisor_hook_counts WHERE event IN ('repeat', 'failure', 'compaction')"), 3);
    assert_eq!(value(s, "SELECT count(*) FROM supervisor_points WHERE metric = 'queue_waits'"), 2);
    assert_eq!(value(s, "SELECT cast(value AS INTEGER) FROM supervisor_points WHERE metric = 'queue_waits' AND at > 1800000000"), 0);

    let path = app.cfg.dir.join("kanban.sqlite");
    let before = supervisor::replay(&path, PROJECT, Some(NOW - 1)).unwrap();
    assert!(before.contains("drift.loop: No retained hook counters for this period."));
    let after = supervisor::replay(&path, PROJECT, Some(later)).unwrap();
    assert!(!after.contains("drift.loop: No retained hook counters for this period."));
    let daily = NOW + DAY;
    collect_project(&app, &ready(s), || daily).unwrap();
    assert_eq!(value(s, "SELECT count(*) FROM supervisor_points WHERE metric = 'queue_waits_p90'"), 2);
    collect_project(&app, &ready(s), || NOW + 31 * DAY).unwrap();
    assert_eq!(value(s, "SELECT count(*) FROM supervisor_hook_counts"), 0);
}

#[test]
fn ac218_queue_growth_uses_only_the_first_available_samples() {
    let f = Fixture::new("queue-trend");
    set_enabled(&f.store, PROJECT, true, NOW).unwrap();
    for (at, value) in [(NOW, 1.0), (NOW + samples::CADENCE, 2.0), (NOW + 2 * samples::CADENCE, 4.0)] {
        f.write("INSERT INTO supervisor_points (project, metric, subject, at, value) VALUES ('default', 'queue_waits', '', ?1, ?2)", (at, value));
    }
    let d = Fixture::detector("flow.queue_growth");
    let earlier = supervisor::evaluate(&f.store, PROJECT, NOW + samples::CADENCE, std::slice::from_ref(&d)).unwrap();
    assert!(earlier.detectors[0].findings.is_empty());
    let current = supervisor::evaluate(&f.store, PROJECT, NOW + 2 * samples::CADENCE, &[d]).unwrap();
    let finding = &current.detectors[0].findings[0];
    assert_eq!(finding.measure.value, 3.0);
    assert_eq!(finding.evidence.len(), 3);
}

#[test]
fn ac218_unobserved_subscription_does_not_clear_its_signal() {
    let f = Fixture::new("partial-limit");
    set_enabled(&f.store, PROJECT, true, NOW).unwrap();
    let reset = NOW + 24 * 3600;
    let insert = |subject, at, value| f.write(
        "INSERT INTO supervisor_points (project, metric, subject, at, value, resets_at)
        VALUES ('default', 'subscription_used', ?1, ?2, ?3, ?4)", (subject, at, value, reset));
    for subject in ["claude/300", "codex/300"] {
        insert(subject, NOW, 20.0);
        insert(subject, NOW + samples::CADENCE, 70.0);
    }
    let rule = Fixture::detector("flow.limit_near");
    let run = |at| supervisor::evaluate(&f.store, PROJECT, at, std::slice::from_ref(&rule))
        .unwrap().record(&f.store).unwrap();
    run(NOW + samples::CADENCE);
    assert_eq!(supervisor::signals(&f.store, PROJECT).unwrap().len(), 2);
    insert("claude/300", NOW + 6 * samples::CADENCE, 10.0);
    insert("claude/300", NOW + 7 * samples::CADENCE, 10.0);
    run(NOW + 7 * samples::CADENCE);
    let found = supervisor::signals(&f.store, PROJECT).unwrap();
    assert_eq!(found[0].state, State::Resolved);
    assert_eq!(found[1].state, State::New);
}

#[tokio::test]
async fn ac219_independent_watchdog_reports_two_misses_once_and_honors_recovery() {
    let (_, app) = crate::testutil::app_with_tasks("supervisor-watch");
    let s = &app.tasks;
    set_enabled(s, PROJECT, true, NOW).unwrap();
    let project = ready(s);
    collect_project(&app, &project, || NOW).unwrap();
    let live = ready(s);
    check_heartbeat(s, NOW + BEAT).unwrap();
    assert!(supervisor::signals(s, PROJECT).unwrap().is_empty());
    check_heartbeat(s, NOW + 2 * BEAT).unwrap();
    check_heartbeat(s, NOW + 3 * BEAT).unwrap();
    let fault = supervisor::signals(s, PROJECT).unwrap().pop().unwrap();
    assert_eq!((&*fault.detector, fault.level, fault.count), ("self.collector_dead", 3, 1));
    assert_eq!(fault.evidence, [Evidence::SupervisorHeartbeat { project: PROJECT.into(), last_ok: Some(NOW) }]);
    assert!(supervisor::signals(s, OTHER).unwrap().is_empty());
    collect_project(&app, &ready(s), || NOW + 3 * BEAT).unwrap();
    report_dead(s, &live, NOW + 3 * BEAT).unwrap();
    assert_eq!(supervisor::signals(s, PROJECT).unwrap()[0].state, State::Resolved);
}

#[tokio::test]
async fn ac221_attach_replays_distinct_verdicts_seeds_problems_and_retires_disabled_rule() {
    let (root, app) = crate::testutil::app_with_tasks("supervisor-attach");
    let fixture = Fixture::in_path(root.join("state/kanban.sqlite"));
    let store = &app.tasks;
    fixture.write("UPDATE projects SET dir = '' WHERE id = ?1", [PROJECT]);
    fixture.task("history", None, "P2", "DONE", NOW - 7 * WEEK);
    fixture.task("work", None, "P2", "FAILED", NOW - 5 * WEEK);
    for week in 0..=4 {
        for offset in 0..3 {
            fixture.transition("work", NOW - week * WEEK - 60 + offset, "IN PROGRESS", "FAILED");
        }
    }
    let rule = Rule::FailedWithoutReason;
    let name = rule.id();
    let preferences = |mode| serde_json::from_value(json!({
        "detectors": { (name): { "mode": mode, "threshold": { "absolute": 1 } } }
    })).unwrap();
    store.with(|tx| settings::write(tx, PROJECT, 0, preferences("live"), false)).unwrap();
    let past = store.snapshot(SCHEMA, |tx| {
        let detector = settings::read(tx, PROJECT)?.settings.detector(rule);
        let mut past = std::collections::BTreeSet::new();
        replay::visit_history(tx, PROJECT, NOW - 7 * WEEK, NOW, &[detector], |rule, f| {
            past.insert(replay::firing_key(PROJECT, rule, f)?);
            Ok(())
        })?;
        Ok(past)
    }).unwrap();
    assert_eq!(past.len(), 3);
    for (i, firing) in past.iter().enumerate() {
        fixture.write("INSERT INTO supervisor_hold_verdicts VALUES (?1, ?2, ?3, ?4, ?5)",
            (PROJECT, name, firing, NOW - 2 * WEEK, i < 2));
    }
    fixture.write("INSERT INTO supervisor_hold_verdicts VALUES (?1, ?2, ?3, ?4, 0)",
        (OTHER, name, past.first().unwrap(), NOW - 2 * WEEK));
    fixture.write("INSERT INTO supervisor_hold_verdicts VALUES (?1, ?2, 'no-replay-match', ?3, 0)",
        (PROJECT, name, NOW - 2 * WEEK));

    set_enabled(store, PROJECT, true, NOW).unwrap();
    collect_project(&app, &ready(store), || NOW).unwrap();
    let report = || store.snapshot(SCHEMA, |tx| attach::read(tx, PROJECT)).unwrap().unwrap();
    let status = report().status(NOW, true).unwrap();
    let scan = &status["detectors"][name];
    assert_eq!(scan["replay_firings"], 3);
    assert_eq!(scan["holds"]["replay_judged"], 3);
    assert_eq!(scan["holds"]["replay_right"], 2);
    assert_eq!(scan["hold_state"], "eligible");
    assert_eq!(value(store, "SELECT count(*) FROM supervisor_attach_seeds
        JOIN supervisor_signals USING (fingerprint) WHERE detector = 'flow.failed_no_reason'"), 1);
    assert_eq!(report().status(NOW, false).unwrap()["detectors"][name]["hold_state"], "inactive");

    // No collector beat may see the off setting. Its authority is retired in the write transaction.
    store.with(|tx| settings::write(tx, PROJECT, 1, preferences("off"), false)).unwrap();
    store.with(|tx| settings::write(tx, PROJECT, 2, preferences("live"), false)).unwrap();
    store.with(|tx| settings::write(tx, PROJECT, 3, preferences("live"), false)).unwrap();
    collect_project(&app, &ready(store), || NOW + WEEK + 1).unwrap();
    let revised = report().status(NOW + WEEK + 1, true).unwrap();
    assert_eq!(revised["detectors"][name]["mode"], "shadow");
    assert_eq!(revised["detectors"][name]["hold_state"], "inactive");
    assert_eq!(value(store, "SELECT count(*) FROM supervisor_attach_seeds
        JOIN supervisor_signals USING (fingerprint) WHERE detector = 'flow.failed_no_reason'"), 1);
}

#[tokio::test]
async fn ac254_collector_resumes_at_mute_expiry_without_new_events() {
    let (root, app) = crate::testutil::app_with_tasks("supervisor-mute-clock");
    let f = Fixture::in_path(root.join("state/kanban.sqlite"));
    f.task("repeat", Some("Bug"), "P2", "IN PROGRESS", NOW - 2 * WEEK);
    for i in 0..3 {
        f.failed_run("repeat", &format!("failed-{i}"), NOW - 60 + 2 * i);   // a launch apart from the previous FAILED second
    }
    let store = &app.tasks;
    set_enabled(store, PROJECT, true, NOW).unwrap();
    collect_project(&app, &ready(store), || NOW).unwrap();
    let rule = Rule::FailedRepeat;
    let until = NOW + 40;
    let change = json!({"detector":rule.id(),"action":{
        "operation":"mute","mode":"shadow","reason":"Review repeated failures","until":until}});
    store.with(|tx| {
        use supervisor::config_changes as config;
        let replay = config::preview(tx, PROJECT, "test-round", serde_json::from_value(change.clone()).unwrap(), NOW + 1)?;
        let proposal = serde_json::from_value(json!({"change":change,"replay":replay["replay"]})).unwrap();
        let verified = config::verify(tx, PROJECT, "test-round", proposal, NOW + 1)?;
        config::submit(tx, PROJECT, "test-round", verified, NOW + 1)?;
        Ok(())
    }).unwrap();
    let state = || supervisor::signals(store, PROJECT).unwrap().into_iter()
        .find(|s| s.detector == rule.id()).unwrap().state;
    collect_project(&app, &ready(store), || until - 1).unwrap();
    assert_eq!(state(), State::Shadow);
    let before = ready(store);
    collect_project(&app, &before, || until).unwrap();
    assert_eq!(ready(store).cursor, before.cursor);
    assert_eq!(state(), State::New);
}

#[tokio::test]
async fn ac221_integration_switch_rescans_the_attachment_and_a_quiet_beat_keeps_it() {
    let (root, app) = crate::testutil::app_with_tasks("supervisor-rescan");
    let fixture = Fixture::in_path(root.join("state/kanban.sqlite"));
    let store = &app.tasks;
    set_enabled(store, PROJECT, true, NOW).unwrap();
    collect_project(&app, &ready(store), || NOW).unwrap();
    let scanned = || value(store, "SELECT json_extract(attachment, '$.scanned_at') FROM project_supervisor");
    assert_eq!(scanned(), NOW);
    write(store, "UPDATE project_supervisor SET attachment = json_set(attachment, '$.scanned_at', 0)", []);

    // A beat with no scan source writes no attachment: an edit made between scans stays (TERM-841).
    collect_project(&app, &ready(store), || NOW + 20).unwrap();
    assert_eq!(scanned(), 0);
    // Integration's switch is a scan source: the next beat recomputes the attachment from real inputs.
    fixture.integration(true);
    collect_project(&app, &ready(store), || NOW + 40).unwrap();
    assert_eq!(scanned(), NOW + 40);
}
