// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-240, AC-241, AC-242 and AC-243 through the retained store.

use rusqlite::Transaction;
use serde_json::json;
use crate::tasks::{ops::{self, sql}, schedules};
use super::{DAY, WEEK, decisions, digest, engine_tests::{Fixture, NOW, OTHER, PROJECT}, nudges, register, review, settings};

fn enabled(f: &Fixture, at: i64) {
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at, last_ok)
        VALUES (?1, 1, 1, ?2, ?2)", (PROJECT, at));
}

fn seed(f: &Fixture, key: &str, task: &str, level: u8, state: &str, at: i64) {
    let scores = json!({"impact":level,"blast":0,"reversibility":0,"confidence":1.0});
    let measure = json!({"value":4,"threshold":2,"baseline":2,"window_start":at-WEEK,"window_end":at});
    f.write("INSERT INTO supervisor_signals (fingerprint, project, detector, subject_kind, subject_id, metric,
        evidence, measure, scores, first_seen, last_seen, count, state)
        VALUES (?1, ?2, 'drift.diff_outlier', 'task', ?3, 'diff_size', '[]', ?4, ?5, ?6, ?6, 1, ?7)",
        (key, PROJECT, task, measure.to_string(), scores.to_string(), at, state));
}

fn propose(tx: &Transaction, key: &str, run: &str, kind: &str, at: i64) -> Result<(), String> {
    let draft = serde_json::from_value(json!({"title":"Measured change","lens":"flow","hypothesis":"The known cause.",
        "disproof":"It still repeats.","confidence":0.8,"effort":1,"signals":[key],
        "fix_options":[{"layer":"check","action":"Check the recorded failure."}]})).unwrap();
    let problem = register::open(tx, PROJECT, draft, at)?;
    register::update(tx, PROJECT, serde_json::from_value(json!({"id":problem,"state":"confirmed"})).unwrap(), at)?;
    let choice = || serde_json::from_value(json!({"problem":problem,"kind":kind,"why":"The choice belongs to the owner.",
        "recommend_why":"The first fix is reversible.","waiting":"One extra week of delay.","cost":2,"recommended":"first",
        "options":[{"id":"first","label":"First fix"},{"id":"second","label":"Second fix"}]})).unwrap();
    let proposed = decisions::propose(tx, PROJECT, Some(run), choice(), at)?;
    assert_eq!(proposed["decision"], decisions::propose(tx, PROJECT, Some(run), choice(), at)?["decision"]);
    assert!(decisions::propose(tx, OTHER, Some(run), choice(), at).is_err());
    Ok(())
}

