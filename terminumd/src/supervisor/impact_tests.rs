// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213: each detector sizes its share in its own additive quantity on a seeded store, or says why it has none.

use serde_json::json;

use super::engine_tests::{Fixture, NOW, PROJECT};
use super::impact::{Assessment, Unit};
use super::{DAY, Detector, WEEK, Finding, evaluate};

fn findings_of(f: &Fixture, detector: Detector) -> Vec<Finding> {
    evaluate(&f.store, PROJECT, NOW, &[detector]).unwrap().detectors.remove(0).findings
}

fn findings(f: &Fixture, name: &str) -> Vec<Finding> {
    findings_of(f, Fixture::detector(name))
}

fn assessed(f: &Fixture, name: &str, metric: &str) -> (Assessment, u8) {
    let found = findings(f, name).into_iter().find(|found| found.metric == metric)
        .unwrap_or_else(|| panic!("{name} did not fire {metric}"));
    (found.assessment, found.scores.impact)
}

/// Each firing metric of a detector with the units of its shares.
fn units(f: &Fixture, name: &str) -> Vec<(&'static str, Vec<Unit>)> {
    findings(f, name).into_iter().map(|found| {
        assert!(found.assessment.shares.iter().all(|s| s.amount > 0.0 && s.amount <= s.total), "{name}");
        let mut units: Vec<_> = found.assessment.shares.iter().map(|s| s.unit).collect();
        units.dedup();
        (found.metric, units)
    }).collect()
}

fn amount(assessment: &Assessment, unit: Unit) -> f64 {
    assessment.shares.iter().filter(|s| s.unit == unit).map(|s| s.amount).sum()
}

#[test]
fn ac213_starved_task_and_old_bug_share_open_priority_task_days() {
    let f = Fixture::new("impact-priority-days");
    f.agent("agent");
    for n in 0..17 {
        f.task(&format!("open-{n}"), None, "P1", "IN PROGRESS", NOW - 2 * WEEK);
    }
    f.task("low", None, "P2", "IN PROGRESS", NOW - 2 * WEEK);
    f.task("closed", None, "P0", "IN PROGRESS", NOW - 2 * WEEK);
    f.transition("closed", NOW - 2 * WEEK + DAY, "IN PROGRESS", "DONE");
    f.task("starved", Some("Bug"), "P1", "TODO", NOW - 6 * DAY);
    for (rule, metric) in [("flow.starved", "todo_minutes"), ("product.user_bug_age", "open_bug_minutes")] {
        let (assessment, impact) = assessed(&f, rule, metric);
        // Only the five days past the one-day limit count: the share never back-fills earlier weeks.
        let [share] = assessment.shares.as_slice() else { panic!("{rule}: {:?}", assessment.shares) };
        assert_eq!((share.unit, share.week), (Unit::PriorityTaskDays, NOW));
        assert!((share.amount - 5.0).abs() < 0.001, "{rule}: {}", share.amount);
        // 17 P1 tasks all week and the starved one for six days; P2 and closed tasks stay out.
        assert!((share.total - 125.0).abs() < 0.001, "{rule}: {}", share.total);
        assert_eq!(impact, 1, "{rule}");
        assert!(assessment.no_share.is_none());
    }
}

#[test]
fn ac213_priority_churn_sizes_strategic_acts_and_operations() {
    let f = Fixture::new("impact-churn");
    f.write("UPDATE users SET name = 'Owner' WHERE id = 'human'", []);
    f.agent("agent");
    f.task("recent", Some("Feature"), "P2", "IN PROGRESS", NOW - 2 * DAY);
    for severity in ["P1", "P0", "P2", "P0", "P2"] {
        f.event("recent", NOW - 50, "human", "edit", &format!("Owner set severity {severity}"), None);
    }
    for n in 0..3 {
        f.run("recent", &format!("owner-{n}"), "agent", NOW - 40 + n);
    }
    let (reversals, _) = assessed(&f, "owner.priority_churn", "priority_reversals");
    assert_eq!(amount(&reversals, Unit::StrategicActs), 3.0);
    assert!(reversals.shares.iter().all(|s| s.amount <= s.total));
    let (starts, _) = assessed(&f, "owner.priority_churn", "owner_starts");
    assert_eq!(amount(&starts, Unit::Operations), 3.0);
}

