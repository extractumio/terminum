// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-245: authority-sensitive branches that need stable store fixtures.

use serde_json::json;

use crate::dispatcher::runs;
use crate::tasks::{auto, ops::{self, sql}, store::HUMAN};
use super::{Rule, WEEK, engine_tests::{Fixture, PROJECT}, holds::{self, Kind, Request}, incidents, incidents_api, SCHEMA};

fn arm(f: &Fixture, rule: Rule, at: i64) {
    let report = json!({
        "attached_at": at - WEEK, "scanned_at": at, "first_observation": at - 4 * WEEK,
        "limited_history": false, "code_only": true,
        "sources": {"integration": false, "code_host": false, "pool_agents": 0, "routines": 0},
        "detectors": {(rule.id()): {"mode":"live","can_run":true,"unavailable":[],
            "threshold":null,"observed_thresholds":{},"replay_firings":0,
            "holds":{"live_at_attach":true,"replay_judged":0,"replay_right":0}}}
    });
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at, last_ok, attachment, settings_revision)
        VALUES (?1, 1, 1, ?2, ?2, ?3, 0)", (PROJECT, at, report.to_string()));
}

fn signal(f: &Fixture, rule: Rule, task: Option<&str>, level: u8, at: i64) {
    let measure = json!({"value":5,"baseline":2,"threshold":4,"window_start":at-1,"window_end":at});
    let scores = json!({"impact":level,"blast":0,"reversibility":0,"confidence":1.0});
    let evidence = json!([{"table":"runs","id":"original"}]);
    f.write("INSERT INTO supervisor_signals (fingerprint, project, detector, subject_kind, subject_id, metric, evidence,
        measure, scores, first_seen, last_seen, count, state) VALUES ('hold-signal', ?1, ?2, ?3, ?4, 'task_dollars',
            ?5, ?6, ?7, ?8, ?8, 1, 'raised')",
        (PROJECT, rule.id(), if task.is_some() { "task" } else { "project" }, task.unwrap_or(PROJECT),
            evidence.to_string(), measure.to_string(), scores.to_string(), at));
}

