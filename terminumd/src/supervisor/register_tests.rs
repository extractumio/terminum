// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use rusqlite::Transaction;
use serde_json::json;

use crate::tasks::{auto, ops::{self, NewTask, rows, sql}, store::HUMAN};

use super::{DAY, WEEK, engine_tests::{Fixture, NOW, OTHER, PROJECT}, fix_tasks, governor, register, settings, task_policy};

fn enabled(f: &Fixture, budgets: serde_json::Value) {
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)", (PROJECT, NOW));
    f.store.with(|tx| {
        let settings = serde_json::from_value(json!({"budgets":budgets})).unwrap();
        settings::write(tx, PROJECT, 0, settings, false).map(drop)
    }).unwrap();
}

fn signal(f: &Fixture, key: &str, subject: &str, impact: u8, share: f64, charter: bool) {
    let scores = json!({"impact":impact,"blast":0,"reversibility":0,"confidence":1.0,
        "charter_breach":charter,"impact_basis":{"ranked":true,"shares":[{"week":1,
        "complete":true,"unit":"failure_events","amount":share*100.0,"total":100.0,"noise":0.0}]}});
    let measure = json!({"value":4,"threshold":2,"baseline":2,"window_start":NOW-WEEK,"window_end":NOW});
    f.write("INSERT INTO supervisor_signals (fingerprint, project, detector, subject_kind, subject_id, metric,
        evidence, measure, scores, first_seen, last_seen, count, state)
        VALUES (?1, ?2, 'flow.failed_repeat', 'task', ?3, 'failed_runs', ?4, ?5, ?6, ?7, ?7, 1, 'new')",
        (key, PROJECT, subject, r#"[{"table":"events","id":"event-a"}]"#, measure.to_string(), scores.to_string(), NOW));
}

fn draft(title: &str, keys: &[&str]) -> register::Draft {
    serde_json::from_value(json!({"title": title, "lens":"flow", "hypothesis":"A repeat causes discarded work.",
        "disproof":"The work still fails with one attempt.", "confidence":0.8, "effort":1, "signals":keys,
        "fix_options":[{"layer":"check", "action":"Add a check for the repeated failure."}]})).unwrap()
}

fn count(tx: &Transaction, table: &str) -> Result<i64, String> {
    tx.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).map_err(sql)
}

