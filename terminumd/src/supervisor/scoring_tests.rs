// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213: detector subject risk sets levels even without an additive impact quantity.

use super::engine_tests::{Fixture, NOW, PROJECT};
use super::{DAY, Rule, evaluate, probes, signals};

fn probing(name: &str) -> Fixture {
    let f = Fixture::new(name);
    f.write("UPDATE projects SET dir = ?1 WHERE id = ?2", (f.path.parent().unwrap().to_str().unwrap(), PROJECT));
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)", (PROJECT, NOW - 90 * DAY));
    f.task("history", None, "P2", "DONE", NOW - 90 * DAY);
    f
}

#[test]
fn ac213_migration_without_a_share_reaches_the_store_and_costly_recovery_level() {
    let f = probing("subject-migration");
    let arch = super::probe_arch::Architecture {
        since: NOW - 90 * DAY,
        migrations: vec![(NOW - DAY, "migrations/001.sql".into())],
        ..Default::default()
    };
    probes::tests::publish_sample(&f, probes::ARCH, NOW - 10, serde_json::to_string(&arch).unwrap());
    let evaluation = f.live(Rule::MigrationEdited, NOW);
    let [found] = evaluation.detectors[0].findings.as_slice() else { panic!("one edited migration") };
    assert_eq!((found.scores.impact, found.scores.blast, found.scores.reversibility), (0, 2, 2));
    assert!(found.assessment.shares.is_empty() && found.assessment.no_share.is_some());
    evaluation.record(&f.store).unwrap();
    let mut recorded = signals(&f.store, PROJECT).unwrap();
    assert_eq!(recorded.len(), 1);
    let signal = recorded.remove(0);
    assert_eq!((signal.level, signal.response), (2, "R4"));
    assert!(!signal.charter_breach);
    let replay = super::replay(&f.path, PROJECT, Some(NOW)).unwrap();
    let line = replay.lines().find(|line| line.contains("existing_migration_edits:")).unwrap();
    assert!(line.contains("impact 0, level 2, response R4"), "{line}");
}

