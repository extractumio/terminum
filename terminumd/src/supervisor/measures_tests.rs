// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use rusqlite::Transaction;
use serde_json::json;

use crate::tasks::{ops::{self, sql}, store::{DISPATCHER, HUMAN}};
use super::{WEEK, engine_tests::{Fixture, NOW, PROJECT}, governor, measures, misses};

fn enabled(f: &Fixture) {
    f.write("INSERT INTO project_supervisor (project,enabled,generation,enabled_at) VALUES (?1,1,1,?2)", (PROJECT, NOW - 10 * WEEK));
}

fn signal(f: &Fixture, key: &str, subject: (&str, &str), state: &str, evidence: &str, at: i64) {
    f.write("INSERT INTO supervisor_signals (fingerprint,project,detector,subject_kind,subject_id,metric,
        evidence,measure,scores,first_seen,last_seen,count,state)
        VALUES (?1,?2,'flow.failed_repeat',?3,?4,'fixture',?5,?6,?7,?8,?8,1,?9)",
        (key, PROJECT, subject.0, subject.1, evidence,
        r#"{"value":3,"baseline":1,"threshold":2,"window_start":1,"window_end":2}"#,
        r#"{"impact":2,"blast":0,"reversibility":0,"confidence":1}"#, at, state));
}

fn owner_bug(tx: &Transaction, by: &str, priority: &str, parent: Option<String>) -> Result<ops::Task, String> {
    ops::create(tx, by, ops::NewTask { title: "Bug: a fixture problem".into(), text: "It failed.".into(),
        project: Some(PROJECT.into()), severity: Some(priority.into()), parent, ..Default::default() }, NOW)
}

#[test]
fn ac255_miss_from_owner_and_target_events() {
    let f = Fixture::new("misses");
    enabled(&f);
    let parent = |title| f.store.with(|tx| ops::create(tx, HUMAN, ops::NewTask {
        title, project: Some(PROJECT.into()), ..Default::default()
    }, NOW - 100)).unwrap();
    let raised = parent("A reported failure".into());
    let shadow = parent("A shadow failure".into());
    signal(&f, "raised", ("task", &raised.key), "raised", "[]", NOW - 20);
    signal(&f, "shadow", ("task", &shadow.key), "shadow", "[]", NOW - 30);
    signal(&f, "at-the-same-time", ("target", "main"), "raised", "[]", NOW);
    signal(&f, "muted", ("target", "next"), "muted", "[]", NOW - 40);
    f.store.with(|tx| {
        let unsignaled = owner_bug(tx, HUMAN, "P1", None)?;
        let raised_bug = owner_bug(tx, HUMAN, "P0", Some(raised.id))?;
        let shadow_bug = owner_bug(tx, HUMAN, "P1", Some(shadow.id))?;
        owner_bug(tx, DISPATCHER, "P0", None)?;
        owner_bug(tx, HUMAN, "P2", None)?;
        misses::target_red(tx, PROJECT, "main", "sha1", "task", NOW)?;
        misses::target_red(tx, PROJECT, "main", "sha1", "task", NOW + 1)?;
        misses::target_red(tx, PROJECT, "next", "sha2", "task", NOW)?;
        misses::watchdog(tx, PROJECT, NOW, NOW - 10)?;
        let read = |source: &str, subject: &str| tx.query_row("SELECT first_evidence_at, signal_state, raised_at
            FROM supervisor_miss_checks WHERE project = ?1 AND source = ?2 AND subject_id = ?3",
            (PROJECT, source, subject), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<i64>>(2)?))).map_err(sql);
        assert_eq!(read("owner_bug", &unsignaled.key)?, (NOW, None, None));
        assert_eq!(read("owner_bug", &raised_bug.key)?, (NOW-20, Some("raised".into()), Some(NOW-20)));
        assert_eq!(read("owner_bug", &shadow_bug.key)?, (NOW-30, Some("shadow".into()), None));
        assert_eq!(read("target_red", "main")?, (NOW, None, None));
        assert_eq!(read("target_red", "next")?, (NOW-40, Some("muted".into()), None));
        assert_eq!(read("watchdog", PROJECT)?, (NOW-10, None, None));
        let card = measures::read(tx, PROJECT, NOW+1, &governor::spending(tx, PROJECT, NOW+1)?)?;
        assert_eq!(card["misses"]["sample"], 6);
        assert_eq!(card["misses"]["count"], 5);
        assert_eq!(super::reflection::shown(tx, PROJECT)?.len(), 5);
        Ok(())
    }).unwrap();
}

