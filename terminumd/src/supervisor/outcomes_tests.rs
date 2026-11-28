// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-246–247 and AC-263 exercise retained outcomes and their effect on the shared line.

use serde_json::json;
use crate::integrate::store::{Left, give_back};
use crate::tasks::{ops::sql, store::{DISPATCHER, HUMAN}};
use super::{Kind, Verdict, check, find, record};
use crate::supervisor::{DAY, WEEK, Rule, breakers, engine_tests::{Fixture, NOW, OTHER, PROJECT},
    incidents, line, line_queue, outcome_evidence, settings, signals};

fn signal(f: &Fixture, fingerprint: &str, rule: Rule, level: u8) {
    let measure = json!({"value":5,"baseline":2,"threshold":4,"window_start":NOW-1,"window_end":NOW});
    let scores = json!({"impact":level,"blast":0,"reversibility":0,"confidence":1.0});
    f.write("INSERT INTO supervisor_signals (fingerprint, project, detector, subject_kind, subject_id, metric,
        evidence, measure, scores, first_seen, last_seen, count, state)
        VALUES (?1,?2,?3,'project','target','fixture','[]',?4,?5,?6,?6,1,'raised')",
        (fingerprint, PROJECT, rule.id(), measure.to_string(), scores.to_string(), NOW));
}

fn problem(f: &Fixture, key: &str, state: &str, run: Option<&str>) -> i64 {
    f.store.with(|tx| {
        let id: i64 = tx.query_row("INSERT INTO supervisor_problems (project,title,lens,hypothesis,disproof,options,
            confidence,effort,state,opened_at,source_round) VALUES (?1,'Fixture','flow','','','[]',1,1,?2,?3,?4)
            RETURNING id", (PROJECT, state, NOW, run), |r| r.get(0)).map_err(sql)?;
        tx.execute("INSERT INTO supervisor_problem_signals (problem,fingerprint,threshold,weeks) VALUES (?1,?2,4,4)",
            (id, key)).map_err(sql)?;
        Ok(id)
    }).unwrap()
}