#[test]
fn ac213_single_task_verify_without_a_share_requires_hours_of_rework() {
    let f = Fixture::new("subject-verify");
    f.integration(true);
    f.task("work", Some("Bug"), "P2", "INTEGRATION", NOW - DAY);
    f.write("INSERT INTO integration_verify_results (attempt, project, task, at, passed, agent, commit_sha)
        VALUES ('attempt', ?1, 'work', ?2, 0, NULL, 'fixture')", (PROJECT, NOW - 60));
    let evaluation = evaluate(&f.store, PROJECT, NOW, &[Fixture::detector(Rule::VerifyRed.id())]).unwrap();
    let [found] = evaluation.detectors[0].findings.as_slice() else { panic!("one failed Verify") };
    assert_eq!((found.scores.impact, found.scores.blast, found.scores.reversibility), (0, 0, 1));
    assert!(found.assessment.shares.is_empty() && found.assessment.no_share.is_some());
    evaluation.record(&f.store).unwrap();
    let signal = signals(&f.store, PROJECT).unwrap().remove(0);
    assert_eq!((signal.level, signal.response), (1, "R2"));
    assert!(!signal.charter_breach);
}

#[test]
fn ac213_nightly_red_raises_scope_but_retries_alone_do_not() {
    let f = probing("subject-gate");
    let gate = super::probe_gate::Gate { since: NOW - 90 * DAY, nightly_since: Some(NOW - 90 * DAY),
        events: vec![super::probe_gate::GateEvent { at: NOW - 60, retried: 1, nightly_red: true }] };
    probes::tests::publish_sample(&f, probes::GATE, NOW - 10, serde_json::to_string(&gate).unwrap());
    let evaluation = f.live(Rule::Flaky, NOW);
    assert_eq!(evaluation.detectors[0].findings.len(), 2);
    evaluation.record(&f.store).unwrap();
    let recorded = signals(&f.store, PROJECT).unwrap();
    for signal in &recorded {
        assert_eq!(signal.scores.impact, 0);
        assert!(signal.impact_basis.shares.is_empty() && signal.impact_basis.no_share.is_some());
        assert!(!signal.charter_breach);
        let json = serde_json::to_value(signal).unwrap();
        if signal.metric == "nightly_reds" {
            assert_eq!((signal.scores.blast, signal.level, signal.response), (2, 2, "R4"));
            assert_eq!(json["score_raises"], serde_json::json!([
                {"score":"blast", "from":1, "to":2, "rule":"nightly_reds"}]));
        } else {
            assert_eq!((signal.metric.as_str(), signal.scores.blast, signal.level), ("gate_retries", 1, 1));
            assert!(json.get("score_raises").is_none());
        }
    }
    let replay = super::replay(&f.path, PROJECT, Some(NOW)).unwrap();
    let line = replay.lines().find(|line| line.contains("nightly_reds:")).unwrap();
    assert!(line.contains("impact 0, level 2, response R4"), "{line}");

    let gate = super::probe_gate::Gate { events: vec![super::probe_gate::GateEvent {
        at: NOW + DAY - 60, retried: 1, nightly_red: false }], ..gate };
    probes::tests::publish_sample(&f, probes::GATE, NOW + DAY, serde_json::to_string(&gate).unwrap());
    f.live(Rule::Flaky, NOW + DAY + 1).record(&f.store).unwrap();
    let current = signals(&f.store, PROJECT).unwrap();
    assert!(current.iter().any(|s| s.metric == "nightly_reds" && s.state == super::signals::State::Resolved));
    let retries = current.iter().find(|s| s.metric == "gate_retries").unwrap();
    assert_eq!((retries.scores.blast, retries.level), (1, 1));
    assert!(serde_json::to_value(retries).unwrap().get("score_raises").is_none());
}

#[test]
fn ac213_agent_verify_scope_needs_the_querys_samples_and_worsening_band() {
    let f = Fixture::new("subject-agent-verify");
    f.integration(true);
    f.agent("agent");
    for (period, at) in [("old", NOW - 35 * DAY), ("now", NOW - 60)] {
        for n in 0..29 {
            let task = format!("{period}-{n}");
            f.task(&task, Some("Bug"), "P2", "INTEGRATION", at - 10);
            f.write("INSERT INTO integration_verify_results (attempt, project, task, at, passed, agent, commit_sha)
                VALUES (?1, ?2, ?1, ?3, ?4, 'agent', 'fixture')", (&task, PROJECT, at, period == "old"));
        }
    }
    let sweep = |at| f.live(Rule::VerifyRed, at);
    assert!(sweep(NOW).detectors[0].findings.iter().all(|found| found.subject_kind == "task"),
        "29 attempts in either window cannot establish an agent-wide regression");
    for (period, at) in [("old", NOW - 35 * DAY), ("now", NOW - 60)] {
        let task = format!("{period}-29");
        f.task(&task, Some("Bug"), "P2", "INTEGRATION", at - 10);
        f.write("INSERT INTO integration_verify_results (attempt, project, task, at, passed, agent, commit_sha)
            VALUES (?1, ?2, ?1, ?3, ?4, 'agent', 'fixture')", (&task, PROJECT, at, period == "old"));
    }
    sweep(NOW).record(&f.store).unwrap();
    let recorded = signals(&f.store, PROJECT).unwrap();
    let agent = recorded.iter().find(|s| s.subject_kind == "agent").expect("30 attempts in both windows establish scope");
    assert_eq!((agent.scores.impact, agent.scores.blast, agent.level), (0, 1, 1));
    assert_eq!(serde_json::to_value(agent).unwrap()["score_raises"], serde_json::json!([
        {"score":"blast", "from":0, "to":1, "rule":"agent_verify_failures"}]));
    assert!(recorded.iter().filter(|s| s.subject_kind == "task").all(|s| s.scores.blast == 0));
    f.write("UPDATE integration_verify_results SET passed = 1 WHERE at > ?1", [NOW - DAY]);
    assert!(sweep(NOW + 1).detectors[0].findings.is_empty(), "a healthy rate is never raised");
}

#[test]
fn ac213_a_wait_blocking_another_task_has_scope_even_below_the_impact_floor() {
    let f = Fixture::new("subject-blocked");
    for n in 0..40 {
        f.task(&format!("unrelated-{n}"), None, "P2", "TODO", NOW - DAY);
    }
    f.task("waiting", None, "P2", "IN PROGRESS", NOW - 100);
    f.transition("waiting", NOW - 60, "IN PROGRESS", "WAIT FOR HUMAN");
    f.task("dependent", None, "P2", "TODO", NOW - 100);
    f.write("INSERT INTO task_links (task, other, kind) VALUES ('dependent', 'waiting', 'blocked_by')", []);
    f.live(Rule::WaitHuman, NOW).record(&f.store).unwrap();
    let recorded = signals(&f.store, PROJECT).unwrap();
    let [signal] = recorded.as_slice() else { panic!("only blocked_tasks fires, the wait is below its age floor") };
    assert_eq!((signal.metric.as_str(), signal.scores.impact, signal.scores.blast, signal.level), ("blocked_tasks", 0, 1, 1));
    assert_eq!(serde_json::to_value(signal).unwrap()["score_raises"], serde_json::json!([
        {"score":"blast", "from":0, "to":1, "rule":"blocked_tasks"}]));
    f.write("DELETE FROM task_links WHERE task = 'dependent'", []);
    assert!(f.live(Rule::WaitHuman, NOW + 1).detectors[0].findings.is_empty());
}

#[test]
fn ac213_project_failure_process_scores_do_not_claim_a_single_task_scope() {
    let f = Fixture::new("subject-failure-area");
    f.agent("agent");
    f.task("failed", Some("Bug"), "P2", "IN PROGRESS", NOW - DAY);
    f.failed_run("failed", "run", NOW - 60);
    f.cost("run", NOW - 59, 1_000_000);
    f.failed_run("failed", "again", NOW - 30);
    let reasonless = f.live(Rule::FailedWithoutReason, NOW);
    assert_eq!(reasonless.detectors[0].findings[0].scores.blast, 1);
    f.live(Rule::CostOutlier, NOW).record(&f.store).unwrap();
    let recorded = signals(&f.store, PROJECT).unwrap();
    let signal = recorded.iter().find(|s| s.metric == "failed_run_dollars").expect("failed spending crosses the floor");
    assert_eq!(signal.scores.blast, 1);
    assert_eq!(serde_json::to_value(signal).unwrap()["score_raises"], serde_json::json!([
        {"score":"blast", "from":0, "to":1, "rule":"failed_spend_area"}]));
}