#[test]
fn ac213_amount_beyond_its_week_total_is_said() {
    let f = Fixture::new("impact-dropped");
    f.task("given-up", None, "P2", "FAILED", NOW - DAY);
    for n in 0..3 {
        f.event("given-up", NOW - 30 + n, "dispatcher", "why", crate::dispatcher::runs::GAVE_UP, None);
    }
    // Three Dispatcher stops, but no launch this week: the run-event total cannot hold them.
    let (stops, impact) = assessed(&f, "owner.priority_churn", "owner_stops");
    assert!(stops.shares.is_empty());
    assert_eq!(impact, 0);
    assert!(stops.no_share.unwrap().contains("exceeded its week's project total"));
}

#[test]
fn ac213_agent_bands_size_the_excess_over_peers() {
    let f = Fixture::new("impact-bands");
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
    let (failed, _) = assessed(&f, "drift.agent_scorecard", "failed_run_share");
    // Every outlier run failed and no peer run did: 30 excess failures of the week's 30.
    assert_eq!(amount(&failed, Unit::FailureEvents), 30.0);
    let (dollars, _) = assessed(&f, "drift.agent_scorecard", "dollars_per_run");
    let [share] = dollars.shares.as_slice() else { panic!("{:?}", dollars.shares) };
    assert_eq!(share.unit, Unit::Spend);
    // Only the cost above the peers' per-run cost is charged: 99 of the week's 101 parts.
    assert!((share.fraction() - 99.0 / 101.0).abs() < 1e-6, "{}", share.fraction());
    let (questions, impact) = assessed(&f, "drift.ask_rate", "questions_per_task");
    assert!(questions.shares.is_empty());
    assert_eq!(impact, 0);
    assert!(questions.no_share.unwrap().contains("answers of several kinds"));
}

#[test]
fn ac213_allocation_sizes_spend_above_its_target() {
    let f = Fixture::new("impact-allocation");
    f.agent("agent");
    for (task, kind, output) in [("bug", "Bug", 1000), ("feature", "Feature", 9000)] {
        f.task(task, Some(kind), "P2", "IN PROGRESS", NOW - 200);
        f.run(task, &format!("{task}-run"), "agent", NOW - 120);
        f.cost(&format!("{task}-run"), NOW - 118, output);
    }
    let mut detector = Fixture::detector("product.allocation");
    detector.allocation.insert("Bug".into(), 0.8);
    detector.allocation.insert("Feature".into(), 0.2);
    let found = findings_of(&f, detector);
    let feature = found.iter().find(|f| f.subject_id == "Feature").unwrap();
    let [share] = feature.assessment.shares.as_slice() else { panic!("{:?}", feature.assessment.shares) };
    assert_eq!(share.unit, Unit::Spend);
    assert!((share.fraction() - 0.7).abs() < 1e-6, "{}", share.fraction());
    // Under its target, a type spent nothing extra: counting it too would count each dollar twice.
    let bug = found.iter().find(|f| f.subject_id == "Bug").unwrap();
    assert!(bug.assessment.shares.is_empty());
    assert!(bug.assessment.no_share.as_deref().unwrap().contains("above zero"));
}

#[test]
fn ac213_returned_tasks_are_run_events() {
    let f = Fixture::new("impact-returned");
    f.integration(true);
    f.agent("agent");
    for n in 0..4 {
        let task = format!("returned-{n}");
        f.task(&task, None, "P2", "IN PROGRESS", NOW - DAY);
        f.run(&task, &format!("{task}-run"), "agent", NOW - 600);
        if n < 2 {
            f.event(&task, NOW - 300, "dispatcher", "comment", "", Some(json!({"left": "red"})));
        }
    }
    let (returned, _) = assessed(&f, "flow.returned", "tasks_returned");
    let [share] = returned.shares.as_slice() else { panic!("{:?}", returned.shares) };
    assert_eq!((share.unit, share.amount, share.total), (Unit::RunEvents, 2.0, 4.0));
}

