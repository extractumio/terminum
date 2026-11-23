// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213: each detector sizes its share in its own additive quantity, or says why it has none.

use serde_json::json;

use super::engine_tests::{Fixture, NOW, PROJECT};
use super::impact::{Assessment, Unit};
use super::{DAY, WEEK, Finding, evaluate};

fn findings(f: &Fixture, name: &str) -> Vec<Finding> {
    let detector = Fixture::detector(name);
    evaluate(&f.store, PROJECT, NOW, &[detector]).unwrap().detectors.remove(0).findings
}

fn assessed(f: &Fixture, name: &str, metric: &str) -> (Assessment, u8) {
    let found = findings(f, name).into_iter().find(|found| found.metric == metric)
        .unwrap_or_else(|| panic!("{name} did not fire {metric}"));
    (found.assessment, found.scores.impact)
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
    let found = evaluate(&f.store, PROJECT, NOW, &[detector]).unwrap().detectors.remove(0).findings;
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
    f.write("INSERT INTO project_integration
        (project,is_on,target,check_line,publish_line,after_line,batch_most,check_minutes,agent_budget_minutes,red_rounds_most,prompt)
        VALUES (?1,1,'main','','','',2,15,15,1,'')", [PROJECT]);
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
