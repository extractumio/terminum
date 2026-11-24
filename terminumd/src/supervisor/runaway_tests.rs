// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213, AC-245: the cost outlier's runaway raise to level 3, from the real detector over a fixture store.

use crate::dispatcher::runs;
use crate::tasks::ops::{self, sql};
use super::{DAY, Evaluation, Finding, Rule, Window, engine_tests::{Fixture, PROJECT}, evaluate, holds::{self, Disposition},
    holds_tests::{arm, working_lead}, measure_detector, runaway, signals};

/// gpt-6-sol's output price is $10 a million tokens.
const DOLLAR: i64 = 100_000;

/// Twenty earlier Bug tasks of $1 to $3 (p95 near $3), and task `lead`'s working run `original` at `dollars`.
fn project(name: &str, dollars: i64, at: i64) -> Fixture {
    let f = Fixture::new(name);
    arm(&f, Rule::CostOutlier, at);
    working_lead(&f, at);
    for n in 0..20 {
        let (task, run) = (format!("old-{n}"), format!("old-{n}-run"));
        f.task(&task, Some("Bug"), "P2", "DONE", at - 36 * DAY);
        f.run(&task, &run, "agent", at - 35 * DAY);
        f.cost(&run, at - 35 * DAY + 60, DOLLAR + n * DOLLAR / 10);
    }
    f.cost("original", at - 60, dollars * DOLLAR);
    f
}

fn lead(evaluation: &Evaluation) -> Option<&Finding> {
    evaluation.detectors[0].findings.iter().find(|f| f.subject_id == "lead" && f.metric == "task_dollars")
}

fn raised(f: &Fixture, at: i64) -> Option<runaway::Raised> {
    lead(&evaluate(&f.store, PROJECT, at, &[Fixture::detector(Rule::CostOutlier.id())]).unwrap()).and_then(|f| f.raised.clone())
}

#[test]
fn ac213_ac245_a_fresh_runaway_marked_run_is_level_three_and_code_stops_only_it() {
    let at = ops::now();
    let f = project("runaway-stop", 30, at);
    let evaluation = evaluate(&f.store, PROJECT, at, &[Fixture::detector(Rule::CostOutlier.id())]).unwrap();
    let found = lead(&evaluation).expect("the run's $30 is past its type's p95");
    let raised = found.raised.clone().expect("fresh, decisive evidence names the marked run");
    assert_eq!((found.scores.impact, raised.score.as_str(), raised.rule.as_str(), raised.run.as_str()),
        (3, "impact", runaway::RULE, "original"));
    assert!(raised.from < 3 && (raised.run_dollars - 30.0).abs() < 1e-9 && (raised.week_dollars - 30.0).abs() < 1e-9);
    assert!(found.charter_never.is_none(), "a measured raise is never a charter breach");
    evaluation.record(&f.store).unwrap();
    let fingerprint = f.store.with(|tx| {
        let [signal] = signals::list(tx, PROJECT)?.into_iter().filter(|s| s.subject_id == "lead").collect::<Vec<_>>()
            .try_into().map_err(|_| "one lead signal")?;
        assert_eq!((signal.level, signal.response, signal.raised.as_ref().map(|r| r.run.as_str())), (3, "R5", Some("original")));
        let Some(Disposition::Applied(stopped)) = holds::automatic(tx, PROJECT, &signal.fingerprint, at)? else {
            panic!("code stops the named run without a round")
        };
        assert!(!stopped.shadow);
        assert_eq!(stopped.run.unwrap().state, runs::Phase::Left);
        Ok(signal.fingerprint)
    }).unwrap();

    // The owner's RUN resumes the task: the new run starts at its own $0, so the raise lapses.
    f.write("UPDATE tasks SET status = 'IN PROGRESS' WHERE id = 'lead'", []);
    f.run("lead", "resumed", "agent", at + 10);
    f.write("UPDATE runs SET state = 'working', pane_key = 'fixture-pane-2' WHERE id = 'resumed'", []);
    f.cost("resumed", at + 20, DOLLAR);
    let again = evaluate(&f.store, PROJECT, at + 30, &[Fixture::detector(Rule::CostOutlier.id())]).unwrap();
    let found = lead(&again).expect("the task's total still fires");
    assert!(found.raised.is_none() && found.scores.impact < 3, "the resumed run is no runaway");
    again.record(&f.store).unwrap();
    f.store.with(|tx| {
        assert!(signals::get(tx, PROJECT, &fingerprint)?.level < 3);
        assert!(holds::automatic(tx, PROJECT, &fingerprint, at + 30)?.is_none());
        // A level 3 the rule did not raise (a round's escalation) is no code stop either.
        tx.execute("UPDATE supervisor_signals SET scores = json_set(scores, '$.impact', 3) WHERE fingerprint = ?1",
            [&fingerprint]).map_err(sql)?;
        assert_eq!(signals::get(tx, PROJECT, &fingerprint)?.level, 3);
        assert!(holds::automatic(tx, PROJECT, &fingerprint, at + 30)?.is_none());
        Ok(())
    }).unwrap();
}

