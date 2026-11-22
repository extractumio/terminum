// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use serde_json::json;
use std::collections::BTreeSet;

use super::{DAY, WEEK, Evidence, Rule, Threshold, evaluate};
use super::engine_tests::{Fixture, NOW, OTHER, PROJECT};

fn observed(f: &Fixture, name: &str) -> Vec<&'static str> {
    let d = Fixture::detector(name);
    evaluate(&f.store, PROJECT, NOW, &[d]).unwrap().detectors[0].findings.iter().map(|found| found.metric).collect()
}

// AC-212: every store rule is quiet on a clean store. Individual fixtures exercise their positive paths below.
#[test]
fn ac212_store_detector_catalog() {
    let f = Fixture::new("quiet-catalog");
    f.agent("agent");
    f.task("clean", Some("Feature"), "P2", "IN PROGRESS", NOW - 200);
    f.run("clean", "early", "agent", NOW - 100);
    f.transition("clean", NOW - 5, "IN PROGRESS", "DONE");
    let detectors: Vec<_> = Rule::ALL.iter().map(|rule| Fixture::detector(rule.id())).collect();
    assert_eq!(detectors.len(), 30);
    let result = evaluate(&f.store, PROJECT, NOW, &detectors).unwrap();
    for detector in &result.detectors {
        assert!(detector.findings.is_empty(), "{} unexpectedly fired", detector.rule.id());
    }
}

#[test]
fn ac212_open_wait_starved_and_bug_age() {
    let f = Fixture::new("ages");
    f.agent("agent");
    f.task("short-wait", None, "P2", "IN PROGRESS", NOW - 3 * DAY);
    f.transition("short-wait", NOW - 100, "IN PROGRESS", "WAIT FOR HUMAN");
    f.task("old-wait", None, "P2", "IN PROGRESS", NOW - 3 * DAY);
    f.transition("old-wait", NOW - 2 * DAY, "IN PROGRESS", "WAIT FOR HUMAN");
    f.task("dependent", None, "P1", "TODO", NOW - 3 * DAY);
    f.write("INSERT INTO task_links (task, kind, other, place) VALUES ('dependent', 'blocked_by', 'short-wait', 0)", []);
    f.task("old-bug", Some("Bug"), "P1", "TODO", NOW - 3 * DAY);
    assert_eq!(observed(&f, "flow.wait_human_age"), ["wait_minutes", "blocked_tasks"]);
    assert_eq!(observed(&f, "flow.starved"), ["todo_minutes"]);
    assert_eq!(observed(&f, "product.user_bug_age"), ["open_bug_minutes"]);
    f.run("old-bug", "active-run", "agent", NOW - 120);
    f.write("UPDATE runs SET state = 'working' WHERE id = 'active-run'", []);
    assert!(observed(&f, "flow.starved").is_empty(), "the slot is occupied");
}

#[test]
fn ac212_distinct_failures_reasons_reruns_and_project_boundaries() {
    let f = Fixture::new("failures");
    f.task("failed", Some("Bug"), "P2", "IN PROGRESS", NOW - 200);
    f.failed_run("failed", "one", NOW - 120);
    f.transition("failed", NOW - 110, "FAILED", "IN PROGRESS");
    f.transition("failed", NOW - 100, "IN PROGRESS", "FAILED");
    assert!(observed(&f, "flow.failed_repeat").is_empty(), "two transitions in one run");
    f.failed_run("failed", "two", NOW - 80);
    f.event("failed", NOW - 79, "dispatcher", "told", "", None);
    f.event("failed", NOW - 79, "dispatcher", "why", "fixture reason", None);
    f.transition("failed", NOW - 60, "FAILED", "DONE");
    f.run("failed", "three", "agent", NOW - 40);

    f.task_in(OTHER, "moved", Some("Bug"), "P0", "IN PROGRESS", NOW - 300);
    f.failed_run("moved", "move-run", NOW - 170);
    f.write("UPDATE tasks SET project = 'default' WHERE id = 'moved'", []);
    f.event("moved", NOW - 160, "human", "project", "", None);
    assert_eq!(observed(&f, "flow.failed_repeat"), ["failed_runs"]);
    assert_eq!(observed(&f, "flow.rerun_after_done"), ["runs_after_done"]);
    let evaluation = evaluate(&f.store, PROJECT, NOW, &[Fixture::detector("flow.failed_no_reason")]).unwrap();
    let findings = &evaluation.detectors[0].findings;
    assert_eq!(findings[0].measure.value, 2.0);
    assert_eq!(findings[0].measure.threshold, 1.0);
    assert_eq!(findings[0].evidence.len(), 2);
}