#[test]
fn ac246_later_outcome_and_immediate_override() {
    let f = Fixture::new("outcomes-clock");
    signal(&f, "nudge", Rule::FailedRepeat, 2);
    f.store.with(|tx| {
        let s = signals::get(tx, PROJECT, "nudge")?;
        record(tx, &s, Kind::Nudge, "note", "task", "", NOW)?;
        check(tx, PROJECT, NOW + DAY - 1)?;
        let a = find(tx, PROJECT, Kind::Nudge, "note")?.unwrap();
        let readings = || crate::tasks::ops::rows(tx, "SELECT days, verdict FROM supervisor_outcomes
            WHERE action = ?1 ORDER BY days", [a.id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)));
        assert!(readings()?.is_empty());
        check(tx, PROJECT, NOW + DAY)?;
        assert_eq!(readings()?, [(1, "undecided".into())]);
        assert_eq!(tx.query_row("SELECT state FROM supervisor_signals WHERE detector = 'self.unjudged'
            AND project = ?1", [PROJECT], |r| r.get::<_, String>(0)).map_err(sql)?, "raised");
        tx.execute("UPDATE supervisor_signals SET last_seen = ?1 WHERE fingerprint = 'nudge'", [NOW + 2 * DAY]).map_err(sql)?;
        check(tx, PROJECT, NOW + WEEK)?;
        tx.execute("UPDATE supervisor_signals SET state = 'resolved' WHERE fingerprint = 'nudge'", []).map_err(sql)?;
        tx.execute("INSERT INTO supervisor_evaluations VALUES (?1,?2,?3)",
            (PROJECT, Rule::FailedRepeat.id(), NOW + 3 * WEEK)).map_err(sql)?;
        check(tx, PROJECT, NOW + 4 * WEEK)?;
        check(tx, PROJECT, NOW + 5 * WEEK)?;
        assert_eq!(readings()?, [(1, "undecided".into()), (7, "wrong".into()), (28, "right".into())]);
        assert_eq!(tx.query_row("SELECT state FROM supervisor_signals WHERE detector = 'self.unjudged'
            AND project = ?1", [PROJECT], |r| r.get::<_, String>(0)).map_err(sql)?, "resolved");
        assert!(breakers::open_id(tx, PROJECT, Rule::FailedRepeat.id(), "R4")?.is_none());
        Ok(())
    }).unwrap();
}

#[test]
fn ac246_each_decision_kind_uses_its_own_evidence() {
    use Verdict::{Right, Undecided, Wrong};
    let f = Fixture::new("outcomes-evidence");
    for (key, level) in [("source", 2), ("alert", 3), ("active", 2), ("cleared", 2)] {
        signal(&f, key, Rule::FailedRepeat, level);
    }
    f.task("fix", Some("Bug"), "P1", "IN PROGRESS", NOW - DAY);
    f.task("red", Some("Bug"), "P1", "TODO", NOW + 10);
    f.task("run", Some("Bug"), "P1", "IN PROGRESS", NOW - DAY);
    f.task("card", None, "P1", "WAIT FOR HUMAN", NOW);
    f.agent("agent");
    f.run("run", "assessed", "agent", NOW + 1);
    f.run("fix", "round-run", "agent", NOW - 1);
    f.write("INSERT INTO supervisor_rounds (id,run,project,generation,fingerprint,signal_revision,case_file,deadline)
        VALUES ('case','round-run',?1,1,'source',1,'{}',?2)", (PROJECT, NOW + DAY));
    let p = problem(&f, "source", "watching", Some("round-run"));
    f.write("INSERT INTO supervisor_problem_tasks VALUES (?1,'fix')", [p]);
    let question = f.event("card", NOW, HUMAN, "comment", "", Some(json!({"questions":true,
        "question":{"recommended":"try-fix","options":[{"id":"try-fix","label":"Try fix"}]}})));
    let incident: i64 = f.store.with(|tx| {
        tx.query_row("INSERT INTO supervisor_incidents (project,subject_kind,subject_id,opened_at,last_firing_at)
            VALUES (?1,'project','target',?2,?2) RETURNING id", (PROJECT, NOW), |r| r.get(0)).map_err(sql)
    }).unwrap();
    let action = f.store.with(|tx| {
        let s = signals::get(tx, PROJECT, "source")?;
        let alert = signals::get(tx, PROJECT, "alert")?;
        let hold: i64 = tx.query_row("INSERT INTO supervisor_holds
            (project,fingerprint,detector,response,level,kind,target,reason,actor,applied_at,shadow)
            VALUES (?1,'source',?2,'R4',2,'pause_integration','','Fixture','system',?3,0) RETURNING id",
            (PROJECT, Rule::FailedRepeat.id(), NOW), |r| r.get(0)).map_err(sql)?;
        for (kind, source, target, baseline) in [
            (Kind::Hold, hold.to_string(), "".into(), ""),
            (Kind::Fix, "fix".into(), "fix".into(), "round-run"),
            (Kind::Run, "run".into(), "run".into(), "assessed"),
            (Kind::Alert, "alert".into(), incident.to_string(), ""),
            (Kind::Cause, "round-run".into(), "source".into(), ""),
            (Kind::Card, question.clone(), "card".into(), "problem"),
            (Kind::Nudge, "nudge".into(), "run".into(), ""),
        ] {
            record(tx, if matches!(kind, Kind::Alert) { &alert } else { &s }, kind, &source, &target, baseline, NOW)?;
        }
        Ok(hold.to_string())
    }).unwrap();
    let inspect = |expected| f.store.snapshot(crate::supervisor::SCHEMA, |tx| {
        for ((kind, source), verdict) in [(Kind::Hold, action.as_str()), (Kind::Fix, "fix"), (Kind::Run, "run"),
            (Kind::Alert, "alert"), (Kind::Cause, "round-run"), (Kind::Card, question.as_str()), (Kind::Nudge, "nudge")]
            .into_iter().zip(expected) {
            let a = find(tx, PROJECT, kind, source)?.unwrap();
            assert_eq!(outcome_evidence::check(tx, PROJECT, &a, NOW + DAY)?.0, verdict, "{source}");
        }
        Ok(())
    }).unwrap();

    inspect([Undecided; 7]);
    f.transition("fix", NOW + 30, "IN PROGRESS", "DONE");
    f.transition("run", NOW + 30, "IN PROGRESS", "DONE");
    f.write("UPDATE supervisor_problems SET state = 'resolved' WHERE id = ?1", [p]);
    f.write("INSERT INTO supervisor_problem_events (problem,at,state,detail) VALUES (?1,?2,'resolved','Protected yardstick held.')",
        (p, NOW + 55));
    f.event("red", NOW + 20, "dispatcher", "comment", "The target failed its own check.", Some(json!({"target_red":true})));
    f.write("INSERT INTO supervisor_evaluations VALUES (?1,?2,?3)", (PROJECT, Rule::FailedRepeat.id(), NOW + 40));
    f.write("UPDATE supervisor_signals SET state = 'resolved' WHERE fingerprint = 'source'", []);
    f.event("card", NOW + 50, HUMAN, "comment", "Chosen.", Some(json!({"choice":{"question":question,"option":"try-fix"}})));
    f.store.with(|tx| incidents::act(tx, incident, incidents::Action::Acknowledge, NOW + 50)).unwrap();
    inspect([Right; 7]);

    f.write("UPDATE supervisor_problems SET state = 'recurred' WHERE id = ?1", [p]);
    inspect([Right, Undecided, Right, Right, Undecided, Undecided, Right]);
    f.write("UPDATE supervisor_signals SET state = 'recurred',last_seen = ?1 WHERE fingerprint = 'source'", [NOW + 60]);
    f.write("UPDATE tasks SET slug = 'RUN-1' WHERE id = 'run'", []);
    f.store.with(|tx| {
        crate::integrate::store::give_back(tx, "run", "RUN-1 returned: red check.", Left::Red, NOW + 50)?;
        let hold = find(tx, PROJECT, Kind::Hold, &action)?.unwrap();
        super::judge(tx, PROJECT, &hold, 0, NOW + 70, (Wrong, "The owner released the unchanged subject."))?;
        let alert = find(tx, PROJECT, Kind::Alert, "alert")?.unwrap();
        incidents::wrong_alert(tx, incident, alert.id, NOW + 70)?;
        Ok(())
    }).unwrap();
    inspect([Wrong; 7]);
}

/// A typed exit from integration of the fixture's task `run`: the queue's own writer for a return, its notice for a landing.
fn leave(tx: &rusqlite::Transaction, how: Exit, at: i64) -> Result<(), String> {
    match how {
        Exit::Back(left) => give_back(tx, "run", "RUN-1 returned.", left, at),
        Exit::Landed => crate::tasks::ops::left_queue(tx, DISPATCHER, "RUN-1", "RUN-1 landed.", "landed", at).map(drop),
    }
}

#[derive(Clone, Copy, Debug)]
enum Exit { Back(Left), Landed }

#[test]
fn ac246_run_counts_each_typed_integration_return_of_its_episode() {
    use Verdict::{Right, Wrong};
    // red, a conflict, and out (a given-up batch, a human's return from a batch): each the queue's give_back
    // (the assessed episode's exit, the later attempt's exit, expected verdict, its reason)
    let cases = [
        (Some(Exit::Back(Left::Conflict)), None, Wrong, "Integration returned the task for a conflict."),
        (Some(Exit::Back(Left::Out)), None, Wrong, "Integration gave the task back without landing it."),
        (Some(Exit::Back(Left::Red)), None, Wrong, "Integration returned the task for a red check."),
        (Some(Exit::Landed), None, Right, "The task ran and finished without return."),
        (None, Some(Exit::Back(Left::Conflict)), Right, "The task ran and finished without return."),
    ];
    for (n, (exit, later, verdict, why)) in cases.into_iter().enumerate() {
        let f = Fixture::new(&format!("outcomes-returns-{n}"));
        signal(&f, "source", Rule::FailedRepeat, 2);
        f.task("run", Some("Bug"), "P1", "INTEGRATION", NOW - DAY);
        f.write("UPDATE tasks SET slug = 'RUN-1' WHERE id = 'run'", []);
        f.agent("agent");
        // an exit before the action belongs to an earlier attempt
        f.store.with(|tx| leave(tx, Exit::Back(Left::Conflict), NOW - 10)).unwrap();
        f.run("run", "assessed", "agent", NOW + 1);
        f.store.with(|tx| record(tx, &signals::get(tx, PROJECT, "source")?, Kind::Run, "run", "run", "assessed", NOW)).unwrap();
        if let Some(how) = exit {
            f.store.with(|tx| leave(tx, how, NOW + 10)).unwrap();
        }
        f.transition("run", NOW + 30, "IN PROGRESS", "DONE");
        f.run("run", "later", "agent", NOW + 40);
        if let Some(how) = later {
            f.store.with(|tx| leave(tx, how, NOW + 50)).unwrap();
        }
        let judged = f.store.with(|tx| outcome_evidence::check(tx, PROJECT, &find(tx, PROJECT, Kind::Run, "run")?.unwrap(), NOW + DAY));
        assert_eq!(judged.unwrap(), (verdict, why), "{exit:?} {later:?}");
    }
}

#[test]
fn ac247_ac263_alert_verdict_scopes_a_grouped_notice_and_orders_the_shared_line() {
    let f = Fixture::new("breaker-mixed-notice");
    signal(&f, "bad", Rule::FailedRepeat, 3);
    signal(&f, "good", Rule::CostOutlier, 2);
    f.write("INSERT INTO project_supervisor (project,enabled,generation,enabled_at) VALUES (?1,1,1,?3), (?2,1,1,?3)", (PROJECT, OTHER, NOW));
    f.store.with(|tx| {
        tx.execute("INSERT INTO supervisor_incidents (id,project,subject_kind,subject_id,opened_at,last_firing_at)
            VALUES (11,?1,'project','target',?3,?3), (12,?2,'project','elsewhere',?3,?3)", (PROJECT, OTHER, NOW)).map_err(sql)?;
        for (fingerprint, level) in [("bad", 3), ("good", 2)] {
            let s = signals::get(tx, PROJECT, fingerprint)?;
            let event: i64 = tx.query_row("INSERT INTO supervisor_incident_events
                (incident,at,kind,actor,fingerprint,detector,metric,level,value,threshold)
                VALUES (11,?1,'signal','system',?2,?3,'fixture',?4,5,4) RETURNING id",
                (NOW, fingerprint, &s.detector, level), |r| r.get(0)).map_err(sql)?;
            tx.execute("INSERT INTO supervisor_incident_signals VALUES (?1,11,?3,x'01',5,4,?2,0)",
                (fingerprint, event, level)).map_err(sql)?;
            record(tx, &s, Kind::Alert, &event.to_string(), "11", "", NOW)?;
        }
        let id: i64 = tx.query_row("SELECT id FROM supervisor_actions WHERE fingerprint = 'bad'", [], |r| r.get(0)).map_err(sql)?;
        assert!(incidents::wrong_alert(tx, 12, id, NOW + 1).is_err());
        assert!(incidents::wrong_alert(tx, 11, id, NOW + 1)?);
        assert!(!incidents::wrong_alert(tx, 11, id, NOW + 2)?);
        assert!(breakers::open_id(tx, PROJECT, Rule::FailedRepeat.id(), "R5")?.is_some());
        assert!(breakers::open_id(tx, PROJECT, Rule::CostOutlier.id(), "R5")?.is_none());
        assert!(breakers::open_id(tx, OTHER, Rule::FailedRepeat.id(), "R5")?.is_none());
        tx.execute("INSERT INTO supervisor_line_repeats VALUES (11,0,?1,?2)", (NOW, NOW+60)).map_err(sql)?;
        tx.execute("INSERT INTO supervisor_line_pending (incident,kind,level,signals,urgent)
            VALUES (12,'opened',3,1,0)", []).map_err(sql)?;
        let policy = line::Settings::default();
        let line_queue::Claim::Notice(critical) = line_queue::claim(tx, &policy, NOW + 601, false)?
            else { panic!("the other project's critical notice is due") };
        assert_eq!((critical.project.as_str(), critical.level), (OTHER, 3));
        let line_queue::Claim::Notice(healthy) = line_queue::claim(tx, &policy, NOW + 601, false)?
            else { panic!("the level-2 member of the first project's notice is due") };
        assert_eq!((healthy.project.as_str(), healthy.level, healthy.signals), (PROJECT, 2, 1));
        tx.execute("INSERT INTO supervisor_line_pending (incident,kind,level,signals,urgent,alerts)
            SELECT 11,'opened',3,2,0,json_group_array(id) FROM supervisor_actions WHERE kind = 'alert'", []).map_err(sql)?;
        tx.execute("DELETE FROM supervisor_incident_signals WHERE fingerprint = 'good'", []).map_err(sql)?;
        tx.execute("DELETE FROM supervisor_line_repeats WHERE incident = 12", []).map_err(sql)?;
        assert!(matches!(line_queue::claim(tx, &policy, NOW + 2000, false)?, line_queue::Claim::Discarded),
            "a stale healthy member cannot keep a queued page alive");
        tx.execute("INSERT INTO supervisor_line_repeats VALUES (11,0,?1,?2)", (NOW, NOW+60)).map_err(sql)?;
        assert!(matches!(line_queue::claim(tx, &policy, NOW + 2000, false)?, line_queue::Claim::Discarded),
            "a broken member cannot send a repeat");
        assert_eq!(tx.query_row("SELECT count(*) FROM supervisor_breaker_cards", [], |r| r.get::<_, i64>(0)).map_err(sql)?, 1);
        assert_eq!(tx.query_row("SELECT count(*) FROM supervisor_line_repeats", [], |r| r.get::<_, i64>(0)).map_err(sql)?, 0);
        Ok(())
    }).unwrap();
}

#[test]
fn ac247_rule_breaker_and_reopen_key() {
    let f = Fixture::new("breaker-reset");
    signal(&f, "trial", Rule::FailedRepeat, 2);
    f.task("history", Some("Bug"), "P2", "DONE", NOW - 8 * WEEK);
    f.event("history", NOW - 5 * WEEK, HUMAN, "comment", "Retained history.", None);
    f.event("history", NOW, HUMAN, "comment", "Recent history.", None);
    f.store.with(|tx| {
        let s = signals::get(tx, PROJECT, "trial")?;
        for source in ["first", "second"] {
            record(tx, &s, Kind::Hold, source, "", "", NOW)?;
            let a = find(tx, PROJECT, Kind::Hold, source)?.unwrap();
            for days in [1, 7, 28] {
                super::judge(tx, PROJECT, &a, days, NOW + 1, (Verdict::Wrong, "Fixture outcome."))?;
            }
            assert_eq!(breakers::open_id(tx, PROJECT, &s.detector, s.response)?.is_some(), source == "second");
        }
        let id = breakers::open_id(tx, PROJECT, &s.detector, s.response)?.unwrap();
        assert!(breakers::review(tx, PROJECT, id, NOW + 2).is_err());
        assert!(breakers::restore(tx, PROJECT, id, "invalid", NOW + 2).is_err());
        let saved = serde_json::from_value(json!({"detectors":{(s.detector.as_str()):{"threshold":{"absolute":3}}}})).unwrap();
        settings::write(tx, PROJECT, 0, saved, false)?;
        assert!(breakers::open_id(tx, PROJECT, &s.detector, s.response)?.is_some(), "changing settings does not restore autonomy");
        let review = breakers::review(tx, PROJECT, id, NOW + 2)?;
        let key = breakers::save_review(tx, PROJECT, id, &review)?;
        assert!(breakers::restore(tx, OTHER, id, &key, NOW + 2).is_err());
        assert!(breakers::restore(tx, PROJECT, id, "invalid", NOW + 2).is_err());
        breakers::restore(tx, PROJECT, id, &key, NOW + 2)?;
        assert!(breakers::open_id(tx, PROJECT, &s.detector, s.response)?.is_none());
        assert!(breakers::restore(tx, PROJECT, id, &key, NOW + 3).is_err());
        record(tx, &s, Kind::Hold, "next", "", "", NOW + 3)?;
        let a = find(tx, PROJECT, Kind::Hold, "next")?.unwrap();
        super::judge(tx, PROJECT, &a, 1, NOW + 4, (Verdict::Wrong, "Fixture outcome."))?;
        assert!(breakers::open_id(tx, PROJECT, &s.detector, s.response)?.is_none(), "old mistakes belong to a previous generation");
        Ok(())
    }).unwrap();
}


#[test]
fn ac247_auto_dispatch_filters_broken_families_before_the_candidate_limit() {
    let f = Fixture::new("breaker-auto");
    signal(&f, "rule", Rule::FailedRepeat, 2);
    f.agent("auto-agent");
    f.write("INSERT INTO project_supervisor (project,enabled,generation,enabled_at) VALUES (?1,1,1,?2)", (PROJECT, NOW));
    f.write("UPDATE projects SET auto_on = 1, auto_mode = 'order', auto_default = 'auto-agent',
        auto_severities = ?2, auto_wait_minutes = 1 WHERE id = ?1", (PROJECT, json!(["P0","P1"]).to_string()));
    f.task("family", None, "P0", "TODO", NOW - 1000);
    f.task("broken-child", None, "P0", "TODO", NOW - 1000);
    f.task("healthy", None, "P1", "TODO", NOW - 1000);
    f.write("UPDATE tasks SET parent = 'family' WHERE id = 'broken-child'", []);
    f.write("UPDATE tasks SET slug = 'TASK-' || number", []);
    f.store.with(|tx| {
        let s = signals::get(tx, PROJECT, "rule")?;
        record(tx, &s, Kind::Fix, "broken-child", "broken-child", "", NOW - 999)?;
        for source in ["error-1", "error-2"] {
            record(tx, &s, Kind::Hold, source, "", "", NOW - 2)?;
            super::judge(tx, PROJECT, &find(tx, PROJECT, Kind::Hold, source)?.unwrap(), 0, NOW,
                (Verdict::Wrong, "Owner released unchanged work."))?;
        }
        let setting = crate::tasks::auto::setting(tx, PROJECT)?;
        let free = || crate::tasks::auto::candidates(tx, PROJECT, &setting, NOW + 1, None);
        assert_eq!(free()?.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(), ["healthy"]);
        let blocked = crate::tasks::auto::breaker_candidates(tx, PROJECT, &setting, NOW + 1)?;
        assert_eq!(blocked.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(), ["family"]);
        tx.execute("UPDATE project_supervisor SET enabled = 0 WHERE project = ?1", [PROJECT]).map_err(sql)?;
        assert_eq!(free()?.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(), ["family", "healthy"]);
        assert!(breakers::open_id(tx, PROJECT, &s.detector, s.response)?.is_some());
        Ok(())
    }).unwrap();
}
