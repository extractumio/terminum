// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-240, AC-241, AC-242 and AC-243 through the retained store.

use rusqlite::Transaction;
use serde_json::json;
use crate::tasks::{ops::{self, sql}, schedules};
use super::{DAY, WEEK, decisions, digest, review_data, engine_tests::{Fixture, NOW, OTHER, PROJECT}, nudges, register, review, settings};

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

fn confirmed(tx: &Transaction, key: &str, at: i64) -> Result<i64, String> {
    let draft = serde_json::from_value(json!({"title":"Measured change","lens":"flow","hypothesis":"The known cause.",
        "disproof":"It still repeats.","confidence":0.8,"effort":1,"signals":[key],
        "fix_options":[{"layer":"check","action":"Check the recorded failure."}]})).unwrap();
    let problem = register::open(tx, PROJECT, draft, at)?;
    register::update(tx, PROJECT, serde_json::from_value(json!({"id":problem,"state":"confirmed"})).unwrap(), at)?;
    Ok(problem)
}

fn choice(problem: i64, kind: &str, recurring: Option<&str>) -> decisions::Proposal {
    serde_json::from_value(json!({"problem":problem,"kind":kind,"why":"The choice belongs to the owner.",
        "recommend_why":"The first fix is reversible.","waiting":"One extra week of delay.","cost":2,"recommended":"first",
        "options":[{"id":"first","label":"First fix"},{"id":"second","label":"Second fix"}],"recurring":recurring})).unwrap()
}

fn propose(tx: &Transaction, key: &str, run: &str, kind: &str, at: i64) -> Result<(), String> {
    let problem = confirmed(tx, key, at)?;
    let proposed = decisions::propose(tx, PROJECT, Some(run), choice(problem, kind, None), at)?;
    assert_eq!(proposed["decision"], decisions::propose(tx, PROJECT, Some(run), choice(problem, kind, None), at)?["decision"]);
    assert!(decisions::propose(tx, OTHER, Some(run), choice(problem, kind, None), at).is_err());
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
        assert_eq!(decisions::cards(tx, PROJECT)?.len(), 1, "deferred decisions appear only in the weekly list");
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

fn session(f: &Fixture, key: &str, session: &str, detector: &str, measure: serde_json::Value) {
    let scores = json!({"impact":0,"blast":0,"reversibility":0,"confidence":1.0});
    f.write("INSERT INTO supervisor_signals (fingerprint, project, detector, subject_kind, subject_id, metric,
        evidence, measure, scores, first_seen, last_seen, count, state)
        VALUES (?1, ?2, ?3, 'session', ?4, 'tool_failures', '[]', ?5, ?6, ?7, ?7, 1, 'raised')",
        (key, PROJECT, detector, session, measure.to_string(), scores.to_string(), NOW));
}

fn figures(value: f64, threshold: f64) -> serde_json::Value {
    json!({"value":value,"threshold":threshold,"baseline":1,"window_start":NOW-DAY,"window_end":NOW})
}