#[test]
fn ac213_metric_without_a_quantity_says_why() {
    let f = Fixture::new("impact-unsized");
    f.agent("agent");
    let old = NOW - 5 * WEEK;
    for (task, at, wait) in [("old", old, 60), ("recent", NOW - 2 * DAY, 7_200)] {
        f.task(task, Some("Feature"), "P2", "IN PROGRESS", at - 10);
        f.event(task, at, "agent:pool:agent", "comment", "", Some(json!({"questions": true})));
        f.event(task, at + wait, "human", "comment", "fixture answer", None);
    }
    let (median, impact) = assessed(&f, "owner.answer_latency", "median_answer_minutes");
    assert!(median.shares.is_empty());
    assert_eq!(impact, 0);
    assert!(median.no_share.unwrap().contains("A median has no additive count"));
}

#[test]
fn ac213_failures_reruns_and_cost_outliers_share_events_and_spend() {
    let f = Fixture::new("impact-failures");
    f.agent("agent");
    f.task("failed", Some("Bug"), "P2", "IN PROGRESS", NOW - 200);
    f.failed_run("failed", "one", NOW - 120);
    f.failed_run("failed", "two", NOW - 80);
    f.transition("failed", NOW - 60, "FAILED", "DONE");
    f.run("failed", "three", "agent", NOW - 40);
    f.cost("three", NOW - 39, 3_000_000);
    assert_eq!(units(&f, "flow.failed_no_reason"), [("reasonless_failures", vec![Unit::FailureEvents])]);
    assert_eq!(units(&f, "flow.rerun_after_done"), [("runs_after_done", vec![Unit::RunEvents, Unit::Tokens, Unit::Spend])]);

    let old = NOW - 5 * WEEK;
    f.task("cheap", Some("Feature"), "P2", "IN PROGRESS", old - 1);
    f.run("cheap", "cheap-run", "agent", old);
    f.cost("cheap-run", old + 1, 10_000);
    f.task("expensive", Some("Feature"), "P2", "IN PROGRESS", NOW - 100);
    f.run("expensive", "expensive-run", "agent", NOW - 90);
    f.cost("expensive-run", NOW - 89, 3_000_000);
    assert!(units(&f, "flow.cost_outlier").contains(&("task_dollars", vec![Unit::Tokens, Unit::Spend])));
}

#[test]
fn ac213_lead_time_and_backlog_share_task_days_and_tasks() {
    let f = Fixture::new("impact-delivery");
    let old = NOW - 5 * WEEK;
    f.task("quick", Some("Feature"), "P2", "IN PROGRESS", old - 600);
    f.transition("quick", old, "IN PROGRESS", "DONE");
    // Each weekly sample compares with the week before it, which needs its own DONE task.
    f.task("prior", Some("Feature"), "P2", "IN PROGRESS", NOW - 8 * DAY - 600);
    f.transition("prior", NOW - 8 * DAY, "IN PROGRESS", "DONE");
    f.task("slow", Some("Feature"), "P2", "IN PROGRESS", NOW - 3 * WEEK);
    f.transition("slow", NOW - DAY, "IN PROGRESS", "DONE");
    for (i, at) in [NOW - DAY, NOW - 8 * DAY, NOW - 15 * DAY].into_iter().enumerate() {
        for n in 0..2 {
            f.task(&format!("waiting-{i}-{n}"), None, "P2", "TODO", at);
        }
    }
    assert_eq!(units(&f, "flow.lead_time_shift"), [("p90_lead_minutes", vec![Unit::TaskDaysToDone])]);
    assert_eq!(units(&f, "flow.backlog_growth"), [("growing_weeks", vec![Unit::Tasks])]);
}

#[test]
fn ac213_schedule_health_shares_schedule_and_failure_events() {
    let f = Fixture::new("impact-routine");
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
    assert_eq!(units(&f, "flow.schedule_health"),
        [("failed_routine_tasks", vec![Unit::FailureEvents]), ("skipped_or_failed_fires", vec![Unit::ScheduleEvents])]);
}

#[test]
fn ac213_wait_shares_task_days_of_wait() {
    let f = Fixture::new("impact-wait");
    f.task("old-wait", None, "P2", "IN PROGRESS", NOW - 3 * DAY);
    f.transition("old-wait", NOW - 2 * DAY, "IN PROGRESS", "WAIT FOR HUMAN");
    assert_eq!(units(&f, "flow.wait_human_age"), [("wait_minutes", vec![Unit::TaskDaysOfWait])]);
}

#[test]
fn ac213_a_level_reaches_a_human_as_its_word() {
    assert_eq!([0, 1, 2, 3].map(super::impact::level_word), ["Record", "Digest", "Card", "Incident"]);
}
