// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-233 and AC-225: a low-confidence verification's interim hold, its comparison, its deadline and its end.

use serde_json::{Value, json};

use crate::tasks::ops::{self, sql};
use super::{Rule, WEEK, engine_tests::{Fixture, PROJECT}, incidents_api, interim, line_queue, signals};

const KEY: &str = "verify-signal";
const RUN: &str = "verify-run";

/// An Operator project whose rule is live for holds, with integration on and `integrated` tasks landed this week.
fn project(f: &Fixture, integrated: usize, at: i64) {
    let report = json!({
        "attached_at": at - WEEK, "scanned_at": at, "first_observation": at - 4 * WEEK,
        "limited_history": false, "code_only": true,
        "sources": {"integration": true, "code_host": false, "pool_agents": 0, "routines": 0},
        "detectors": {(Rule::FailedRepeat.id()): {"mode":"live","can_run":true,"unavailable":[],
            "threshold":null,"observed_thresholds":{},"replay_firings":0,
            "holds":{"live_at_attach":true,"replay_judged":0,"replay_right":0}}}
    });
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at, last_ok, attachment, settings_revision)
        VALUES (?1, 1, 1, ?2, ?2, ?3, 0)", (PROJECT, at, report.to_string()));
    f.write("INSERT INTO project_integration (project, is_on, target, check_line, publish_line, after_line, batch_most,
        check_minutes, agent_budget_minutes, red_rounds_most, prompt) VALUES (?1, 1, 'main', '', '', '', 4, 30, 30, 2, '')", [PROJECT]);
    let landed = Value::from((0..integrated).map(|n| format!("landed-{n}")).collect::<Vec<_>>()).to_string();
    f.write("INSERT INTO integration_batches (id, project, number, tasks, base_sha, state, created_at, ended_at)
        VALUES (1, ?1, 1, ?2, 'base', 'ended', ?3, ?3)", (PROJECT, &landed, at - 3600));
    f.write("INSERT INTO integration_rounds (batch, round, tasks, base_sha, result, ended) VALUES (1, 1, ?1, 'base', 'green', ?2)",
        (&landed, at - 3600));
    f.task("lead", Some("Bug"), "P2", "INTEGRATION", at - 600);
    f.write("UPDATE tasks SET slug = 'FIX-' || number WHERE id = 'lead'", []);
    f.write("INSERT INTO integration_entries (task, project, commit_sha, result, author, queued_at)
        VALUES ('lead', ?1, 'abc1234', 'Done.', 'human', ?2)", (PROJECT, at - 300));
    f.event("lead", at - 300, "human", "queue", "Queued abc1234 for integration into main.", None);
}

/// A level 2 signal on the queued task at confidence 0.5, its latest week's share of failures `share`.
fn signal(f: &Fixture, share: f64, at: i64) {
    let week = at.div_euclid(WEEK);
    let scores = json!({"impact": 2, "blast": 0, "reversibility": 0, "confidence": 0.5, "impact_basis": {"ranked": true,
        "shares": [{"week": week - 1, "complete": true, "unit": "failure_events", "amount": 90.0, "total": 100.0, "noise": 0.05},
                   {"week": week, "complete": false, "unit": "failure_events", "amount": share * 100.0, "total": 100.0, "noise": 0.05}]}});
    let measure = json!({"value": 5, "baseline": 2, "threshold": 4, "window_start": at - WEEK, "window_end": at});
    f.write("INSERT INTO supervisor_signals (fingerprint, project, detector, subject_kind, subject_id, metric, evidence,
        measure, scores, first_seen, last_seen, count, state) VALUES (?1, ?2, ?3, 'task', 'lead', 'failed_runs', '[]',
            ?4, ?5, ?6, ?6, 1, 'raised')",
        (KEY, PROJECT, Rule::FailedRepeat.id(), measure.to_string(), scores.to_string(), at));
}

fn compare(f: &Fixture, at: i64) -> interim::Comparison {
    f.store.with(|tx| {
        let signal = signals::get(tx, PROJECT, KEY)?;
        interim::verification_hold(tx, &signal, "case_verify", at + 900, at)
    }).unwrap()
}

fn status(f: &Fixture) -> String {
    f.store.with(|tx| Ok(ops::get(tx, "FIX-1")?.status)).unwrap()
}

