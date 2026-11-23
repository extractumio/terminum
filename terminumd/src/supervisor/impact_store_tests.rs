// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213: the store detectors that size shares, each on a seeded store.

use super::engine_tests::{Fixture, NOW, PROJECT};
use super::impact::Unit;
use super::{DAY, WEEK, evaluate};

/// Each firing metric of a detector with the units of its shares.
fn units(f: &Fixture, name: &str) -> Vec<(&'static str, Vec<Unit>)> {
    let detector = Fixture::detector(name);
    evaluate(&f.store, PROJECT, NOW, &[detector]).unwrap().detectors.remove(0).findings.into_iter().map(|found| {
        assert!(found.assessment.shares.iter().all(|s| s.amount > 0.0 && s.amount <= s.total), "{name}");
        let mut units: Vec<_> = found.assessment.shares.iter().map(|s| s.unit).collect();
        units.dedup();
        (found.metric, units)
    }).collect()
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