#[test]
fn ac213_ordinary_spending_drift_stays_below_level_three() {
    let at = ops::now();
    let f = project("runaway-drift", 30, at);
    assert!(raised(&f, at).is_some(), "the fixture's base case is a runaway");
    let undo = |statements: &[&str]| statements.iter().for_each(|s| f.write(s, []));

    undo(&["UPDATE usage SET at = at - 1000 WHERE run = 'original'"]);
    assert!(raised(&f, at).is_none(), "a run with no report for 15 minutes is not spending now");
    undo(&["UPDATE usage SET at = at + 1000 WHERE run = 'original'", "UPDATE usage SET output = output / 2 WHERE run = 'original'"]);
    assert!(raised(&f, at).is_none(), "$15 is under the $20 floor");
    undo(&["UPDATE usage SET output = output * 2 WHERE run = 'original'",
        "UPDATE usage SET output = output * 10 WHERE run GLOB 'old-*'"]);
    assert!(raised(&f, at).is_none(), "$30 is under three times a $29 p95");
    undo(&["UPDATE usage SET output = output / 10 WHERE run GLOB 'old-*'", "UPDATE tasks SET type = 'Feature' WHERE id = 'old-0'"]);
    assert!(raised(&f, at).is_none(), "19 earlier tasks are too few for a p95");
    undo(&["UPDATE tasks SET type = 'Bug' WHERE id = 'old-0'", "UPDATE runs SET state = 'closed' WHERE id = 'original'"]);
    assert!(raised(&f, at).is_none(), "a run that no longer works is no current run");
    undo(&["UPDATE runs SET state = 'working' WHERE id = 'original'"]);

    // The rest of the week's spend makes $30 under a tenth of it.
    f.task("busy", Some("Feature"), "P2", "DONE", at - 3 * DAY);
    f.run("busy", "busy-run", "agent", at - 3 * DAY);
    f.cost("busy-run", at - 3 * DAY + 60, 400 * DOLLAR);
    assert!(raised(&f, at).is_none(), "$30 of a $430 week");
    // An unpriced report makes the week's total a lower bound: no raise, and the detector says why.
    f.write("UPDATE usage SET output = 1, model = 'unknown' WHERE run = 'busy-run'", []);
    let evaluation = evaluate(&f.store, PROJECT, at, &[Fixture::detector(Rule::CostOutlier.id())]).unwrap();
    assert!(lead(&evaluation).unwrap().raised.is_none());
    assert!(evaluation.detectors[0].unavailable.contains(&runaway::UNPRICED_WEEK));
}

#[test]
fn ac213_ac215_replay_and_weekly_samples_never_raise() {
    let at = ops::now();
    let f = project("runaway-replay", 30, at);
    let detector = Fixture::detector(Rule::CostOutlier.id());
    f.store.snapshot(super::SCHEMA, |tx| {
        let mut past = measure_detector(tx, PROJECT, &detector, Window::of(at, detector.window_days), false)?;
        let current = measure_detector(tx, PROJECT, &detector, Window::of(at, detector.window_days), true)?;
        assert!(current.readings.iter().any(|r| r.raised.is_some()), "the present reading raises");
        assert!(past.readings.iter().all(|r| r.raised.is_none()), "a historical reading never does");
        assert!(super::collect_findings(&mut past, &detector, Window::of(at, detector.window_days), false)
            .iter().all(|f| f.raised.is_none()));
        Ok(())
    }).unwrap();
}