#[test]
fn ac240_two_drifting_sessions_open_a_retro_card() {
    let f = Fixture::new("drift-card");
    enabled(&f, NOW);
    session(&f, "loop-1", "claude/s1", "drift.loop", figures(5.0, 2.0));
    session(&f, "agents-2", "codex/s2", "drift.subagents", figures(4.0, 2.0));
    session(&f, "loop-3", "claude/s3", "drift.loop", json!({"value":null,"threshold":2}));
    session(&f, "loop-4", "claude/s4", "drift.loop", figures(1.0, 2.0));
    f.store.with(|tx| {
        let card = decisions::retro(tx, PROJECT, NOW)?.expect("impact 0 counters above threshold open a card");
        let task = ops::get(tx, &card)?;
        assert_eq!(task.status, "WAIT FOR HUMAN");
        let text: String = tx.query_row("SELECT body FROM tasks WHERE id = ?1", [&task.key], |r| r.get(0)).map_err(sql)?;
        for named in ["`claude/s1`", "`codex/s2`", "/retro", "no transcript", "Not counted, their counters carry no figure: `claude/s3`"] {
            assert!(text.contains(named), "{named} in {text}");
        }
        assert!(!text.contains("s4"), "a session under its threshold is not named");
        let recommended: String = tx.query_row("SELECT json_extract(data, '$.question.recommended') FROM events
            WHERE task = ?1 AND json_extract(data, '$.questions') = 1", [&task.key], |r| r.get(0)).map_err(sql)?;
        assert_eq!(recommended, "retro");
        assert!(decisions::retro(tx, PROJECT, NOW + 1)?.is_none(), "the same sessions are asked about once");
        tx.execute("UPDATE supervisor_signals SET state = 'resolved' WHERE fingerprint = 'agents-2'", []).map_err(sql)?;
        assert!(decisions::retro(tx, PROJECT, NOW + 2)?.is_none(), "a set inside one asked about asks nothing new");
        tx.execute("UPDATE supervisor_signals SET state = 'raised' WHERE fingerprint = 'agents-2'", []).map_err(sql)?;
        tx.execute("UPDATE supervisor_signals SET measure = ?1 WHERE fingerprint = 'loop-4'", [figures(3.0, 2.0).to_string()]).map_err(sql)?;
        assert_eq!(decisions::retro(tx, PROJECT, NOW + 3)?.as_deref(), Some(card.as_str()), "a third session joins the waiting card");
        let text: String = tx.query_row("SELECT body FROM tasks WHERE id = ?1", [&task.key], |r| r.get(0)).map_err(sql)?;
        assert!(text.contains("3 sessions exceeded") && text.contains("`claude/s4`"), "{text}");
        let cards: i64 = tx.query_row("SELECT count(*) FROM tasks WHERE title = 'Review repeated session drift'", [], |r| r.get(0)).map_err(sql)?;
        assert_eq!(cards, 1);
        assert!(review_data::decisions(tx, PROJECT, NOW + 1)?.contains(&format!("- {card} · Review repeated session drift")));
        assert!(decisions::retro(tx, OTHER, NOW)?.is_none());
        Ok(())
    }).unwrap();
}

#[test]
fn ac240_one_drifting_session_waits_for_the_weekly_list() {
    let f = Fixture::new("drift-deferred");
    enabled(&f, NOW);
    session(&f, "loop-1", "claude/s1", "drift.loop", figures(5.0, 2.0));
    session(&f, "loop-2", "claude/s2", "drift.loop", json!({}));
    f.store.with(|tx| {
        assert!(decisions::retro(tx, PROJECT, NOW)?.is_none(), "a session without figures does not make a pair");
        let deferred: i64 = tx.query_row("SELECT count(*) FROM supervisor_drift_questions WHERE task IS NULL", [], |r| r.get(0)).map_err(sql)?;
        assert_eq!(deferred, 1);
        let review = review_data::decisions(tx, PROJECT, NOW + 1)?;
        assert!(review.contains("- Session drift\n  Why you are needed:\n  One session exceeded"), "{review}");
        assert!(review.contains("Not counted, their counters carry no figure: `claude/s2`"));
        assert!(!review_data::decisions(tx, PROJECT, NOW + WEEK + 1)?.contains("Session drift"), "the list is the week's");
        Ok(())
    }).unwrap();
}

#[test]
fn ac240_standing_rule_weighed_by_its_recurring_question() {
    let f = Fixture::new("standing-rule");
    enabled(&f, NOW);
    f.task("work", Some("Bug"), "P2", "IN PROGRESS", NOW - WEEK);
    let asked = |at| f.event("work", at, "human", "comment", "Which port should the test use?", Some(json!({"questions":true})));
    let first = asked(NOW - 3 * DAY);
    asked(NOW - 2 * DAY);
    asked(NOW - DAY);
    let once = f.event("work", NOW - DAY, "human", "comment", "Asked once?", Some(json!({"questions":true})));
    let unrelated = asked(NOW - DAY);
    seed(&f, "rule", "work", 2, "raised", NOW);
    let evidence = json!([{"table":"events","id":first}, {"table":"events","id":once}, {"table":"events","id":"missing"}]);
    f.write("UPDATE supervisor_signals SET evidence = ?1 WHERE fingerprint = 'rule'", [evidence.to_string()]);
    f.run("work", "rule-run", "agent", NOW);
    f.write("INSERT INTO supervisor_rounds (id, run, project, generation, fingerprint, signal_revision, case_file, deadline)
        VALUES ('rule', 'rule-run', ?1, 1, 'rule', 1, '{}', ?2)", (PROJECT, NOW + 600));
    let card = f.store.with(|tx| {
        let problem = confirmed(tx, "rule", NOW)?;
        let refused = |kind, recurring| decisions::propose(tx, PROJECT, Some("rule-run"), choice(problem, kind, recurring), NOW).unwrap_err();
        assert!(refused("standing_rule", None).contains("names the recurring question"));
        assert!(refused("standing_rule", Some(&once)).contains("asked once"));
        assert!(refused("standing_rule", Some("missing")).contains("No question"));
        assert!(refused("standing_rule", Some(&unrelated)).contains("this problem's signal evidence"));
        assert!(refused("fix_options", Some(&first)).contains("Only a standing rule"));
        decisions::propose(tx, PROJECT, Some("rule-run"), choice(problem, "standing_rule", Some(&first)), NOW)?;
        tx.execute("UPDATE supervisor_rounds SET finding = '{}'", []).map_err(sql)?;
        Ok(decisions::advance(tx, PROJECT, NOW)?.pop().expect("a confirmed high-impact standing rule opens a card"))
    }).unwrap();
    let key = f.store.with(|tx| ops::get(tx, &card)).unwrap().key;
    let question: String = f.store.with(|tx| tx.query_row("SELECT id FROM events WHERE task = ?1
        AND json_extract(data, '$.questions') = 1", [&key], |r| r.get(0)).map_err(sql)).unwrap();
    f.event(&key, NOW + 1, "human", "comment", "First fix", Some(json!({"choice":{"question":question,"option":"first"}})));
    asked(NOW + 2 * DAY);
    f.store.with(|tx| {
        let recurring: String = tx.query_row("SELECT recurring FROM supervisor_decisions", [], |r| r.get(0)).map_err(sql)?;
        assert_eq!(recurring, "Which port should the test use?", "code keeps the question's text, not the round's words");
        let soon = review_data::decisions(tx, PROJECT, NOW + 3600)?;
        assert!(soon.contains(&format!("{card}: standing rule on register problem")) && soon.contains("Effect unknown: accepted less than a day ago."), "{soon}");
        let week = review_data::decisions(tx, PROJECT, NOW + 1 + WEEK)?;
        assert!(week.contains("asked 4 times in the 7.0 days before you accepted it and 1 in the same span after: 3 fewer."), "{week}");
        assert!(week.contains("Askings of the recurring questions of the standing rules below: 3 fewer."), "{week}");
        tx.execute("UPDATE supervisor_decisions SET recurring = NULL", []).map_err(sql)?;
        let unnamed = review_data::decisions(tx, PROJECT, NOW + 1 + WEEK)?;
        assert!(unnamed.contains("Effect unknown: it names no recurring question."), "{unnamed}");
        assert!(unnamed.contains("The effect of the standing rules below is unknown."), "{unnamed}");
        assert!(review_data::decisions(tx, PROJECT, NOW + 1 + 5 * WEEK)?.contains("No standing rule was accepted in the last four weeks."));
        tx.execute("UPDATE supervisor_decisions SET recurring = ?1", [&recurring]).map_err(sql)?;
        Ok(())
    }).unwrap();
    for day in 3..8 { asked(NOW + day * DAY); }
    f.store.with(|tx| {
        let more = review_data::decisions(tx, PROJECT, NOW + 1 + WEEK)?;
        assert!(more.contains("4 times in the 7.0 days before you accepted it and 6 in the same span after: 2 more."), "{more}");
        assert!(more.contains("standing rules below: 2 more."), "{more}");
        tx.execute("UPDATE events SET data = json_set(data, '$.choice.option', 'second') WHERE json_extract(data, '$.choice.option') = 'first'", []).map_err(sql)?;
        assert!(review_data::decisions(tx, PROJECT, NOW + 1 + WEEK)?.contains("No standing rule was accepted"), "another option accepts no rule");
        Ok(())
    }).unwrap();
}
