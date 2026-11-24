// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-238: a fix task archived before it ran frees its problem for one new task, and a second is the owner's ruling;
//! a measure-alone problem that still claims a cause gets the words code writes from its measure. AC-211: counts
//! grown on every evaluation pass keep one occurrence (`schema65.sql`).

use rusqlite::Transaction;
use serde_json::json;

use crate::tasks::{ops::{self, rows, sql}, store::HUMAN};

use super::{DAY, WEEK, engine_tests::{Fixture, NOW, PROJECT}, fix_tasks, register};
use super::register_tests::{enabled, seeded};

fn funded(name: &str) -> Fixture {
    let f = Fixture::new(name);
    enabled(&f, json!({"fixes_ceiling": 10.0}));
    f.task("baseline", Some("Feature"), "P2", "DONE", NOW - DAY);
    f.run("baseline", "baseline-run", "agent", NOW - DAY);
    f.cost("baseline-run", NOW - DAY, 100_000);
    f
}

fn backlog(f: &Fixture, key: &str) {
    seeded(f, key, ("flow.backlog_growth", "project", PROJECT, "growing_weeks"),
        json!({"value":4,"threshold":3,"baseline":3,"window_start":NOW-4*WEEK,"window_end":NOW}), json!([]));
}

fn problem_of(tx: &Transaction, key: &str) -> Result<register::Problem, String> {
    let id = tx.query_row("SELECT problem FROM supervisor_problem_signals WHERE fingerprint = ?1", [key], |r| r.get(0)).map_err(sql)?;
    register::get(tx, PROJECT, id, NOW + WEEK)
}

fn history(tx: &Transaction, id: i64) -> Result<Vec<(String, String)>, String> {
    rows(tx, "SELECT state, detail FROM supervisor_problem_events WHERE problem = ?1 ORDER BY id", [id], |r| Ok((r.get(0)?, r.get(1)?)))
}

#[test]
fn ac238_a_fix_task_archived_unrun_frees_its_problem_for_one_new_task() {
    let f = funded("archived-fix");
    for key in ["archived", "done-by-hand", "ran"] { backlog(&f, key); }
    let first = f.store.with(|tx| {
        let made = fix_tasks::advance(tx, PROJECT, NOW)?;
        assert_eq!(made.len(), 3, "one task per problem");
        let task = |key| problem_of(tx, key).map(|p| p.tasks[0].clone());
        Ok([task("archived")?, task("done-by-hand")?, task("ran")?])
    }).unwrap();
    let [archived, by_hand, ran] = first.map(|slug| f.store.with(|tx| ops::get(tx, &slug)).unwrap());
    f.transition(&by_hand.key, NOW + 1, "TODO", "DONE");
    f.run(&ran.key, "fix-run", "agent", NOW + 1);
    f.store.with(|tx| {
        for t in [&archived, &by_hand, &ran] { ops::archive(tx, HUMAN, &t.id, true, NOW + 2)?; }
        let made = fix_tasks::advance(tx, PROJECT, NOW + 3)?;
        assert_eq!(made.len(), 1, "only the problem whose task never ran and was not done gets a new one: {made:?}");
        let p = problem_of(tx, "archived")?;
        assert_eq!((p.state.as_str(), p.tasks.len()), ("fix_in_work", 2));
        let said = history(tx, p.id)?;
        assert!(said.contains(&("confirmed".into(), format!("Fix task {} was archived without running; the fix gate may make one new task.", archived.id))), "{said:?}");
        for key in ["done-by-hand", "ran"] {
            let other = problem_of(tx, key)?;
            assert_eq!((other.state.as_str(), other.tasks.len()), ("fix_in_work", 1), "{key} stays in work");
        }
        assert!(fix_tasks::advance(tx, PROJECT, NOW + 4)?.is_empty(), "one new task, never a second while it is on the board");
        let second = ops::get(tx, &made[0])?;
        ops::archive(tx, HUMAN, &second.id, true, NOW + 5)?;
        assert!(fix_tasks::advance(tx, PROJECT, NOW + 6)?.is_empty(), "the second archive makes no third task");
        let p = problem_of(tx, "archived")?;
        assert_eq!(p.state, "accepted", "the second archive is the owner's ruling");
        let ruling: String = tx.query_row("SELECT ruling FROM supervisor_problems WHERE id = ?1", [p.id], |r| r.get(0)).map_err(sql)?;
        assert_eq!(ruling, format!("Its fix tasks {} and {} were archived without running.", archived.id, second.id));
        Ok(())
    }).unwrap();
}

