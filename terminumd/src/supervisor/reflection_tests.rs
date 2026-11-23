// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-248 and AC-249 exercise the retained trigger producers and actual scored runs.

use serde_json::{Value, json};

use crate::tasks::{ops::sql, store::DISPATCHER};
use super::{DAY, WEEK, engine_tests::{Fixture, NOW, PROJECT}, governor, outcomes, reflection, reflection_timeline, rounds, signals};

fn enabled(f: &Fixture) {
    f.write("INSERT INTO project_supervisor (project,enabled,generation,enabled_at)
        VALUES (?1,1,1,?2)", (PROJECT, NOW - DAY));
}

#[test]
fn ac248_wrong_alert_and_breaker_share_a_request_and_misses_are_durable() {
    let f = Fixture::new("reflection-triggers");
    enabled(&f);
    f.write("INSERT INTO supervisor_actions (project,detector,response,kind,source,fingerprint,target,at)
        VALUES (?1,'flow.failed_repeat','R4','alert','fixture','','100',?2)", (PROJECT, NOW - 1));
    f.store.with(|tx| {
        let alert = outcomes::find(tx, PROJECT, outcomes::Kind::Alert, "fixture")?.unwrap();
        let wrong = (outcomes::Verdict::Wrong, "Alert marked wrong.");
        assert!(outcomes::judge(tx, PROJECT, &alert, 0, NOW, wrong)?);
        assert!(!outcomes::judge(tx, PROJECT, &alert, 0, NOW, wrong)?);
        let action = reflection::shown(tx, PROJECT)?;
        assert_eq!(action.len(), 1);
        assert_eq!(action[0]["triggers"], json!(["breaker_opened", "alert_wrong"]));
        let timeline = reflection_timeline::build(tx, PROJECT, "action", &alert.id.to_string(), None, NOW)?;
        for kind in ["action", "outcome", "breaker"] {
            assert!(timeline.iter().any(|event| event["kind"] == kind), "{timeline:?}");
        }
        for _ in 0..2 {
            signals::diagnostic(tx, PROJECT, "self.miss", ("target","example","earlier_evidence"),
                &[], ((NOW - 20) as f64, NOW as f64), NOW)?;
        }
        assert_eq!(reflection::waiting(tx, NOW + 2, None)?.len(), 2);
        assert!(reflection::shown(tx, PROJECT)?.iter().any(|r| r["triggers"] == json!(["miss"])));
        tx.execute("UPDATE project_supervisor SET generation = 2 WHERE project = ?1", [PROJECT]).map_err(sql)?;
        assert!(reflection::waiting(tx, NOW + 2, None)?.is_empty());
        Ok(())
    }).unwrap();
}

#[test]
fn ac248_decision_band_needs_two_sufficient_windows_and_crosses_once() {
    let f = Fixture::new("reflection-bands");
    enabled(&f);
    f.store.with(|tx| {
        let add = |offset: i64, count: i64, at: i64, verdict: &str| -> Result<(), String> {
            tx.execute("INSERT INTO supervisor_actions (project,detector,response,kind,source,fingerprint,target,at)
                SELECT ?1,'flow.failed_repeat','R4','nudge',json_each.value + ?2,'','example',?3
                FROM json_each(?4)", (PROJECT, offset, at, json!((0..count).collect::<Vec<_>>()).to_string())).map_err(sql)?;
            tx.execute("INSERT INTO supervisor_outcomes SELECT id,1,?2,?3,'Measured'
                FROM supervisor_actions WHERE project = ?1 AND at = ?4 AND CAST(source AS INTEGER) >= ?5",
                (PROJECT, at + DAY, verdict, at, offset)).map_err(sql)?;
            Ok(())
        };
        add(1, 30, NOW - 5 * WEEK, "right")?;
        add(100, 29, NOW - WEEK, "wrong")?;
        outcomes::check_bands(tx, PROJECT, 1, NOW)?;
        assert!(rounds::waiting(tx, NOW, None)?.is_empty());
        add(129, 1, NOW - WEEK, "wrong")?;
        outcomes::check_bands(tx, PROJECT, 1, NOW)?;
        outcomes::check_bands(tx, PROJECT, 1, NOW + 1)?;
        let cards = reflection::shown(tx, PROJECT)?;
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["triggers"], json!(["decision_rate"]));
        Ok(())
    }).unwrap();
}

#[test]
fn ac248_budget_records_an_actual_excursion_and_deduplicates_while_over() {
    let f = Fixture::new("reflection-budget");
    enabled(&f);
    f.store.with(|tx| {
        let mut spend = governor::Spending::default();
        spend.judgment.limit_tokens = 500;
        spend.judgment.held_tokens = 500;
        reflection::budget(tx, PROJECT, &spend, NOW)?;
        assert!(reflection::waiting(tx, NOW + 2, None)?.is_empty());
        spend.judgment.tokens = 501;
        for at in [NOW + 1, NOW + 2] { reflection::budget(tx, PROJECT, &spend, at)?; }
        let requests = reflection::shown(tx, PROJECT)?;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["triggers"], json!(["budget_exceeded"]));
        Ok(())
    }).unwrap();
}

#[test]
fn ac249_agent_rates_and_sample_floor() {
    let f = Fixture::new("agent-scorecard");
    f.agent("reviewed");
    f.agent("other");
    for i in 0..30 {
        let (task, run) = (format!("debt-{i}"), format!("run-{i}"));
        f.task(&task, Some("Debt"), "P2", "IN PROGRESS", NOW - 100);
        f.run(&task, &run, "reviewed", NOW - 90);
        f.cost(&run, NOW - 20, i + 1);
        if i == 0 {
            f.transition(&task, NOW - 40, "IN PROGRESS", "FAILED");
            for how in ["red", "conflict"] {
                f.event(&task, NOW - 45, DISPATCHER, "comment", "Fixture exit.", Some(json!({"left":how})));
            }
            for at in [NOW - 85, NOW - 80] {
                f.event(&task, at, "agent:pool:reviewed", "comment", "Fixture question.", Some(json!({"questions":true})));
            }
        }
    }
    f.task("other-bug", Some("Bug"), "P2", "IN PROGRESS", NOW - 100);
    f.run("other-bug", "other-run", "other", NOW - 90);
    f.cost("other-run", NOW - 20, 17);
    let read = || f.store.with(|tx| super::spend::review_rates(tx, PROJECT, super::review_data::window(NOW))).unwrap();
    fn row(rates: &[Value], agent: &str, kind: &str) -> Value {
        rates.iter().find(|row| row["agent"] == agent && row["task_type"] == kind).unwrap().clone()
    }
    let rates = read();
    assert_eq!(rates.len(), 8);
    assert!(row(&rates, "reviewed", "Debt")["return"]["value"].is_null());
    f.write("INSERT INTO project_integration
        (project,is_on,target,check_line,publish_line,after_line,batch_most,check_minutes,agent_budget_minutes,red_rounds_most,prompt)
        VALUES (?1,1,'main','','','',2,15,15,1,'')", [PROJECT]);
    let rates = read();
    let r = row(&rates, "reviewed", "Debt");
    assert_eq!(r["return"]["value"], json!(1. / 30.));
    assert_eq!(r["failure"]["value"], json!(1. / 30.));
    assert_eq!(r["questions"]["value"], json!(2. / 30.));
    for metric in ["failure", "return", "questions", "cost"] {
        assert_eq!(r[metric]["sample"], 30, "{r}");
        assert_eq!(r[metric]["can_gate"], true, "{r}");
        assert!(r[metric]["interval_95"].is_array(), "{r}");
    }
    let small = row(&rates, "other", "Bug");
    for metric in ["failure", "return", "questions", "cost"] {
        assert_eq!(small[metric]["sample"], 1, "{small}");
        assert_eq!(small[metric]["can_gate"], false, "{small}");
        assert!(small[metric]["interval_95"].is_array(), "{small}");
    }
    let detector = Fixture::detector("drift.agent_scorecard");
    let observed = super::evaluate(&f.store, PROJECT, NOW, &[detector]).unwrap();
    assert!(observed.detectors.iter().flat_map(|check| &check.findings).all(|row| row.subject_id != "other/Bug"),
        "a one-case agent does not trigger drift");
}
