// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-253, AC-254 and AC-257: proposals, mutes and detector retirement.

use rusqlite::Transaction;
use serde_json::{Value, json};

use super::{DAY, Mode, Rule, Threshold, WEEK, config_changes as config, config_lifecycle as lifecycle,
    engine_tests::{Fixture, NOW, OTHER, PROJECT}, settings};
use crate::tasks::ops::{self, sql};

const NAME: &str = "flow.failed_repeat";
const RULE: Rule = Rule::FailedRepeat;
const ROUND: &str = "reflection-run";

fn fixture(name: &str) -> Fixture {
    with_failures(name, 3)
}

fn with_failures(name: &str, failures: usize) -> Fixture {
    let f = Fixture::new(name);
    f.write("INSERT INTO project_supervisor (project,enabled,generation,enabled_at) VALUES (?1,1,1,?2)",
        (PROJECT, NOW - 20 * WEEK));
    f.task("repeat", Some("Bug"), "P2", "IN PROGRESS", NOW - 20 * WEEK);
    for i in 0..failures {
        f.failed_run("repeat", &format!("failed-{i}"), NOW - 50 + 5 * i as i64);
    }
    f
}

fn preview(tx: &Transaction, change: &Value, at: i64) -> Result<Value, String> {
    config::preview(tx, PROJECT, ROUND, serde_json::from_value(change.clone()).unwrap(), at)
}

fn submission(tx: &Transaction, change: &Value, replay: &Value, at: i64) -> Result<config::Submitted, String> {
    let input = serde_json::from_value(json!({"change": change, "replay": replay})).unwrap();
    let verified = config::verify(tx, PROJECT, ROUND, input, at)?;
    config::submit(tx, PROJECT, ROUND, verified, at)
}

fn tune(threshold: f64) -> Value {
    json!({"detector": NAME, "action":{"operation":"tune","threshold":{"absolute":threshold}}})
}

fn choose(tx: &Transaction, id: i64, task: &str, choice: &str, at: i64) -> Result<(), String> {
    let question: String = tx.query_row("SELECT question FROM supervisor_config_changes WHERE id = ?1", [id], |r| r.get(0)).map_err(sql)?;
    config::owner_choice(tx, &ops::get(tx, task)?, &question, choice, at)
}

#[test]
fn ac253_config_classes_replay_and_revert() {
    let f = fixture("config-revert");
    f.store.with(|tx| {
        let change = tune(2.5);
        let evidence = preview(tx, &change, NOW)?;
        assert_eq!(evidence["class"], "tune");
        assert_eq!(evidence["replay"]["before"]["firings"], 1);
        assert_eq!(evidence["replay"]["after"]["firings"], 1);
        assert_eq!(evidence["replay"]["labeled_weeks"], json!([]));
        assert!(serde_json::from_value::<config::Submission>(json!({"change": change})).is_err());
        let mut forged = evidence["replay"].clone();
        forged["after"]["firings"] = json!(900);
        assert!(submission(tx, &change, &forged, NOW).unwrap_err().contains("does not reproduce"));

        let applied = submission(tx, &change, &evidence["replay"], NOW)?;
        assert_eq!(settings::read(tx, PROJECT)?.settings.detector(RULE).threshold, Threshold::Absolute(2.5));
        assert!(config::weekly(tx, PROJECT, NOW)?.contains("1 → 1 firings"));
        let (recorded, replay) = config::for_reflection(tx, PROJECT, ROUND, applied.change)?;
        assert!(recorded.contains(NAME));
        assert!(replay.contains("weekly cutoffs"));
        assert!(config::for_reflection(tx, PROJECT, "different-round", applied.change).is_err());

        let mut owner = settings::read(tx, PROJECT)?;
        owner.settings.budgets.round_token_ceiling = Some(100);
        settings::write_at(tx, PROJECT, owner.revision, owner.settings, NOW + 1)?;
        assert!(config::revert(tx, OTHER, applied.change, NOW + 1).is_err());
        config::revert(tx, PROJECT, applied.change, NOW + 2)?;
        config::revert(tx, PROJECT, applied.change, NOW + 3)?;
        let after = settings::read(tx, PROJECT)?;
        assert_eq!(after.settings.detector(RULE).threshold, Threshold::Absolute(2.0));
        assert_eq!(after.settings.budgets.round_token_ceiling, Some(100));
        assert!(config::weekly(tx, PROJECT, NOW + 3)?.contains("reverted"));
        Ok(())
    }).unwrap();
}