#[test]
fn ac212_lead_time_and_consecutive_backlog_growth() {
    let f = Fixture::new("delivery");
    let old = NOW - 5 * WEEK;
    f.task("quick", Some("Feature"), "P2", "IN PROGRESS", old - 600);
    f.transition("quick", old, "IN PROGRESS", "DONE");
    f.task("slow", Some("Feature"), "P2", "IN PROGRESS", NOW - 3 * WEEK);
    f.transition("slow", NOW - DAY, "IN PROGRESS", "DONE");
    for (i, at) in [NOW - DAY, NOW - 8 * DAY, NOW - 15 * DAY].into_iter().enumerate() {
        for n in 0..2 {
            f.task(&format!("waiting-{i}-{n}"), None, "P2", "TODO", at);
        }
    }
    assert_eq!(observed(&f, "flow.lead_time_shift"), ["median_lead_minutes", "p90_lead_minutes"]);
    assert_eq!(observed(&f, "flow.backlog_growth"), ["growing_weeks"]);
    let mut d = Fixture::detector("flow.lead_time_shift");
    d.threshold = Threshold::Change { by: 1_000_000.0 };
    assert!(evaluate(&f.store, PROJECT, NOW, &[d]).unwrap().detectors[0].findings.is_empty());
}

#[test]
fn ac212_schedule_health_counts_skips_failures_and_failing_tasks() {
    let f = Fixture::new("routine");
    f.task("failed-a", None, "P2", "IN PROGRESS", NOW - 100);
    f.task("failed-b", None, "P2", "IN PROGRESS", NOW - 100);
    f.write("INSERT INTO schedules (id, number, project, title, body, severity, agent_id, once_at, budget_seconds, last_due, author, created_at, updated_at)
        VALUES ('routine', 1, 'default', '', '', 'P2', 'agent', ?1, 600, ?1, 'human', ?1, ?1)", [NOW - 100]);
    for (task, state) in [(Some("failed-a"), "started"), (Some("failed-b"), "started"), (None, "skipped"), (None, "failed")] {
        f.write("INSERT INTO schedule_fires (schedule, due_at, at, task, state) VALUES ('routine', ?1, ?1, ?2, ?3)", (NOW - 80, task, state));
        if let Some(task) = task {
            f.transition(task, NOW - 40, "IN PROGRESS", "FAILED");
        }
    }
    assert_eq!(observed(&f, "flow.schedule_health"), ["failed_routine_tasks", "skipped_or_failed_fires"]);
}

#[test]
fn ac212_spend_measures_and_missing_prices() {
    let f = Fixture::new("cost");
    let old = NOW - 5 * WEEK;
    f.task("cheap", Some("Feature"), "P2", "IN PROGRESS", old - 1);
    f.run("cheap", "cheap-run", "agent", old);
    f.cost("cheap-run", old + 1, 10_000);
    f.task("expensive", Some("Feature"), "P2", "IN PROGRESS", NOW - 110);
    f.failed_run("expensive", "expensive-run", NOW - 90);
    f.cost("expensive-run", NOW - 89, 3_000_000);
    f.task("untyped", None, "P2", "IN PROGRESS", NOW - 101);
    f.run("untyped", "untyped-run", "agent", NOW - 91);
    f.cost("untyped-run", NOW - 89, 3_000_000);
    assert_eq!(observed(&f, "flow.cost_outlier"), ["failed_run_dollars", "task_dollars"]);
    let mut d = Fixture::detector("product.allocation");
    d.allocation.insert("Bug".into(), 0.8);
    d.allocation.insert("Feature".into(), 0.2);
    let seen = evaluate(&f.store, PROJECT, NOW, std::slice::from_ref(&d)).unwrap();
    let metrics: BTreeSet<_> = seen.detectors[0].findings.iter().map(|found| found.metric).collect();
    assert_eq!(metrics, BTreeSet::from(["allocation_share", "untyped_share"]));
    f.write("UPDATE usage SET model = 'unknown' WHERE run = 'expensive-run'", []);
    let unavailable = evaluate(&f.store, PROJECT, NOW, &[d]).unwrap();
    assert!(unavailable.detectors[0].findings.is_empty());
    assert!(!unavailable.detectors[0].unavailable.is_empty());
}

#[test]
fn ac212_agent_bands_use_actual_questions_peer_rates_and_sample_floors() {
    let f = Fixture::new("drift");
    for agent in ["peer", "outlier"] {
        f.agent(agent);
        for n in 0..30 {
            let task = format!("{agent}-{n}");
            let run = format!("{task}-run");
            f.task(&task, Some("Feature"), "P2", "IN PROGRESS", NOW - 200);
            f.run(&task, &run, agent, NOW - 120);
            f.cost(&run, NOW - 118, if agent == "peer" { 1000 } else { 100_000 });
            if agent == "outlier" {
                f.event(&task, NOW - 100, "agent:pool:outlier", "comment", "", Some(json!({"questions": true})));
                f.transition(&task, NOW - 98, "WAIT FOR HUMAN", "FAILED");
            }
        }
    }
    assert_eq!(observed(&f, "drift.ask_rate"), ["questions_per_task"]);
    assert_eq!(observed(&f, "drift.agent_scorecard"), ["dollars_per_run", "failed_run_share", "questions_per_task"]);
    f.write("DELETE FROM inputs WHERE run = 'outlier-29-run'", []);
    assert!(observed(&f, "drift.ask_rate").is_empty());
}

fn work_period(f: &Fixture, phase: &str, at: i64, feature: bool) {
    f.task(phase, feature.then_some("Feature"), "P2", "IN PROGRESS", at - 10);
    f.transition(phase, at + 1, "IN PROGRESS", "DONE");
}

fn send(f: &Fixture, at: i64, pane: &str) {
    f.write("INSERT INTO messages (at, sender, via, pane, host, cwd, project, text)
        VALUES (?1, 'human', 'page', ?2, 'local', '', '', '')", (at * 1000, pane));
}

#[test]
fn ac212_owner_load_involvement_and_store_guard() {
    let f = Fixture::new("owner-load");
    let old = NOW - 5 * WEEK;
    work_period(&f, "feature", old, true);
    work_period(&f, "ordinary", NOW - 3600, false);
    send(&f, old + 10, "%a");
    send(&f, old + 20, "%b");
    send(&f, old + 30, "%c");
    send(&f, NOW - 600, "%a");
    assert_eq!(observed(&f, "improve.involvement"), ["features_per_owner_hour"]);
    let guards = observed(&f, "improve.guard_broken");
    assert!(guards.contains(&"lower_concurrency_with_fewer_features"));
    send(&f, NOW - 590, "%b");
    send(&f, NOW - 580, "%c");
    send(&f, NOW - 570, "%d");
    assert!(observed(&f, "owner.load").contains(&"concurrent_sessions"));
}

#[test]
fn ac212_owner_questions_operations_and_priority_churn() {
    let f = Fixture::new("owner-operations");
    f.write("UPDATE users SET name = 'Owner' WHERE id = 'human'", []);
    f.agent("agent");
    let old = NOW - 5 * WEEK;
    work_period(&f, "old", old, true);
    work_period(&f, "recent", NOW - DAY, true);
    for (task, at, wait) in [("old", old - 600, 60), ("recent", NOW - 2 * DAY, 7_200)] {
        f.event(task, at, "agent:pool:agent", "comment", "", Some(json!({"questions": true})));
        f.event(task, at + wait, "human", "comment", "fixture answer", None);
    }
    for _ in 0..2 {
        f.event("recent", NOW - 51, "human", "edit", "Owner changed its text", None);
    }
    for severity in ["P1", "P0", "P2", "P0", "P2"] {
        f.event("recent", NOW - 50, "human", "edit", &format!("Owner set severity {severity}"), None);
    }
    assert_eq!(observed(&f, "owner.answer_latency"), ["median_answer_minutes"]);
    assert_eq!(observed(&f, "improve.operations"), ["operations_per_done"]);
    assert_eq!(observed(&f, "owner.priority_churn"), ["priority_reversals"]);
    let seen = evaluate(&f.store, PROJECT, NOW, &[Fixture::detector("owner.answer_latency")]).unwrap();
    let findings = &seen.detectors[0].findings;
    assert_eq!(findings[0].measure.baseline, 1.0);
    assert_eq!(findings[0].measure.value, 120.0);
    assert!(findings[0].evidence.iter().all(|r| matches!(r, Evidence::Events(_))));
}

// AC-212: a measured partial rule can later clear its signal and still report its other input missing.
#[test]
fn ac212_measured_metric_clears_while_other_input_is_missing() {
    let f = Fixture::new("partially-measured");
    f.agent("agent");
    for n in 0..3 {
        let task = format!("pending-{n}");
        f.task(&task, None, "P2", "WAIT FOR HUMAN", NOW - 100);
        f.event(&task, NOW - 90, "agent:pool:agent", "comment", "", Some(json!({"questions": true})));
    }
    let d = Fixture::detector("owner.answer_latency");
    let checked = evaluate(&f.store, PROJECT, NOW, std::slice::from_ref(&d)).unwrap();
    assert!(!checked.detectors[0].unavailable.is_empty());
    checked.record(&f.store).unwrap();
    assert_eq!(super::signals(&f.store, PROJECT).unwrap()[0].metric, "pending_answers");
    f.event("pending-0", NOW + 1, "human", "comment", "fixture answer", None);
    evaluate(&f.store, PROJECT, NOW + 2, &[d]).unwrap().record(&f.store).unwrap();
    assert_eq!(super::signals(&f.store, PROJECT).unwrap()[0].state, super::signals::State::Resolved);
}

// AC-212: a transfer does not turn another task's share of a joint run into exclusive spend.
#[test]
fn ac212_shared_run_remains_shared_after_a_task_moves() {
    let f = Fixture::new("shared-spend");
    let historical = NOW - 5 * WEEK;
    f.task("small", Some("Feature"), "P2", "IN PROGRESS", historical - 10);
    f.run("small", "small-run", "agent", historical);
    f.cost("small-run", historical + 1, 1000);
    f.task("stays", Some("Feature"), "P2", "IN PROGRESS", NOW - 100);
    f.task("moves", Some("Bug"), "P2", "IN PROGRESS", NOW - 100);
    f.run("stays", "shared-run", "agent", NOW - 90);
    f.write("INSERT INTO run_tasks VALUES ('shared-run', 'moves', 0)", []);
    f.cost("shared-run", NOW - 80, 3_000_000);
    f.write("UPDATE tasks SET project = 'other' WHERE id = 'moves'", []);
    f.event("moves", NOW - 20, "human", "project", "", None);
    assert!(observed(&f, "flow.cost_outlier").is_empty());
    let allocation = evaluate(&f.store, PROJECT, NOW, &[Fixture::detector("product.allocation")]).unwrap();
    let found = &allocation.detectors[0].findings;
    assert_eq!(found[0].metric, "untyped_share");
    assert_eq!(found[0].measure.value, 1.0);
    assert!(found[0].evidence.iter().all(|r| !matches!(r, Evidence::Tasks(id) if id == "moves")));
}