/// A recorded finding as `record_finding` stores it, at `confidence`.
fn finding(confidence: f64) -> Value {
    json!({"cause": "A cause.", "confidence": confidence, "disproof": "A disproof.", "fix_proposal": "A fix.",
        "evidence": [{"table": "runs", "id": "failed-1"}], "measurements": []})
}

/// The verification round the hold belongs to, ended with `finding`.
fn round(f: &Fixture, finding: Option<Value>, at: i64) {
    f.agent("verifier");
    f.run("lead", RUN, "verifier", at);
    let case = f.store.with(|tx| Ok(json!({"id": "case_verify", "kind": "verification",
        "signal": signals::get(tx, PROJECT, KEY)?}))).unwrap();
    f.write("INSERT INTO supervisor_rounds (id, run, project, generation, fingerprint, signal_revision, case_file, deadline, finding, ended_at)
        VALUES ('case_verify', ?1, ?2, 1, ?3, 0, ?4, ?5, ?6, ?5)",
        (RUN, PROJECT, KEY, case.to_string(), at + 900, finding.map(|f| f.to_string())));
}

#[test]
fn ac233_holding_costs_less_than_waiting_takes_the_task_out_until_the_deadline() {
    let f = Fixture::new("interim-hold");
    let at = ops::now();
    project(&f, 20, at);
    signal(&f, 0.40, at);
    let found = compare(&f, at);
    assert_eq!((found.wait_share, found.hold_share), (Some(0.2), Some(0.05)), "the share times confidence; one of 20 tasks");
    assert_eq!(found.task.as_deref(), Some("FIX-1"));
    assert!(found.decision.starts_with("Holding costs less"), "{}", found.decision);
    assert_eq!(status(&f), "IN PROGRESS", "taken out of the queue");
    let (round, until): (String, i64) = f.store.with(|tx| tx.query_row(
        "SELECT round, until FROM supervisor_holds WHERE id = ?1", [found.hold.unwrap()],
        |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)).unwrap();
    assert_eq!((round.as_str(), until), ("case_verify", at + 900), "the round's deadline bounds the hold");
}

#[test]
fn ac233_waiting_costs_less_or_has_no_measure_and_nothing_is_held() {
    let f = Fixture::new("interim-wait");
    let at = ops::now();
    project(&f, 5, at);
    signal(&f, 0.30, at);
    let found = compare(&f, at);
    assert_eq!((found.wait_share, found.hold_share), (Some(0.15), Some(0.2)));
    assert!(found.hold.is_none() && found.task.is_none());
    assert!(found.decision.contains("waiting costs less"), "{}", found.decision);
    assert_eq!(status(&f), "INTEGRATION");

    f.write("UPDATE supervisor_signals SET scores = json_set(scores, '$.impact_basis', json('{\"ranked\":false,\"shares\":[],\"no_share\":\"No weekly total.\"}'))", []);
    let no_share = compare(&f, at);
    assert!(no_share.wait_share.is_none() && no_share.hold.is_none());
    assert!(no_share.decision.ends_with("No weekly total."), "{}", no_share.decision);
}

#[test]
fn ac233_an_unconfirmed_verification_ends_its_hold_even_after_a_human_comment() {
    let f = Fixture::new("interim-end");
    let at = ops::now();
    project(&f, 20, at);
    signal(&f, 0.40, at);
    compare(&f, at);
    f.event("lead", at + 10, "human", "comment", "Looking at it.", None);
    round(&f, Some(finding(0.4)), at);
    let told = f.store.with(|tx| interim::round_ended(tx, RUN, at + 20)).unwrap();
    assert_eq!(told.as_deref(), Some("FIX-1"));
    assert_eq!(status(&f), "INTEGRATION", "queued again with its commit");
    let released: (Option<String>, Option<i64>) = f.store.with(|tx| tx.query_row(
        "SELECT released_by, released_at FROM supervisor_holds", [], |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)).unwrap();
    assert_eq!(released, (Some("system".into()), Some(at + 20)));
}