#[test]
fn ac238_a_measure_alone_problem_that_claims_a_cause_is_rewritten() {
    let f = funded("old-seed");
    backlog(&f, "old");
    backlog(&f, "round");
    let old_words = |key: &str| f.store.with(|tx| {
        let id = problem_of(tx, key)?.id;
        tx.execute("UPDATE supervisor_problems SET title = 'growing weeks: project',
            hypothesis = 'The condition measured by flow.backlog_growth is contributing to this problem.',
            disproof = 'The detector remains clear during the observation window without the proposed change.',
            options = '[{\"layer\":\"check\",\"action\":\"Confirm the cause in the retained evidence and fix the measured condition.\"}]'
            WHERE id = ?1", [id]).map_err(sql)?;
        Ok(id)
    }).unwrap();
    f.store.with(|tx| register::advance(tx, PROJECT, NOW)).unwrap();
    let (old, round) = (old_words("old"), old_words("round"));
    f.write("UPDATE supervisor_problems SET automatic = 0 WHERE id = ?1", [round]);
    f.store.with(|tx| {
        register::advance(tx, PROJECT, NOW + 1)?;
        let p = register::get(tx, PROJECT, old, NOW + 1)?;
        assert_eq!(p.title, "Backlog growth: 4 growing weeks, threshold 3");
        assert!(p.hypothesis.starts_with("No investigation has run yet: code opened this problem from the flow.backlog_growth measure alone"), "{}", p.hypothesis);
        assert_eq!(p.disproof, "Growing weeks stays within the threshold of 3 for 4 weeks with no change made.");
        assert!(p.fix_options[0].action.starts_with("Find the cause in the evidence rows"));
        assert_eq!(history(tx, old)?.last().unwrap().1, "Its text is rewritten from its detector's measure.");
        let kept = register::get(tx, PROJECT, round, NOW + 1)?;
        assert_eq!(kept.title, "growing weeks: project", "a problem that waits for a round's finding keeps its words");
        register::advance(tx, PROJECT, NOW + 2)?;
        assert_eq!(history(tx, old)?.iter().filter(|(_, d)| d.starts_with("Its text is rewritten")).count(), 1, "once");
        Ok(())
    }).unwrap();
}

#[test]
fn ac211_schema65_resets_counts_that_counted_evaluation_passes() {
    let f = Fixture::new("count-reset");
    for (key, count, state) in [("passes", 146, "new"), ("back", 404, "recurred"), ("once", 1, "raised"), ("ruled", 23, "new")] {
        backlog(&f, key);
        f.write("UPDATE supervisor_signals SET count = ?2, state = ?3 WHERE fingerprint = ?1", (key, count, state));
    }
    f.store.with(|tx| {
        let id = register::open(tx, PROJECT, super::register_tests::draft("Ruled", &["ruled"]), NOW)?;
        register::rule(tx, PROJECT, id, "The owner accepts this as is.", NOW)?;
        tx.execute_batch(include_str!("../tasks/schema65.sql")).map_err(sql)?;
        let counts = rows(tx, "SELECT fingerprint, count FROM supervisor_signals ORDER BY fingerprint", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        assert_eq!(counts, [("back".into(), 2), ("once".into(), 1), ("passes".into(), 1), ("ruled".into(), 1)]);
        let ruling: i64 = tx.query_row("SELECT ruling_count FROM supervisor_problem_signals WHERE fingerprint = 'ruled'", [], |r| r.get(0)).map_err(sql)?;
        assert_eq!(ruling, 1, "a ruling's count is no higher than its signal's, so its next repeat reopens it");
        Ok(())
    }).unwrap();
}