#[test]
fn ac240_decision_card_routing() {
    let f = Fixture::new("decision-routing");
    enabled(&f, NOW);
    f.task("work", Some("Bug"), "P2", "IN PROGRESS", NOW);
    for (signal, run, level) in [("high", "high-run", 2), ("routine", "routine-run", 3), ("low", "low-run", 1)] {
        seed(&f, signal, "work", level, "raised", NOW);
        f.run("work", run, "agent", NOW);
        f.write("INSERT INTO supervisor_rounds (id, run, project, generation, fingerprint, signal_revision, case_file, deadline)
            VALUES (?1, ?2, ?3, 1, ?1, 1, '{}', ?4)", (signal, run, PROJECT, NOW + 600));
    }
    f.store.with(|tx| {
        for (key, run, kind) in [("high","high-run","fix_options"),("routine","routine-run","routine"),("low","low-run","fix_options")] {
            propose(tx, key, run, kind, NOW)?;
        }
        assert!(decisions::advance(tx, PROJECT, NOW)?.is_empty(), "an unchecked finding opens nothing");
        tx.execute("UPDATE supervisor_rounds SET finding = '{}'", []).map_err(sql)?;
        let cards = decisions::advance(tx, PROJECT, NOW + 1)?;
        assert_eq!(cards.len(), 1);
        assert_eq!(ops::get(tx, &cards[0])?.status, "WAIT FOR HUMAN");
        assert_eq!(decisions::pending(tx, PROJECT)?.iter().filter(|d| d.task.is_none()).count(), 2);
        assert!(decisions::advance(tx, PROJECT, NOW + 2)?.is_empty());
        Ok(())
    }).unwrap();
}

#[test]
fn ac241_nudge_updates_one_comment() {
    let f = Fixture::new("nudge-once");
    f.task("work", Some("Bug"), "P2", "TODO", NOW);
    f.write("UPDATE tasks SET slug = 'WORK-1' WHERE id = 'work'", []);
    seed(&f, "drift", "work", 1, "raised", NOW);
    seed(&f, "shadow", "work", 2, "shadow", NOW);
    seed(&f, "ignored", "work", 0, "raised", NOW);
    f.store.with(|tx| {
        let comments = nudges::collected(tx, PROJECT, NOW, NOW)?;
        assert_eq!(comments.len(), 1);
        assert!(comments[0].1.is_some());
        assert!(nudges::collected(tx, OTHER, NOW, NOW)?.is_empty());
        assert!(nudges::collected(tx, PROJECT, NOW, NOW)?.is_empty());
        tx.execute("UPDATE supervisor_signals SET measure = json_set(measure, '$.value', 10) WHERE fingerprint = 'drift'", []).map_err(sql)?;
        assert_eq!(nudges::collected(tx, PROJECT, NOW, NOW + 1)?, vec![(comments[0].0.clone(), None)]);
        let count: i64 = tx.query_row("SELECT count(*) FROM events WHERE kind = 'comment' AND task = 'work' AND
            text LIKE 'Supervisor detected drift in this session.%' AND text LIKE '%measured 10%'", [], |r| r.get(0)).map_err(sql)?;
        assert_eq!(count, 1);
        Ok(())
    }).unwrap();
}

#[test]
fn ac243_digest_only_when_new() {
    let f = Fixture::new("exception-digest");
    enabled(&f, NOW - DAY);
    seed(&f, "shadow", "s", 2, "shadow", NOW);
    seed(&f, "quiet", "s", 0, "new", NOW);
    assert!(!f.store.with(|tx| digest::publish(tx, PROJECT, NOW)).unwrap());
    for i in 0..7 { seed(&f, &format!("news-{i}"), "s", 1, "raised", NOW); }
    f.store.with(|tx| {
        assert!(digest::publish(tx, PROJECT, NOW + 1)?);
        assert!(!digest::publish(tx, PROJECT, NOW + 2)?);
        let result = serde_json::to_value(digest::latest(tx, PROJECT)?).unwrap();
        assert_eq!(result["entries"].as_array().unwrap().len(), 5);
        assert_eq!(result["last_ok"], NOW - DAY, "the actual stale heartbeat is retained");
        assert_eq!(result["entries"][0]["measure"]["baseline"], 2.0);
        assert!(!digest::publish(tx, PROJECT, NOW + DAY)?, "a quiet day produces nothing");
        assert!(digest::latest(tx, OTHER)?.is_none());
        Ok(())
    }).unwrap();
}

#[test]
fn ac242_review_schedule_and_charter_due_once() {
    let f = Fixture::new("weekly-schedule");
    enabled(&f, NOW);
    f.agent("strong-agent");
    f.write("UPDATE agents SET at_once = 1 WHERE id = 'strong-agent'", []);
    f.store.with(|tx| {
        let pack = serde_json::from_value(json!({"tiers":{"strong":"strong-agent"},"charter":{"priority_review_at":NOW}})).unwrap();
        assert!(!review::sync(tx, PROJECT, NOW)?);
        settings::write(tx, PROJECT, 0, pack, false)?;
        assert!(review::sync(tx, PROJECT, NOW)?);
        assert!(!review::sync(tx, PROJECT, NOW + 1)?);
        let list = schedules::all(tx)?;
        assert_eq!(list.len(), 1);
        assert!(list[0].supervisor_review && list[0].active());
        assert_eq!(schedules::kept_by_agent(tx)?["strong-agent"].total(), 1);
        decisions::charter_due(tx, PROJECT, NOW)?;
        decisions::charter_due(tx, PROJECT, NOW + WEEK)?;
        let count: i64 = tx.query_row("SELECT count(*) FROM supervisor_charter_questions", [], |r| r.get(0)).map_err(sql)?;
        assert_eq!(count, 1);
        Ok(())
    }).unwrap();
}
