// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! terminum-reports.md AC-276 and AC-277: a critical report as the Supervisor's critical signal, and reports as
//! its evidence.

use rusqlite::OptionalExtension;

use super::engine_tests::{Fixture, NOW, PROJECT};
use super::incidents::{self, Action};
use super::{DAY, Rule, evaluate};
use crate::reports::record::{self, New, Stamp, role};
use crate::tasks::ops::sql;
use crate::tasks::store::HUMAN;

fn enabled(f: &Fixture) {
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)", (PROJECT, NOW - 9 * DAY));
}

/// A report of `task` (none: of no task) made at `at`: its key.
fn report(f: &Fixture, task: Option<&str>, critical: bool, at: i64) -> String {
    let new = New { project: PROJECT.into(), title: "Build broken".into(), summary: "main is red".into(), body: "## What\nred".into(),
        critical, author: HUMAN.into(), role: role::WORKER, task: task.map(Into::into), ..New::default() };
    f.store.with(|tx| record::create(tx, new, at).map(|(key, _, _)| key)).unwrap()
}

/// The report's alerting incident and whether its own line stands.
fn alert(f: &Fixture, key: &str) -> (Option<i64>, bool) {
    f.store.with(|tx| tx.query_row("SELECT alerted_by, line FROM reports WHERE id = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)).unwrap()
}

fn incident(f: &Fixture, id: i64) -> (String, Option<i64>, Option<i64>) {
    f.store.with(|tx| tx.query_row("SELECT subject_id, acknowledged_at, closed_at FROM supervisor_incidents WHERE id = ?1", [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(sql)).unwrap()
}

fn read_at(f: &Fixture, key: &str) -> Option<i64> {
    f.store.with(|tx| tx.query_row("SELECT read_at FROM reports WHERE id = ?1", [key], |r| r.get(0)).map_err(sql)).unwrap()
}

fn reports(f: &Fixture) -> i64 {
    f.store.with(|tx| tx.query_row("SELECT count(*) FROM reports", [], |r| r.get(0)).map_err(sql)).unwrap()
}

#[test]
fn ac276_a_critical_report_opens_its_task_s_incident_and_alerts_once() {
    let f = Fixture::new("report-incident");
    enabled(&f);
    f.task("build", Some("Bug"), "P1", "IN PROGRESS", NOW - DAY);
    let first = report(&f, Some("build"), true, NOW);
    let (by, line) = alert(&f, &first);
    let id = by.expect("an incident alerts for it");
    assert_eq!(incident(&f, id).0, "build", "the subject is the report's task");
    assert!(!line, "its own line stays down: one alert, the incident's");
    assert!(f.store.with(record::lines).unwrap().is_empty());
    assert_eq!(reports(&f), 1, "the incident makes no second report of its own");
    let level: u8 = f.store.with(|tx| tx.query_row("SELECT level FROM supervisor_incident_signals WHERE incident = ?1", [id], |r| r.get(0)).map_err(sql)).unwrap();
    assert_eq!(level, 3, "critical: the author's level");

    // a second one on the same task in the same second joins it; both are its reports
    let second = report(&f, Some("build"), true, NOW);
    assert_eq!(alert(&f, &second), (Some(id), false));
    assert_eq!(alert(&f, &first).0, Some(id));

    // another task's report opens its own incident and leaves the first open (one subject measured)
    f.task("deploy", Some("Bug"), "P1", "IN PROGRESS", NOW - DAY);
    let other = report(&f, Some("deploy"), true, NOW);
    let other_id = alert(&f, &other).0.expect("its own incident");
    assert_ne!(other_id, id);
    assert_eq!(incident(&f, id).2, None, "task build's incident stays open");
}

/// The owner's Acknowledge press: the incident acknowledged, its reports read (incidents_api.rs).
fn acknowledge(f: &Fixture, id: i64, at: i64) {
    f.store.with(|tx| { assert!(incidents::act(tx, id, Action::Acknowledge, at)?); record::read_of_incident(tx, id, at) }).unwrap();
}

fn open_incidents(f: &Fixture) -> i64 {
    f.store.with(|tx| tx.query_row("SELECT count(*) FROM supervisor_incidents WHERE closed_at IS NULL", [], |r| r.get(0)).map_err(sql)).unwrap()
}

#[test]
fn ac276_acknowledging_reads_reading_acknowledges_and_a_snooze_keeps_them_unread() {
    let f = Fixture::new("report-ack");
    enabled(&f);
    f.task("build", Some("Bug"), "P1", "IN PROGRESS", NOW - DAY);
    let key = report(&f, Some("build"), true, NOW);
    let id = alert(&f, &key).0.unwrap();
    assert!(f.store.with(|tx| incidents::act(tx, id, Action::Snooze, NOW + 60)).unwrap());
    assert_eq!(read_at(&f, &key), None, "a snooze leaves its report unread");
    f.live(Rule::ReportCritical, NOW + 120).record(&f.store).unwrap();
    assert_eq!(incident(&f, id).2, None, "so the incident stays open and its repeats come back");
    acknowledge(&f, id, NOW + 180);
    assert_eq!(read_at(&f, &key), Some(NOW + 180), "acknowledging reads its report");

    f.task("deploy", Some("Bug"), "P1", "IN PROGRESS", NOW - DAY);
    let first = report(&f, Some("deploy"), true, NOW + 300);
    let second = report(&f, Some("deploy"), true, NOW + 301);
    let id = alert(&f, &first).0.unwrap();
    f.store.with(|tx| {
        assert!(record::stamp(tx, &first, Stamp::Read, Some(NOW + 400))?);
        assert!(record::acknowledge_by_reading(tx, &first, NOW + 400)?, "reading one acknowledges the incident");
        assert!(!record::acknowledge_by_reading(tx, &first, NOW + 500)?, "once");
        let by: Option<String> = tx.query_row("SELECT actor FROM supervisor_incident_events WHERE incident = ?1 AND kind = 'acknowledged'",
            [id], |r| r.get(0)).optional().map_err(sql)?;
        assert_eq!(by.as_deref(), Some(HUMAN), "recorded as the owner's");
        Ok(())
    }).unwrap();
    assert_eq!(incident(&f, id).1, Some(NOW + 400));
    assert_eq!(read_at(&f, &second), None, "its other report stays unread: reading one reads no other");
}

#[test]
fn ac276_reading_clears_the_signal_and_a_later_report_joins_the_new_incident() {
    let f = Fixture::new("report-clear");
    enabled(&f);
    f.task("build", Some("Bug"), "P1", "IN PROGRESS", NOW - DAY);
    let key = report(&f, Some("build"), true, NOW);
    let id = alert(&f, &key).0.unwrap();
    f.live(Rule::ReportCritical, NOW + 60).record(&f.store).unwrap();
    assert_eq!(incident(&f, id).2, None, "re-measured unread: it stays");
    f.store.with(|tx| record::stamp(tx, &key, Stamp::Read, Some(NOW + 100))).unwrap();
    f.live(Rule::ReportCritical, NOW + 120).record(&f.store).unwrap();
    assert!(incident(&f, id).2.is_some(), "read: the signal resolves and the incident closes");

    // past the grouping window, a later report opens a new incident and takes the subject's unread reports along
    let late = report(&f, Some("build"), true, NOW + DAY);
    let later = report(&f, Some("build"), true, NOW + 2 * DAY);
    let newest = alert(&f, &later).0.unwrap();
    assert_eq!(alert(&f, &late).0, Some(newest), "the unread earlier report points at the incident that alerts now");
}

#[test]
fn ac276_with_the_supervisor_off_or_for_an_incident_s_own_report_the_line_stands() {
    let f = Fixture::new("report-off");
    f.task("build", Some("Bug"), "P1", "IN PROGRESS", NOW - DAY);
    let key = report(&f, Some("build"), true, NOW);
    assert_eq!(alert(&f, &key), (None, true), "no Supervisor: its own line alerts (AC-271)");

    enabled(&f);
    let own = f.store.with(|tx| {
        let id: i64 = tx.query_row("INSERT INTO supervisor_incidents (project, subject_kind, subject_id, opened_at, last_firing_at)
            VALUES (?1, 'project', ?1, ?2, ?2) RETURNING id", (PROJECT, NOW), |r| r.get(0)).map_err(sql)?;
        crate::reports::sources::incident(tx, PROJECT, id, "project default", "why", NOW)?;
        tx.query_row("SELECT id FROM reports WHERE incident = ?1", [id], |r| r.get::<_, String>(0)).map_err(sql)
    }).unwrap();
    assert_eq!(alert(&f, &own).0, None, "an incident never feeds itself");
    let signals: i64 = f.store.with(|tx| tx.query_row("SELECT count(*) FROM supervisor_signals WHERE detector = 'reports.critical'", [], |r| r.get(0)).map_err(sql)).unwrap();
    assert_eq!(signals, 0);
}

#[test]
fn ac276_a_report_alerts_once_whatever_happens_to_it_later() {
    let f = Fixture::new("report-once");
    // made with the Supervisor off, it alerted by its own line: turning the Supervisor on pages nothing more
    f.task("build", Some("Bug"), "P1", "IN PROGRESS", NOW - DAY);
    let lined = report(&f, Some("build"), true, NOW - 600);
    enabled(&f);
    f.live(Rule::ReportCritical, NOW).record(&f.store).unwrap();
    assert_eq!((alert(&f, &lined), open_incidents(&f)), ((None, true), 0), "its line alerted; no incident");

    // one of two deleted, past the grouping window: the incident is updated, not a new one
    f.task("deploy", Some("Bug"), "P1", "IN PROGRESS", NOW - DAY);
    let first = report(&f, Some("deploy"), true, NOW);
    let second = report(&f, Some("deploy"), true, NOW + 1);
    let id = alert(&f, &first).0.unwrap();
    f.store.with(|tx| record::stamp(tx, &first, Stamp::Deleted, Some(NOW + 3600))).unwrap();
    f.live(Rule::ReportCritical, NOW + 3600).record(&f.store).unwrap();
    assert_eq!((alert(&f, &second).0, open_incidents(&f)), (Some(id), 1), "the remaining report stays with its open incident");

    // read, the incident closes; made unread again, it raises nothing
    acknowledge(&f, id, NOW + 3700);
    f.live(Rule::ReportCritical, NOW + 3800).record(&f.store).unwrap();
    assert!(incident(&f, id).2.is_some());
    f.store.with(|tx| record::stamp(tx, &second, Stamp::Read, None)).unwrap();
    f.live(Rule::ReportCritical, NOW + 3900).record(&f.store).unwrap();
    assert_eq!(open_incidents(&f), 0, "unread again after its incident closed: no second alert");
    let postmortems: i64 = f.store.with(|tx| tx.query_row("SELECT count(*) FROM supervisor_reflections WHERE triggers LIKE '%incident_postmortem%'",
        [], |r| r.get(0)).map_err(sql)).unwrap();
    assert_eq!(postmortems, 0, "a critical report's incident gets no postmortem round: its author said what happened");

    // a Supervisor round's own critical report never feeds an incident
    let own = f.store.with(|tx| record::create(tx, New { project: PROJECT.into(), title: "Round".into(), summary: "s".into(), body: "b".into(),
        critical: true, author: HUMAN.into(), role: role::SUPERVISOR, ..New::default() }, NOW + 4000).map(|(key, _, _)| key)).unwrap();
    assert_eq!(alert(&f, &own).0, None);
    assert_eq!(open_incidents(&f), 0);
}

#[test]
fn ac276_a_collector_pass_read_before_the_report_cannot_silence_it() {
    let f = Fixture::new("report-race");
    enabled(&f);
    f.task("build", Some("Bug"), "P1", "IN PROGRESS", NOW - DAY);
    let early = f.live(Rule::ReportCritical, NOW);        // the collector's snapshot, before the report is kept
    let key = report(&f, Some("build"), true, NOW - 1);
    let first = alert(&f, &key).0.unwrap();
    early.record(&f.store).unwrap();
    assert!(incident(&f, first).2.is_some(), "the stale pass resolved it, unacknowledged");
    f.live(Rule::ReportCritical, NOW + 60).record(&f.store).unwrap();
    let (by, line) = alert(&f, &key);
    let again = by.expect("it alerts again");
    assert!(again != first && incident(&f, again).2.is_none() && !line, "a new open incident alerts for the unread report");
}

#[test]
fn ac276_a_report_made_critical_by_update_report_opens_its_incident() {
    let f = Fixture::new("report-update");
    enabled(&f);
    f.task("build", Some("Bug"), "P1", "IN PROGRESS", NOW - DAY);
    let key = report(&f, Some("build"), false, NOW);
    // the project's Supervisor at Operator may change its reports (AC-280)
    let manager = crate::tasks::tools::Who { conversation: None, key: "tmux:local:terminum:%9".into(), agent: "supervisor".into(), title: "Supervisor".into(),
        project: Some(PROJECT.into()), dir: None, supervisor: true };
    let id: String = f.store.with(|tx| tx.query_row("SELECT slug FROM reports WHERE id = ?1", [&key], |r| r.get(0)).map_err(sql)).unwrap();
    crate::reports::tools::run(&f.store, &f.path, &manager, "update_report", &serde_json::json!({"id": id, "critical": "yes"})).unwrap();
    let (by, line) = alert(&f, &key);
    assert!(by.is_some() && !line, "its incident alerts for it, its line not");
    crate::reports::tools::run(&f.store, &f.path, &manager, "update_report", &serde_json::json!({"id": id, "critical": "no"})).unwrap();
    f.live(Rule::ReportCritical, NOW + 60).record(&f.store).unwrap();
    assert_eq!(open_incidents(&f), 0, "no longer critical: its signal clears");
}

#[test]
fn ac277_unread_reports_and_unanswered_replies_are_readings_and_owner_waits_stay_questions() {
    let f = Fixture::new("report-evidence");
    let fired = |name: &str, at: i64| -> Vec<(String, f64)> {
        evaluate(&f.store, PROJECT, at, &[Fixture::detector(name)]).unwrap().detectors[0].findings.iter()
            .map(|found| (found.subject_id.clone(), found.measure.value)).collect()
    };
    let key = report(&f, None, false, NOW - 2 * DAY);
    assert_eq!(fired("reports.unread_age", NOW), [(PROJECT.to_string(), 2880.0)], "the oldest unread report, in minutes");
    // AC-213: the unread-report area still has a level without an additive impact share.
    f.live(Rule::UnreadReports, NOW).record(&f.store).unwrap();
    let signal = super::signals(&f.store, PROJECT).unwrap().remove(0);
    assert_eq!((signal.scores.impact, signal.scores.blast, signal.scores.reversibility, signal.level), (0, 1, 0, 1));
    assert!(signal.impact_basis.shares.is_empty() && signal.impact_basis.no_share.is_some());
    assert!(fired("reports.unread_age", NOW - 2 * DAY + 60).is_empty(), "quiet under a day");
    assert!(fired("flow.wait_human_age", NOW).is_empty(), "a report never waits in WAIT FOR HUMAN");

    f.write("UPDATE reports SET replied_at = ?1 WHERE id = ?2", (NOW - 2 * DAY, &key));
    assert_eq!(fired("reports.follow_up_age", NOW), [(key.clone(), 2880.0)], "a reply no agent answered");
    f.agent("agent");
    f.write("INSERT INTO report_notes (id, report, author, kind, text, at) VALUES ('n1', ?1, 'agent:pool:agent', 'comment', 'done', ?2)", (&key, NOW - DAY));
    assert!(fired("reports.follow_up_age", NOW).is_empty(), "answered by its agent's comment");

    f.store.with(|tx| record::stamp(tx, &key, Stamp::Read, Some(NOW))).unwrap();
    assert!(fired("reports.unread_age", NOW).is_empty(), "read: quiet");
}