#[test]
fn ac233_a_failed_end_stays_as_an_ordinary_hold_that_says_why() {
    let f = Fixture::new("interim-failed");
    let at = ops::now();
    project(&f, 20, at);
    signal(&f, 0.40, at);
    let found = compare(&f, at);
    f.event("lead", at + 10, "human", "move", "Moved to TODO.", None);
    round(&f, None, at);
    f.store.with(|tx| interim::round_ended(tx, RUN, at + 20)).unwrap();
    let (until, failed, released): (Option<i64>, Option<String>, Option<i64>) = f.store.with(|tx| tx.query_row(
        "SELECT until, end_failed, released_at FROM supervisor_holds WHERE id = ?1", [found.hold.unwrap()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(sql)).unwrap();
    assert_eq!((until, released), (None, None), "no longer interim, still in force");
    assert!(failed.unwrap().contains("changed"), "the release's own words");
}

#[test]
fn ac233_a_strong_finding_without_a_firing_recomputation_does_not_keep_the_hold() {
    let f = Fixture::new("interim-unchecked");
    let at = ops::now();
    project(&f, 20, at);
    signal(&f, 0.40, at);
    compare(&f, at);
    round(&f, Some(finding(0.9)), at);
    f.store.with(|tx| interim::round_ended(tx, RUN, at + 20)).unwrap();
    assert_eq!(status(&f), "INTEGRATION", "the model's confidence alone is no confirmation");
}

#[test]
fn ac225_an_interim_hold_says_its_deadline_on_the_incident_and_the_line() {
    let f = Fixture::new("interim-surfaces");
    let at = ops::now();
    project(&f, 20, at);
    signal(&f, 0.40, at);
    f.write("UPDATE supervisor_signals SET scores = json_set(scores, '$.blast', 3)", []);
    f.write("INSERT INTO supervisor_incidents (id, project, subject_kind, subject_id, opened_at, last_firing_at)
        VALUES (1, ?1, 'task', 'lead', ?2, ?2)", (PROJECT, at));
    f.write("INSERT INTO supervisor_incident_events (id, incident, at, kind, actor, fingerprint, detector, metric, level, value, threshold)
        VALUES (1, 1, ?1, 'signal', 'system', ?2, ?3, 'failed_runs', 3, 5, 4)", (at, KEY, Rule::FailedRepeat.id()));
    f.write("INSERT INTO supervisor_incident_signals VALUES (?1, 1, 3, x'00', 5, 4, 1, 0)", [KEY]);
    compare(&f, at);
    let incident = f.store.with(|tx| {
        let view = incidents_api::pending(tx)?.into_iter().next().expect("a level 3 hold joins an incident");
        Ok(serde_json::to_value(view).unwrap())
    }).unwrap();
    assert_eq!(incident["holds"][0]["until"], json!(at + 900));
    assert_eq!(incident["held"], "Task taken out of integration while its verification runs.");
    assert_eq!(incident["no_answer"], "Without an answer the hold ends with its verification, unless verification confirms the signal.");
    let notice = line_queue::Notice { project: PROJECT.into(), incident: 1, kind: line_queue::Kind::Updated, level: 3,
        signals: 1, channels: 1, budget: None, holds: vec![(super::holds::Kind::TakeOut, true)] };
    let text = notice.render(None);
    assert!(text.contains("Task taken out of integration while its verification runs, for up to 15 minutes."), "{text}");
    assert!(text.contains("the hold ends with its verification"), "{text}");
}

#[test]
fn ac233_a_verification_code_confirms_keeps_its_hold_as_an_ordinary_one() {
    let f = Fixture::new("interim-confirmed");
    let at = ops::now();
    f.agent("agent");
    project(&f, 20, at);
    // Failures older than a day: a human's recent move would refuse the hold (task policy).
    for (n, when) in [(1, at - 3 * super::DAY), (2, at - 2 * super::DAY)] { f.failed_run("lead", &format!("failed-{n}"), when); }
    f.write("UPDATE tasks SET status = 'INTEGRATION' WHERE id = 'lead'", []);
    signal(&f, 0.40, at);
    let found = compare(&f, at);
    assert!(found.hold.is_some(), "{}", found.decision);
    round(&f, Some(finding(0.8)), at);
    f.store.with(|tx| interim::round_ended(tx, RUN, at + 20)).unwrap();
    let (until, released): (Option<i64>, Option<i64>) = f.store.with(|tx| tx.query_row(
        "SELECT until, released_at FROM supervisor_holds WHERE id = ?1", [found.hold.unwrap()],
        |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)).unwrap();
    assert_eq!((until, released), (None, None), "two failures still cross the threshold of two");
    assert_eq!(status(&f), "IN PROGRESS", "still out of integration");
}