#[test]
fn ac256_supervisor_scorecard_denominators() {
    let f = Fixture::new("scorecard");
    enabled(&f);
    let as_of = NOW + 5 * WEEK;
    let spending = |tx: &Transaction| governor::spending(tx, PROJECT, as_of);
    f.store.with(|tx| {
        let empty = measures::read(tx, PROJECT, as_of, &spending(tx)?)?;
        assert!(empty["resolved"]["rate"]["value"].is_null());
        assert!(empty["owner_load"]["verdict_minutes"].is_null());
        Ok(())
    }).unwrap();

    f.task("fix", None, "P2", "INTEGRATION", NOW);
    f.task("fix-watching", None, "P2", "INTEGRATION", as_of - 10);
    f.task("review", None, "P2", "DONE", as_of - 20);
    for (name, born) in [("fix", NOW), ("fix-watching", as_of - 10)] {
        f.event(name, born + 1, DISPATCHER, "comment", "Published.",
            Some(json!({"left":"landed","notice":true})));
    }
    let proof = f.event("fix", NOW, HUMAN, "comment", "Earlier evidence.", None);
    signal(&f, "fixed", ("task", "fix"), "resolved", &json!([{"table":"events","id":proof}]).to_string(), NOW + 60);
    signal(&f, "watching", ("task", "fix-watching"), "new", "[]", as_of - 9);
    f.agent("worker");
    f.run("fix", "fix-run", "worker", NOW);
    f.cost("fix-run", as_of - 10, 42);
    f.run("review", "judgment-run", "worker", as_of - 40);
    f.cost("judgment-run", as_of - 30, 66);
    for (id, state, task, key) in [(1, "resolved", "fix", "fixed"), (2, "watching", "fix-watching", "watching")] {
        f.write("INSERT INTO supervisor_problems
            (id,project,title,lens,hypothesis,disproof,options,confidence,effort,state,opened_at)
            VALUES (?1,?2,'Fixture','flow','','','[]',1,1,?3,?4)", (id, PROJECT, state, NOW));
        f.write("INSERT INTO supervisor_problem_tasks VALUES (?1,?2)", (id, task));
        f.write("INSERT INTO supervisor_problem_signals (problem,fingerprint,threshold,weeks,clear_at,clear_seen)
            VALUES (?1,?2,2,4,?3,?4)", (id, key, if id == 1 { NOW + 30 } else { as_of - 9 }, as_of));
    }
    f.write("INSERT INTO supervisor_problem_events (problem,at,state,detail) VALUES (1,?1,'resolved','Measured.')", [as_of - 3]);
    let review = f.event("review", as_of - 20, DISPATCHER, "comment", "Fixture review.", None);
    f.write("INSERT INTO supervisor_review_reports VALUES ('review',?1,?2,?3)", (PROJECT, as_of - 20, review));
    f.write("INSERT INTO supervisor_review_items VALUES ('review','fixed',0)", []);
    f.write("INSERT INTO supervisor_verdicts (task,fingerprint,at,verdict,active_seconds)
        VALUES ('review','fixed',?1,'useful',30)", [as_of - 2]);
    f.write("INSERT INTO supervisor_task_marks (task,project,estimate) VALUES ('fix',?1,0)", [PROJECT]);
    f.write("INSERT INTO supervisor_review_runs (run,project,cap,generation) VALUES ('judgment-run',?1,100,1)", [PROJECT]);
    f.write("INSERT INTO supervisor_rounds (id,run,project,generation,fingerprint,signal_revision,case_file,deadline,
        finding,refused,ended_at) VALUES ('round','judgment-run',?1,1,'fixed',1,?2,?3,?4,2,?3)",
        (PROJECT, r#"{"kind":"investigation"}"#, as_of - 10, r#"{"confidence":0.8}"#));
    // The investigation represents this run in spend, so it must be counted once.
    f.write("DELETE FROM supervisor_review_runs WHERE run = 'judgment-run'", []);
    f.write("INSERT INTO supervisor_actions (id,project,detector,response,kind,source,fingerprint,target,at)
        VALUES (1,?1,'flow.failed_repeat','R2','cause','judgment-run','fixed','fixed',?2),
               (2,?1,'flow.failed_repeat','R2','fix','made-fix','fixed','fix',?2),
               (3,?1,'flow.failed_repeat','R2','card','question','fixed','review',?2)", (PROJECT, as_of - 20));
    f.write("INSERT INTO supervisor_outcomes VALUES (1,1,?1,'right','fixture'),(2,0,?1,'wrong','fixture')", [as_of - 2]);
    f.event("fix", as_of - 10, HUMAN, "archive", "Archived by the owner.", None);
    f.store.with(|tx| {
        misses::target_red(tx, PROJECT, "main", "fixture", "task", as_of)?;
        measures::page_dispatched(tx, PROJECT, as_of)?;
        measures::observe_resolutions(tx, PROJECT, as_of)?;
        let m = measures::read(tx, PROJECT, as_of, &spending(tx)?)?;
        assert_eq!(m["decisions"]["right"], 1);
        assert_eq!(m["decisions"]["rate"]["sample"], 2);
        assert_eq!(m["decisions"]["pending"], 1);
        assert_eq!(m["resolved"]["rate"]["value"], 0.5);
        assert_eq!(m["resolved"]["watching"], 1);
        assert_eq!(m["resolved"]["rate"]["can_gate"], false);
        assert!(m["resolved"]["rate"]["interval_95"].is_array());
        assert_eq!(m["misses"]["count"], 1);
        assert_eq!(m["delays"][0]["seconds"], 60.0);
        assert_eq!(m["overrides"]["count"], 1);
        assert_eq!(m["overrides"]["rate"]["sample"], 3);
        assert_eq!(m["refusals"], json!({"count":2,"sample":3}));
        assert_eq!(m["usefulness"]["rate"]["value"], 1.0);
        assert_eq!(m["calibration"][0]["sample"], 1);
        assert!(m["calibration"][0]["rate"].is_null());
        assert_eq!(m["judgment"]["tokens"], 66);
        assert_eq!(m["judgment"]["sample"], 1);
        assert!(m["fixes"]["dollars"].as_f64().unwrap() > 0.);
        assert_eq!(m["fixes"]["sample"], 1);
        assert_eq!(m["weekly_resolved"], 1);
        assert_eq!(m["owner_load"], json!({"cards":1,"pages":1,"presses":1,"timed":1,"verdict_minutes":0.5}));
        let future = as_of + 3601;
        assert_eq!(measures::read(tx, PROJECT, future, &spending(tx)?)?["resolved"]["count"], 1);
        tx.execute("INSERT INTO supervisor_problem_events (problem,at,state,detail) VALUES (1,?1,'recurred','New evidence.')", [future]).map_err(sql)?;
        let recurred = measures::read(tx, PROJECT, future, &spending(tx)?)?;
        assert_eq!(recurred["resolved"]["count"], 0);
        assert_eq!(recurred["resolved"]["recurred"], 1);
        Ok(())
    }).unwrap();
}

#[test]
fn ac258_protected_state_denials() {
    use crate::roles::Role;
    use crate::supervisor::{Level, config_changes, register};

    let names = ["set_supervisor_level", "set_supervisor_budgets", "set_scorecard", "set_yardstick",
        "record_owner_verdict", "edit_ledger", "set_charter", "set_guardrails", "set_channels",
        "set_probes", "revert_config_change"];
    let protected = [
        ("level", json!("operator")), ("budgets", json!({"judgment_ceiling":1})),
        ("scorecard", json!({"window":1})), ("yardstick", json!({"weeks":1})),
        ("verdicts", json!([])), ("ledger", json!([])), ("never", json!([])), ("priorities", json!([])),
        ("guardrails", json!({"threshold_percent":99})), ("channels", json!([])), ("probes", json!({})),
    ];
    for level in [Level::Observer, Level::Advisor, Level::Operator] {
        let role = Role::Supervisor(Some(level));
        for tool in names { assert!(!role.allows(tool), "{level:?} {tool}"); }
        for (name, value) in &protected {
            let mut proposal = json!({"detector":"flow.failed_repeat","action":{"operation":"tune","window_days":7}});
            proposal["action"][name] = value.clone();
            assert!(serde_json::from_value::<config_changes::Proposal>(proposal).is_err(), "{name}");
            let mut update = json!({"id":42});
            update[name] = value.clone();
            assert!(serde_json::from_value::<register::Update>(update).is_err(), "{name}");
        }
    }
    assert!(serde_json::from_value::<register::Update>(json!({"id":42,"observation_weeks":52})).is_err());
}