#[test]
fn ac236_register_rank_ruling_and_recurrence() {
    let f = Fixture::new("register");
    enabled(&f, json!({}));
    signal(&f, "high", "A", 2, 0.22, false);
    signal(&f, "another", "B", 1, 0.06, false);
    signal(&f, "lower", "C", 1, 0.08, false);
    f.task("landed-fix", Some("Bug"), "P1", "DONE", NOW);
    f.store.with(|tx| {
        assert!(register::open(tx, PROJECT, draft("No signal", &[]), NOW).is_err());
        assert!(register::open(tx, OTHER, draft("Wrong project", &["high"]), NOW).is_err());
        let low = register::open(tx, PROJECT, draft("Lower impact", &["lower"]), NOW)?;
        let high = register::open(tx, PROJECT, draft("Clustered cause", &["high","another"]), NOW)?;
        let ranked = register::ranked(tx, PROJECT, NOW, false)?;
        assert_eq!(ranked.iter().map(|p| p.id).collect::<Vec<_>>(), [high, low]);
        assert_eq!(ranked[0].yardstick.len(), 2);
        assert_eq!(ranked[0].impact, 2);
        assert!((ranked[0].rank - 0.176).abs() < 1e-9);

        let update = |weeks| serde_json::from_value(json!({"id":high, "observation_weeks": weeks})).unwrap();
        assert!(register::update(tx, PROJECT, update(2), NOW).is_err());
        register::update(tx, PROJECT, update(5), NOW)?;
        register::rule(tx, PROJECT, high, "Accept only the evidence already seen.", NOW)?;
        tx.execute("UPDATE supervisor_signals SET count = count + 1 WHERE fingerprint = 'high'", []).map_err(sql)?;
        register::advance(tx, PROJECT, NOW + 1)?;
        assert_eq!(register::get(tx, PROJECT, high, NOW + 1)?.state, "accepted");
        tx.execute("UPDATE supervisor_signals SET evidence = ?1 WHERE fingerprint = 'high'",
            [r#"[{"table":"events","id":"new-event"},{"table":"events","id":"event-a"}]"#]).map_err(sql)?;
        register::advance(tx, PROJECT, NOW + 2)?;
        assert_eq!(register::get(tx, PROJECT, high, NOW + 2)?.state, "recurred");
        let set_clear = |at| tx.execute("UPDATE supervisor_problem_signals SET clear_at = ?2, clear_seen = ?3 WHERE problem = ?1",
            (high, NOW, at)).map_err(sql);
        set_clear(NOW + 4 * WEEK)?;
        register::advance(tx, PROJECT, NOW + 4 * WEEK)?;
        assert_eq!(register::get(tx, PROJECT, high, NOW + 4 * WEEK)?.state, "watching");
        register::advance(tx, PROJECT, NOW + 5 * WEEK)?;
        assert!(register::get(tx, PROJECT, high, NOW + 5 * WEEK)?.active(), "an old sample cannot resolve a problem");
        set_clear(NOW + 5 * WEEK)?;
        register::advance(tx, PROJECT, NOW + 5 * WEEK)?;
        assert_eq!(register::get(tx, PROJECT, high, NOW + 5 * WEEK)?.state, "resolved");

        // AC-248: a finished fix recurs once and carries the new signal.
        tx.execute("INSERT INTO supervisor_problem_tasks VALUES (?1,'landed-fix')", [high]).map_err(sql)?;
        register::advance(tx, PROJECT, NOW + 5 * WEEK + 1)?;
        register::advance(tx, PROJECT, NOW + 5 * WEEK + 2)?;
        let requests = super::reflection::shown(tx, PROJECT)?;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["triggers"], json!(["fix_recurred"]));
        Ok(())
    }).unwrap();
}

#[test]
fn ac237_judgment_reserve_and_caps() {
    let f = Fixture::new("judgment");
    enabled(&f, json!({"judgment_ceiling":100, "daily_token_ceiling":95, "round_token_ceiling":80}));
    f.task("baseline", Some("Feature"), "P2", "DONE", NOW - DAY);
    f.run("baseline","round-run","agent",NOW);
    f.store.with(|tx| {
        assert_eq!(governor::admit_round(tx, PROJECT, NOW, false)?, 70);
        tx.execute("INSERT INTO supervisor_rounds (id,run,project,generation,fingerprint,signal_revision,case_file,
            deadline,token_cap) VALUES ('case','round-run',?1,1,'signal',1,'{}',?2,70)",
            (PROJECT, NOW + 900)).map_err(sql)?;
        assert_eq!(governor::admit_round(tx, PROJECT, NOW, false).unwrap_err(), governor::JUDGMENT_NOTICE);
        assert_eq!(governor::admit_round(tx, PROJECT, NOW, true)?, 25);
        Ok(())
    }).unwrap();
    f.cost("round-run", NOW + 2, 71);
    f.store.with(|tx| {
        governor::judge_usage(tx, "round-run", NOW + 2)?;
        assert_eq!(count(tx, "supervisor_signals")?, 2);
        assert!(tx.query_row("SELECT failure IS NOT NULL FROM supervisor_rounds WHERE id = 'case'", [], |r| r.get::<_, bool>(0)).map_err(sql)?);
        assert_eq!(governor::spending(tx, PROJECT, NOW + 2)?.judgment.tokens, 71);
        assert_eq!(governor::admit_round(tx, PROJECT, NOW + 2, true)?, 24);
        Ok(())
    }).unwrap();
}