#[test]
fn ac253_coverage_reduction_and_open_breaker_require_owner() {
    let f = with_failures("config-owner", 2);
    f.store.with(|tx| {
        let change = tune(2.5);
        let evidence = preview(tx, &change, NOW)?;
        assert_eq!(evidence["class"], "owner");
        assert_eq!(evidence["replay"]["after"]["firings"], 0);
        let pending = submission(tx, &change, &evidence["replay"], NOW)?;
        let card = pending.task.unwrap();
        assert_eq!(ops::get(tx, &card)?.status, "WAIT FOR HUMAN");
        assert_eq!(settings::read(tx, PROJECT)?.settings.detector(RULE).threshold, Threshold::Absolute(2.0));
        choose(tx, pending.change, &card, "apply", NOW + 1)?;
        assert_eq!(settings::read(tx, PROJECT)?.settings.detector(RULE).threshold, Threshold::Absolute(2.5));
        config::revert(tx, PROJECT, pending.change, NOW + 2)?;
        let action: i64 = tx.query_row("INSERT INTO supervisor_actions
            (project,detector,response,kind,source,fingerprint,target,at) VALUES (?1,?2,'R5','alert','fixture','','',?3)
            RETURNING id", (PROJECT, NAME, NOW), |r| r.get(0)).map_err(sql)?;
        tx.execute("INSERT INTO supervisor_breakers (project,detector,response,opened_at,opened_by,config)
            VALUES (?1,?2,'R5',?3,?4,'null')", (PROJECT, NAME, NOW, action)).map_err(sql)?;
        let change = tune(1.8);
        let evidence = preview(tx, &change, NOW + 3)?;
        assert_eq!(evidence["class"], "owner");
        Ok(())
    }).unwrap();
}

#[test]
fn ac253_enable_runs_in_shadow_and_engine_requests_keep_their_class() {
    let f = fixture("config-shadow");
    f.store.with(|tx| {
        let mut saved = settings::read(tx, PROJECT)?;
        saved.settings.detectors.entry(NAME.into()).or_default().mode = Some(Mode::Off);
        settings::write_at(tx, PROJECT, saved.revision, saved.settings, NOW - 1)?;
        let change = json!({"detector":NAME,"action":{"operation":"enable"}});
        let result = preview(tx, &change, NOW)?;
        assert_eq!(result["class"], "extend");
        let applied = submission(tx, &change, &result["replay"], NOW)?;
        assert_eq!(settings::read(tx, PROJECT)?.settings.mode(RULE), Mode::Shadow);
        assert!(config::promote(tx, PROJECT, applied.change, NOW).unwrap_err().contains("three distinct"));
        let report = ops::create(tx, "human", ops::NewTask { title: "Trial verdicts".into(),
            project: Some(PROJECT.into()), ..Default::default() }, NOW + 1)?;
        let event = ops::comment(tx, "human", &report.id, "Review.", NOW + 1)?;
        tx.execute("INSERT INTO supervisor_review_reports VALUES (?1, ?2, ?3, ?4)",
            (&report.key, PROJECT, NOW + 1, event)).map_err(sql)?;
        for fp in ["trial-a", "trial-b", "trial-c"] {
            tx.execute("INSERT INTO supervisor_signals
                (fingerprint,project,detector,subject_kind,subject_id,metric,evidence,measure,scores,first_seen,last_seen,count,state)
                VALUES (?1,?2,?3,'task','repeat','failed_runs','[]','{}','{}',?4,?4,1,'shadow')",
                (fp, PROJECT, NAME, NOW + 1)).map_err(sql)?;
            tx.execute("INSERT INTO supervisor_review_items VALUES (?1, ?2, 0)", (&report.key, fp)).map_err(sql)?;
            tx.execute("INSERT INTO supervisor_verdicts (task, fingerprint, at, verdict) VALUES (?1, ?2, ?3, 'useful')",
                (&report.key, fp, NOW + 1)).map_err(sql)?;
        }
        config::promote(tx, PROJECT, applied.change, NOW + 2)?;
        let live = settings::read(tx, PROJECT)?.settings;
        assert_eq!(live.mode(RULE), Mode::Live);
        assert!(live.owner_live(RULE));
        let promotion = tx.query_row("SELECT max(id) FROM supervisor_config_changes", [], |r| r.get(0)).map_err(sql)?;
        config::revert(tx, PROJECT, promotion, NOW + 3)?;
        config::revert(tx, PROJECT, applied.change, NOW + 4)?;
        assert_eq!(settings::read(tx, PROJECT)?.settings.mode(RULE), Mode::Off);
        let engine = json!({"detector":NAME,"action":{"operation":"engine","requirement":"query"}});
        let evidence = preview(tx, &engine, NOW + 5)?;
        assert_eq!(evidence["class"], "engine");
        let pending = submission(tx, &engine, &evidence["replay"], NOW + 5)?;
        assert_eq!(ops::get(tx, &pending.task.unwrap())?.project, PROJECT);
        let mut saved = settings::read(tx, PROJECT)?;
        saved.settings.engine_board = Some(OTHER.into());
        settings::write_at(tx, PROJECT, saved.revision, saved.settings, NOW + 6)?;
        let evidence = preview(tx, &engine, NOW + 6)?;
        let queued = submission(tx, &engine, &evidence["replay"], NOW + 6)?;
        let task = ops::get(tx, &queued.task.unwrap())?;
        assert_eq!(task.project, OTHER);
        let body: String = tx.query_row("SELECT body FROM tasks WHERE id = ?1", [&task.key], |r| r.get(0)).map_err(sql)?;
        assert!(body.contains("named query"));
        assert!(!body.contains(ROUND));
        assert!(!super::task_policy::check_marked(tx, &task, true)?);
        Ok(())
    }).unwrap();
}

#[test]
fn ac254_mute_authority_and_expiry() {
    let f = fixture("config-mute");
    f.store.with(|tx| {
        let until = NOW + 2 * DAY;
        let change = json!({"detector":NAME,"action":{"operation":"mute","mode":"digest","reason":"Repeated alerts","until":until}});
        let evidence = preview(tx, &change, NOW)?;
        assert_eq!(evidence["class"], "tune");
        submission(tx, &change, &evidence["replay"], NOW)?;
        let old = settings::read(tx, PROJECT)?.settings;
        assert_eq!(old.mode(RULE), Mode::Live);
        assert_eq!(old.delivery(RULE, until - 1), Some(Mode::Digest));
        assert!(old.expired_since(RULE, until - 1, until));
        assert_eq!(old.delivery(RULE, until), None);
        let mute = old.detectors[NAME].mute.as_ref().unwrap();
        assert_eq!(mute.owner.as_deref(), Some(ROUND));
        assert_eq!(mute.reason, "Repeated alerts");
        assert_eq!(preview(tx, &change, NOW + 1)?["class"], "owner");
        let malformed = json!({"detector":NAME,"action":{"operation":"mute","mode":"shadow","reason":"","until":until}});
        assert!(preview(tx, &malformed, NOW).unwrap_err().contains("reason and an end"));
        let length = json!({"detector":NAME,"action":{"operation":"mute","mode":"off","reason":"Review","until":NOW+15*DAY}});
        assert_eq!(preview(tx, &length, NOW)?["class"], "owner");
        Ok(())
    }).unwrap();
}

#[test]
fn ac257_sunset_age_and_exemptions() {
    let f = fixture("config-sunset");
    f.write("INSERT INTO supervisor_signals
        (fingerprint,project,detector,subject_kind,subject_id,metric,evidence,measure,scores,first_seen,last_seen,count,state)
        VALUES ('protected',?1,'flow.failed_repeat','task','repeat','failed_runs','[]','{}',
            '{\"impact\":3,\"blast\":0,\"reversibility\":0,\"confidence\":1}',?2,?2,1,'new')",
        (PROJECT, NOW - 15 * WEEK));
    f.write("INSERT INTO supervisor_actions (project,detector,response,kind,source,fingerprint,target,at)
        VALUES (?1,'flow.backlog_growth','R4','run','fixture','','',?2)", (PROJECT, NOW - WEEK));
    f.store.with(|tx| {
        let enabled = NOW - 8 * WEEK;
        lifecycle::advance(tx, PROJECT, enabled, NOW - 1)?;
        assert!(lifecycle::status(tx, PROJECT)?.is_empty());
        lifecycle::advance(tx, PROJECT, enabled, NOW)?;
        let weekly = lifecycle::on_weekly(tx, PROJECT)?;
        assert!(weekly.contains("flow.wait_human_age"));
        assert!(!weekly.contains(NAME));
        assert!(!weekly.contains("flow.backlog_growth"));
        assert!(!weekly.contains("self.collector_dead"));
        lifecycle::advance(tx, PROJECT, enabled, NOW + 8 * WEEK)?;
        assert!(lifecycle::status(tx, PROJECT)?.iter().any(|r| r.detector == "flow.wait_human_age" && r.removal));
        assert_eq!(settings::read(tx, PROJECT)?.settings.mode(Rule::WaitHuman), Mode::Live);
        Ok(())
    }).unwrap();
}