#[test]
fn ac245_code_can_hold_a_level_three_run_without_a_round() {
    let f = Fixture::new("holds-code");
    let at = ops::now();
    arm(&f, Rule::CostOutlier, at);
    f.task("lead", Some("Bug"), "P2", "IN PROGRESS", at - 120);
    f.write("UPDATE tasks SET slug = 'FIX-' || number WHERE id = 'lead'", []);
    f.agent("agent");
    f.run("lead", "original", "agent", at - 50);
    f.write("UPDATE runs SET state = 'working', pane_key = 'fixture-pane' WHERE id = 'original'", []);
    signal(&f, Rule::CostOutlier, Some("lead"), 3, at);
    f.store.with(|tx| {
        let stopped = holds::automatic(tx, PROJECT, "hold-signal", at)?.expect("L3 evidence names the current run");
        assert!(!stopped.shadow);
        assert_eq!(stopped.run.unwrap().state, runs::Phase::Left);
        assert!(holds::automatic(tx, PROJECT, "hold-signal", at + 1)?.is_none());
        let (actor, response, rounds): (String, String, i64) = tx.query_row(
            "SELECT actor, response, (SELECT count(*) FROM supervisor_rounds) FROM supervisor_holds WHERE id = ?1",
            [stopped.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(sql)?;
        assert_eq!((actor.as_str(), response.as_str(), rounds), ("system", "R5", 0));
        Ok(())
    }).unwrap();
}

#[test]
fn ac245_grouped_run_retains_its_open_members_and_failed_release_stays_held() {
    let f = Fixture::new("holds-group");
    let at = ops::now();
    arm(&f, Rule::FailedRepeat, at);
    for (key, status) in [("lead", "IN PROGRESS"), ("peer", "IN PROGRESS"), ("done", "DONE")] {
        f.task(key, Some("Bug"), "P2", status, at - 120);
        f.write("UPDATE tasks SET slug = 'FIX-' || number WHERE id = ?1", [key]);
    }
    f.agent("agent");
    f.run("lead", "original", "agent", at - 50);
    f.write("INSERT INTO run_tasks (run, task) VALUES ('original','peer'), ('original','done')", []);
    f.write("INSERT INTO auto_starts (task, project, at) VALUES ('lead', ?1, ?2), ('peer', ?1, ?2)", (PROJECT, at - 50));
    f.write("UPDATE runs SET state = 'working', pane_key = 'fixture-pane' WHERE id = 'original'", []);
    signal(&f, Rule::FailedRepeat, Some("lead"), 2, at);
    f.store.with(|tx| {
        let lead: String = tx.query_row("SELECT slug FROM tasks WHERE id = 'lead'", [], |r| r.get(0)).map_err(sql)?;
        let stopped = holds::apply(tx, PROJECT, Request { signal: "hold-signal".into(), kind: Kind::StopRun,
            task: Some(lead), reason: "Fixture measured risk.".into() }, "system", at)?;
        let (undo, pending) = holds::run_undo(tx, PROJECT, stopped.id)?;
        assert_eq!(undo.run, "original");
        assert_eq!(undo.tasks.len(), 2);
        assert!(pending.is_none());
        let tasks: Vec<_> = undo.tasks.iter().map(|id| ops::get(tx, id)).collect::<Result<_, _>>()?;
        assert!(tasks.iter().all(|t| t.status == "FAILED" && t.key != "done"));
        assert!(auto::tripped(tx, PROJECT, at - 100)?.is_none(), "a hold is a Give up, not an agent failure");
        assert!(holds::check_run_release(tx, PROJECT, stopped.id, "original", &tasks[..1], false).is_err());
        holds::check_run_release(tx, PROJECT, stopped.id, "original", &tasks, false)?;

        let run = stopped.run.unwrap();
        let keys = tasks.iter().map(|t| t.key.clone()).collect::<Vec<_>>();
        let new = runs::add(tx, &run, &keys, runs::Started::default(), at + 1)?;
        holds::reserve_run_release(tx, stopped.id, HUMAN, &new)?;
        assert!(holds::check_run_release(tx, PROJECT, stopped.id, &new, &tasks, false).is_err());
        assert_eq!(holds::list(tx, PROJECT)?.len(), 1, "reservation is not release");
        runs::change(tx, &new, |r| r.state = runs::Phase::Gone)?;
        holds::check_run_release(tx, PROJECT, stopped.id, &new, &tasks, false)?;
        assert_eq!(holds::list(tx, PROJECT)?.len(), 1, "failed launch preserves the hold and lets RUN try again");
        assert!(holds::run_launched(tx, &new, at + 2)?);
        assert!(holds::list(tx, PROJECT)?.is_empty());
        Ok(())
    }).unwrap();
}

#[test]
fn ac245_cleared_signal_leaves_the_project_incident_open_until_release() {
    let f = Fixture::new("holds-incident");
    let at = ops::now();
    arm(&f, Rule::FailedRepeat, at);
    signal(&f, Rule::FailedRepeat, None, 2, at);
    f.write("UPDATE projects SET auto_on = 1, auto_mode = 'order' WHERE id = ?1", [PROJECT]);
    let hold = f.store.with(|tx| {
        let applied = holds::apply(tx, PROJECT, Request { signal: "hold-signal".into(),
            kind: Kind::PauseDispatch, task: None, reason: "Fixture measured risk.".into() }, "system", at)?;
        let incident = holds::get(tx, PROJECT, applied.id)?.incident.unwrap();
        tx.execute("UPDATE supervisor_signals SET state = 'resolved' WHERE fingerprint = 'hold-signal'", []).map_err(sql)?;
        incidents::clear_resolved(tx, PROJECT, Rule::FailedRepeat, at + 1)?;
        assert_eq!(incidents_api::pending(tx)?[0].closed_at, None);
        holds::board_release(tx, PROJECT, Kind::PauseDispatch, "", HUMAN, at + 2)?;
        assert_eq!(incidents_api::pending(tx)?[0].closed_at, Some(at + 2));
        Ok(incident)
    }).unwrap();
    f.store.snapshot(SCHEMA, |tx| {
        assert_eq!(incidents_api::events(tx, hold, None)?[0].kind, "all_clear");
        Ok(())
    }).unwrap();
}