#[test]
fn ac238_fix_budget_and_critical_exception() {
    let f = Fixture::new("fixes");
    enabled(&f, json!({"fixes_ceiling":1.0}));
    f.task("baseline", Some("Feature"), "P2", "DONE", NOW - DAY);
    f.run("baseline","baseline-run","agent",NOW - DAY);
    f.cost("baseline-run", NOW - DAY, 100_000);
    for (key, impact, share) in [("low",1,0.9),("bigger",2,0.35),("smaller",2,0.1),("middle",2,0.2)] {
        signal(&f, key, key, impact, share, false);
        f.store.with(|tx| register::open(tx,PROJECT,draft(key,&[key]),NOW)).unwrap();
    }
    let (fix, card) = f.store.with(|tx| {
        let changed = fix_tasks::advance(tx, PROJECT, NOW)?;
        assert_eq!(changed.len(), 2);
        let fix = ops::get(tx, &changed[0])?;
        assert_eq!((fix.name.as_str(),fix.severity.as_str(),fix.owner.as_str()), ("bigger","P1","supervisor:default"));
        assert!(fix.supervisor_marked);
        let card = ops::get(tx, &changed[1])?;
        assert_eq!(card.status, "WAIT FOR HUMAN");
        let body = ops::text_of(tx, &card.id)?;
        let (middle, smaller) = (body.find("middle").unwrap(), body.find("smaller").unwrap());
        assert!(middle < smaller && !body.contains("bigger") && !body.contains("low"));
        assert!(fix_tasks::advance(tx, PROJECT, NOW + 1)?.is_empty());
        assert_eq!(count(tx, "supervisor_budget_cards")?, 1);
        let spend = governor::spending(tx, PROJECT, NOW+1)?.fixes;
        assert_eq!((spend.dollars,spend.held_dollars), (0.0,1.0));
        assert_eq!(fix_tasks::auto_reservation(tx, std::slice::from_ref(&fix), NOW+1)?, Some(1.0));
        let (mut lower, rev) = { let saved = settings::read(tx, PROJECT)?; (saved.settings, saved.revision) };
        lower.budgets.fixes_ceiling = Some(0.0);
        settings::write(tx, PROJECT, rev, lower, false)?;
        assert_eq!(fix_tasks::auto_reservation(tx, std::slice::from_ref(&fix), NOW+1).unwrap_err(), governor::FIX_NOTICE);
        let (mut restored, rev) = { let saved = settings::read(tx, PROJECT)?; (saved.settings, saved.revision) };
        restored.budgets.fixes_ceiling = Some(1.0);
        settings::write(tx, PROJECT, rev, restored, false)?;
        Ok((fix,card))
    }).unwrap();
    signal(&f, "charter", "D", 3, 0.02, true);
    let exception = f.store.with(|tx| {
        let changed = fix_tasks::advance(tx, PROJECT, NOW + 2)?;
        assert_eq!(changed.len(), 1);
        let task = ops::get(tx, &changed[0])?;
        assert_eq!(task.severity, "P0");
        assert!(ops::text_of(tx,&task.id)?.contains("is exempt"));
        Ok(task)
    }).unwrap();
    f.run(&fix.key, "shared-run", "agent", NOW + 3);
    f.write("INSERT INTO run_tasks (run, task) VALUES ('shared-run', ?1)", [&exception.key]);
    f.write("INSERT INTO supervisor_fix_runs VALUES ('shared-run', 1)", []);
    f.write("UPDATE runs SET started_by = 'supervisor:default' WHERE id = 'shared-run'", []);
    f.cost("shared-run", NOW + 3, 150_000);
    f.run("baseline", "starter-run", "agent", NOW + 3);
    f.write("UPDATE runs SET started_by = 'supervisor:default' WHERE id = 'starter-run'", []);
    f.cost("starter-run", NOW + 3, 50_000);
    f.store.with(|tx| {
        let spend = governor::spending(tx, PROJECT, NOW+4)?.fixes;
        assert_eq!((spend.dollars,spend.exception_dollars), (2.0,1.5));
        assert_eq!(count(tx, "supervisor_budget_cards")?, 1);
        assert_eq!(ops::get(tx, &card.id)?.status, "WAIT FOR HUMAN");
        Ok(())
    }).unwrap();
}

#[test]
fn ac236_ac238_a_round_proposal_needs_a_checked_finding() {
    let f = Fixture::new("checked-fix");
    enabled(&f, json!({"fixes_ceiling": 1.0}));
    f.task("baseline", Some("Feature"), "P2", "DONE", NOW - DAY);
    f.run("baseline", "round-run", "agent", NOW);
    signal(&f, "high", "A", 2, 0.3, false);
    f.store.with(|tx| {
        tx.execute("INSERT INTO supervisor_rounds (id, run, project, generation, fingerprint, signal_revision,
            case_file, deadline) VALUES ('case', 'round-run', ?1, 1, 'high', 1, '{}', ?2)",
            (PROJECT, NOW + 90)).map_err(sql)?;
        let id = register::open(tx, PROJECT, draft("Proposed fix", &["high"]), NOW)?;
        register::require_finding(tx, id, Some("round-run"))?;
        assert!(!register::get(tx, PROJECT, id, NOW)?.action_ready);
        assert!(fix_tasks::advance(tx, PROJECT, NOW)?.is_empty());
        tx.execute("UPDATE supervisor_rounds SET finding = '{}', found_at = ?1 WHERE run = 'round-run'",
            [NOW + 1]).map_err(sql)?;
        assert_eq!(fix_tasks::advance(tx, PROJECT, NOW + 2)?.len(), 1);
        Ok(())
    }).unwrap();
}

#[test]
fn ac239_marked_task_policy_and_override() {
    let f = Fixture::new("override");
    enabled(&f, json!({}));
    f.agent("agent");
    let t = f.store.with(|tx| {
        let t = ops::create(tx, HUMAN, NewTask { title:"Bug: Task with evidence".into(),
            severity:Some("P1".into()), ..Default::default() }, NOW - DAY)?;
        tx.execute("UPDATE tasks SET agent = 'agent' WHERE id = ?1", [&t.key]).map_err(sql)?;
        Ok(t)
    }).unwrap();
    signal(&f, "evidence", &t.key, 2, 0.2, false);
    f.store.with(|tx| {
        let p = register::open(tx, PROJECT, draft("Scoped problem", &["evidence"]), NOW)?;
        task_policy::propose(tx,PROJECT,p,&t,None,NOW)?;
        assert!(task_policy::rank(tx,PROJECT,&t.key,NOW)?.is_some());
        tx.execute("INSERT INTO supervisor_task_marks (task, project, estimate) VALUES (?1, ?2, 0)", (&t.key, PROJECT)).map_err(sql)?;
        let setting = auto::setting(tx,PROJECT)?.with(&json!({"wait_minutes":0}))?;
        assert_eq!(auto::candidates(tx,PROJECT,&setting,NOW,None)?.len(),1);
        let off = setting.with(&json!({"supervisor_tasks":false}))?;
        assert!(auto::candidates(tx,PROJECT,&off,NOW,None)?.is_empty());
        ops::move_to(tx,HUMAN,&t.id,"IN PROGRESS",NOW+1)?;
        assert!(task_policy::record_override(tx,&t,NOW+2)?.is_some());
        assert!(task_policy::check(tx,&t,NOW+2).is_err());
        assert!(task_policy::rank(tx,PROJECT,&t.key,NOW+2)?.is_none());
        task_policy::record_override(tx,&t,NOW+3)?;
        let counts = rows(tx,"SELECT count FROM supervisor_signals WHERE detector = 'self.fights'",[],|r| r.get::<_,i64>(0))?;
        assert_eq!(counts,[1]);
        task_policy::check(tx,&t,NOW+DAY+1)?;
        assert!(task_policy::rank(tx,PROJECT,&t.key,NOW+DAY+1)?.is_none(), "an old proposal stays superseded");
        task_policy::propose(tx,PROJECT,p,&t,None,NOW+DAY+1)?;
        assert!(task_policy::rank(tx,PROJECT,&t.key,NOW+DAY+1)?.is_some());
        assert_eq!(ops::get(tx,&t.id)?.severity,"P1");
        Ok(())
    }).unwrap();
}
