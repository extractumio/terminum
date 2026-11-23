// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use std::{cell::Cell, path::PathBuf};

use rusqlite::{Connection, Params};
use serde_json::json;

use super::{Detector, Evidence, Mode, SCHEMA, WEEK, evaluate, signals};
use super::signals::State;
use crate::tasks::store::Store;

pub(super) const NOW: i64 = 1_800_000_000;
pub(super) const PROJECT: &str = "default";
pub(super) const OTHER: &str = "other";

pub(super) struct Fixture {
    pub store: Store,
    pub path: PathBuf,
    number: Cell<i64>,
}

impl Fixture {
    pub fn new(name: &str) -> Self {
        Self::in_path(crate::testutil::fresh_dir(&format!("supervisor-{name}")).join("kanban.sqlite"))
    }

    pub fn in_path(path: PathBuf) -> Self {
        let fixture = Self { store: Store::open(path.clone()), path, number: Cell::new(0) };
        fixture.write("INSERT INTO projects (id, name, color, dir, place) VALUES ('other', 'Other', 1, '', 1)", []);
        fixture
    }

    pub fn write(&self, statement: &str, args: impl Params) {
        self.store.with(|tx| tx.execute(statement, args).map(drop).map_err(crate::tasks::ops::sql)).unwrap();
    }

    fn number(&self) -> i64 {
        let n = self.number.get() + 1;
        self.number.set(n);
        n
    }

    pub fn task(&self, id: &str, kind: Option<&str>, priority: &str, status: &str, at: i64) {
        self.task_in(PROJECT, id, kind, priority, status, at);
    }

    pub fn task_in(&self, project: &str, id: &str, kind: Option<&str>, priority: &str, status: &str, at: i64) {
        self.write("INSERT INTO tasks (id, number, project, status, severity, rank, owner, created_at, updated_at, title, body, type)
            VALUES (?1, ?2, ?3, ?4, ?5, 0, 'human', ?6, ?6, '', '', ?7)",
            (id, self.number(), project, status, priority, at, kind));
        self.event(id, at, "human", "made", "", None);
    }

    pub fn event(&self, task: &str, at: i64, by: &str, kind: &str, text: &str, data: Option<serde_json::Value>) -> String {
        let id = format!("event-{}", self.number());
        self.write("INSERT INTO events (id, task, at, author, kind, text, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (&id, task, at, by, kind, text, data.map(|d| d.to_string())));
        id
    }

    pub fn transition(&self, task: &str, at: i64, from: &str, to: &str) -> String {
        self.write("UPDATE tasks SET status = ?2, updated_at = ?3 WHERE id = ?1", (task, to, at));
        self.event(task, at, "dispatcher", "move", "", Some(json!({"from": from, "to": to})))
    }

    pub fn agent(&self, agent: &str) {
        self.write("INSERT INTO agents (id, name, kind, command, color, project) VALUES (?1, ?1, 'codex', '', 0, 'default')", [agent]);
        self.write("INSERT INTO users (id, name, kind, role) VALUES ('agent:pool:' || ?1, ?1, 'agent', 'agent')", [agent]);
    }

    pub fn run(&self, task: &str, run: &str, agent: &str, at: i64) {
        self.write("INSERT INTO runs (id, task, project, agent_id, kind, command, folder, state, started_by, started_how)
            SELECT ?2, id, project, ?3, 'codex', '', '', 'closed', 'human', 'run' FROM tasks WHERE id = ?1", (task, run, agent));
        self.write("INSERT INTO run_tasks (run, task) VALUES (?1, ?2)", (run, task));
        self.write("INSERT INTO inputs (at, ended, sender, component, kind, text, bytes, pane, run, version)
            VALUES (?1, ?1, 'dispatcher', 'dispatcher.start', 'launch', '', 0, '', ?2, '')", (at * 1000, run));
    }

    pub fn failed_run(&self, task: &str, run: &str, at: i64) -> String {
        self.run(task, run, "agent", at);
        self.transition(task, at + 1, "IN PROGRESS", "FAILED")
    }

    pub fn cost(&self, run: &str, at: i64, output: i64) {
        self.write("INSERT INTO usage (run, agent_id, at, input, output, cache_write, cache_read, model)
            SELECT id, agent_id, ?2, 0, ?3, 0, 0, 'gpt-6-sol' FROM runs WHERE id = ?1", (run, at, output));
    }

    pub fn detector(id: &str) -> Detector {
        let mut detector = Detector::named(id).unwrap();
        detector.mode = Mode::Live;
        detector
    }
}

// AC-211: read through a separate snapshot, retain a stable fingerprint, and update the signal's evidence.
#[test]
fn ac211_signal_fingerprint_and_named_query() {
    let f = Fixture::new("signal");
    f.task("repeat", Some("Bug"), "P2", "IN PROGRESS", NOW - 200);
    let first = f.failed_run("repeat", "run-a", NOW - 50);
    let second = f.failed_run("repeat", "run-b", NOW - 20);
    let d = Fixture::detector("repeated_failures");
    let sweep = |at| evaluate(&f.store, PROJECT, at, std::slice::from_ref(&d)).unwrap();
    let initial = sweep(NOW);
    assert!(signals(&f.store, PROJECT).unwrap().is_empty());
    initial.record(&f.store).unwrap();
    sweep(NOW + 2).record(&f.store).unwrap();
    sweep(NOW + 4).record(&f.store).unwrap();
    assert_eq!(signals(&f.store, PROJECT).unwrap()[0].count, 1, "sweeps of a persisting condition are not repeats");
    let original = &signals(&f.store, PROJECT).unwrap()[0];
    assert_eq!(original.state, State::New);
    assert_eq!(original.level, 1);
    assert_eq!(original.response, "R2");
    assert_eq!(original.measure.value, 2.0);
    assert_eq!(original.measure.threshold, 2.0);
    assert!(original.evidence.contains(&Evidence::Events(first)));
    assert!(original.evidence.contains(&Evidence::Events(second)));
    assert!(signals(&f.store, OTHER).unwrap().is_empty());

    let third = f.failed_run("repeat", "run-c", NOW + 5);
    let repeated = sweep(NOW + 10);
    repeated.record(&f.store).unwrap();
    repeated.record(&f.store).unwrap();
    initial.record(&f.store).unwrap();
    let list = signals(&f.store, PROJECT).unwrap();
    assert_eq!(list.len(), 1);
    let changed = &list[0];
    assert_eq!(changed.fingerprint, original.fingerprint);
    assert_eq!((changed.first_seen, changed.last_seen, changed.count), (NOW, NOW + 10, 2));
    assert_eq!(changed.measure.value, 3.0);
    assert!(changed.evidence.contains(&Evidence::Events(third)));
    sweep(NOW + 20).record(&f.store).unwrap();
    assert_eq!(signals(&f.store, PROJECT).unwrap()[0].count, 2, "one new failed run is one repeat, however often swept");

    f.write("UPDATE supervisor_signals SET state = 'raised'", []);
    let later = NOW + 5 * WEEK;
    sweep(later).record(&f.store).unwrap();
    assert_eq!(signals(&f.store, PROJECT).unwrap()[0].state, State::Resolved);
    f.failed_run("repeat", "run-d", later + 1);
    f.failed_run("repeat", "run-e", later + 3);
    sweep(later + 10).record(&f.store).unwrap();
    let recurred = &signals(&f.store, PROJECT).unwrap()[0];
    assert_eq!(recurred.fingerprint, original.fingerprint);
    assert_eq!((recurred.state, recurred.count), (State::Recurred, 3));
    sweep(later + 20).record(&f.store).unwrap();
    assert_eq!(signals(&f.store, PROJECT).unwrap()[0].count, 3, "the return is one repeat");
}

// AC-211: a spend reading's new usage rows and a diagnostic's new measure are no repeat; a new event is.
#[test]
fn ac211_window_inputs_are_not_repeats() {
    let f = Fixture::new("signal-inputs");
    let old = NOW - 5 * WEEK;
    f.task("cheap", Some("Feature"), "P2", "IN PROGRESS", old - 1);
    f.run("cheap", "cheap-run", "agent", old);
    f.cost("cheap-run", old + 1, 10_000);
    f.task("expensive", Some("Feature"), "P2", "IN PROGRESS", NOW - 110);
    f.failed_run("expensive", "expensive-run", NOW - 90);
    f.cost("expensive-run", NOW - 89, 3_000_000);
    let d = Fixture::detector("flow.cost_outlier");
    let sweep = |at| evaluate(&f.store, PROJECT, at, std::slice::from_ref(&d)).unwrap().record(&f.store).unwrap();
    let counts = |metric: &str| signals(&f.store, PROJECT).unwrap().into_iter()
        .filter(|s| s.metric == metric).map(|s| (s.last_seen, s.count)).collect::<Vec<_>>();
    sweep(NOW);
    f.cost("expensive-run", NOW + 5, 3_000_000);
    sweep(NOW + 10);
    let spend = signals(&f.store, PROJECT).unwrap().into_iter().find(|s| s.metric == "task_dollars").unwrap();
    assert!(spend.evidence.len() >= 2, "the later usage row joined the evidence");
    assert_eq!(counts("task_dollars"), [(NOW + 10, 1)], "a growing spend total is one occurrence");

    let diagnose = |evidence: &[Evidence], used, at| f.store.with(|tx| signals::diagnostic(tx, PROJECT, "self.budget",
        ("round", "round-1", "judgment_round_tokens"), evidence, (used, 100.), at)).unwrap();
    diagnose(&[], 120., NOW);
    diagnose(&[], 150., NOW + 15);
    assert_eq!(counts("judgment_round_tokens"), [(NOW + 15, 1)], "rising use of one budget is one occurrence");
    diagnose(&[Evidence::Events("event-a".into())], 150., NOW + 30);
    assert_eq!(counts("judgment_round_tokens"), [(NOW + 30, 2)]);
}

// AC-211: validate the catalog before opening SQLite, reject mismatches, and refuse detector writes.
#[test]
fn ac211_validation_and_readonly_snapshot() {
    let f = Fixture::new("readonly");
    assert!(Detector::named("DELETE FROM tasks").unwrap_err().contains("Unknown"));
    let mut detector = Fixture::detector("open_waits");
    detector.window_days = 0;
    assert!(evaluate(&f.store, PROJECT, NOW, &[detector]).unwrap_err().contains("Invalid"));
    let d = Fixture::detector("flow.failed_repeat");
    assert!(evaluate(&f.store, PROJECT, NOW, &[d.clone(), d.clone()]).is_err());
    assert!(evaluate(&f.store, "missing", NOW, std::slice::from_ref(&d)).is_err());

    let held = f.store.with(|tx| {
        tx.execute("INSERT INTO meta VALUES ('uncommitted', 'value')", []).unwrap();
        let seen: bool = f.store.snapshot(SCHEMA, |read| {
            assert!(read.execute("DELETE FROM tasks", []).is_err());
            read.query_row("SELECT EXISTS(SELECT 1 FROM meta WHERE key = 'uncommitted')", [], |r| r.get(0)).map_err(crate::tasks::ops::sql)
        })?;
        Ok(seen)
    }).unwrap();
    assert!(!held, "reader never took or saw the write turn");
    Connection::open(&f.path).unwrap().pragma_update(None, "user_version", (SCHEMA + 1) as i64).unwrap();
    assert!(evaluate(&f.store, PROJECT, NOW, &[d]).unwrap_err().contains(&format!("schema is {}; Supervisor queries require {SCHEMA}", SCHEMA + 1)));
}

// AC-211: failing after an earlier rule wrote must also roll back deduplication state.
#[test]
fn ac211_record_rolls_back_and_retries() {
    let f = Fixture::new("rollback");
    f.task("bug", Some("Bug"), "P1", "TODO", NOW - 3 * super::DAY);
    f.failed_run("bug", "run-a", NOW - 40);
    f.failed_run("bug", "run-b", NOW - 22);
    f.transition("bug", NOW - 20, "FAILED", "TODO");
    let evaluation = evaluate(&f.store, PROJECT, NOW, &[Fixture::detector("flow.failed_repeat"), Fixture::detector("product.user_bug_age")]).unwrap();
    f.write("CREATE TRIGGER refuse BEFORE INSERT ON supervisor_signals WHEN new.detector='product.user_bug_age'
        BEGIN SELECT RAISE(ABORT, 'test refusal'); END", []);
    assert!(evaluation.record(&f.store).unwrap_err().contains("test refusal"));
    assert!(signals(&f.store, PROJECT).unwrap().is_empty());
    f.write("DROP TRIGGER refuse", []);
    evaluation.record(&f.store).unwrap();
    assert_eq!(signals(&f.store, PROJECT).unwrap().iter().map(|s| s.count).collect::<Vec<_>>(), [1, 1]);
}

// AC-212: disabled and missing rules do not make signals, and a later enable first writes shadow.
#[test]
fn ac212_off_missing_and_shadow() {
    let f = Fixture::new("shadow");
    f.task("bug", Some("Bug"), "P0", "TODO", NOW - 3 * super::DAY);
    let mut d = Detector::named("product.user_bug_age").unwrap();
    let off = evaluate(&f.store, PROJECT, NOW, &[d.clone(), Fixture::detector("flow.returned"), Fixture::detector("drift.loop")]).unwrap();
    assert_eq!(off.detectors[0].unavailable, ["Detector is off."]);
    assert_eq!(off.detectors[1].unavailable, ["Project integration is off."]);
    assert_eq!(off.detectors[2].unavailable, ["No retained hook counters for this period."]);
    off.record(&f.store).unwrap();
    assert!(signals(&f.store, PROJECT).unwrap().is_empty());
    d.enable();
    let shadow = evaluate(&f.store, PROJECT, NOW, &[d]).unwrap();
    shadow.record(&f.store).unwrap();
    assert_eq!(signals(&f.store, PROJECT).unwrap()[0].state, State::Shadow);
}

// AC-217: a persisting pass over a few hundred signals rewrites none of their content, and a counted signal with
// thousands of evidence rows is compared once per write, so neither holds the store's write turn.
#[test]
fn ac217_persisting_signals_write_only_what_changed() {
    let f = Fixture::new("persisting");
    // Fixture's task and failed_run, in one transaction: separate writes would take most of the test.
    f.store.with(|tx| {
        let run = |sql: &str, args: &[&dyn rusqlite::ToSql]| tx.execute(sql, args).map(drop).map_err(crate::tasks::ops::sql);
        for i in 0..300_i64 {
            let task = format!("task-{i}");
            run("INSERT INTO tasks (id, number, project, status, severity, rank, owner, created_at, updated_at, title, body, type)
                VALUES (?1, ?2, 'default', 'FAILED', 'P2', 0, 'human', ?3, ?3, '', '', 'Bug')", &[&task, &(i + 1), &(NOW - 900)])?;
            for (id, at) in [(format!("{task}-a"), NOW - 600), (format!("{task}-b"), NOW - 500)] {
                run("INSERT INTO runs (id, task, project, agent_id, kind, command, folder, state, started_by, started_how)
                    VALUES (?2, ?1, 'default', 'agent', 'codex', '', '', 'closed', 'human', 'run')", &[&task, &id])?;
                run("INSERT INTO run_tasks (run, task) VALUES (?1, ?2)", &[&id, &task])?;
                run("INSERT INTO inputs (at, ended, sender, component, kind, text, bytes, pane, run, version)
                    VALUES (?1, ?1, 'dispatcher', 'dispatcher.start', 'launch', '', 0, '', ?2, '')", &[&(at * 1000), &id])?;
                run("INSERT INTO events (id, task, at, author, kind, text, data) VALUES ('failed-' || ?2, ?1, ?3, 'dispatcher',
                    'move', '', '{\"from\":\"IN PROGRESS\",\"to\":\"FAILED\"}')", &[&task, &id, &(at + 1)])?;
            }
        }
        Ok(())
    }).unwrap();
    let d = Fixture::detector("repeated_failures");
    let pass = |at| evaluate(&f.store, PROJECT, at, std::slice::from_ref(&d)).unwrap().record(&f.store).unwrap();
    let mut big: Vec<_> = (0..4000).map(|i| Evidence::Events(format!("event-{i:032}"))).collect();
    let diagnose = |evidence: &[Evidence], at| f.store.with(|tx| signals::diagnostic(tx, PROJECT, "self.budget",
        ("round", "round-1", "judgment_round_tokens"), evidence, (150., 100.), at)).unwrap();
    let stored = || f.store.snapshot(SCHEMA, |tx| crate::tasks::ops::rows(tx, "SELECT subject_id, count, revision, evidence,
        last_seen FROM supervisor_signals ORDER BY subject_id", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?,
        r.get::<_, i64>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?)))).unwrap();
    pass(NOW);
    diagnose(&big, NOW);
    let first = stored();
    assert_eq!(first.len(), 301);

    pass(NOW + 10);
    diagnose(&big, NOW + 10);
    for (before, after) in first.iter().zip(&stored()) {
        assert_eq!((&before.0, before.1, before.2, &before.3), (&after.0, after.1, after.2, &after.3), "unchanged content");
        assert_eq!(after.4, if after.0 == "round-1" { NOW } else { NOW + 10 }, "a persisting finding moves last seen alone");
    }

    let task = "task-7";
    f.store.with(|tx| tx.execute_batch(&format!("INSERT INTO runs (id, task, project, agent_id, kind, command, folder, state,
        started_by, started_how) VALUES ('{task}-c', '{task}', 'default', 'agent', 'codex', '', '', 'closed', 'human', 'run');
        INSERT INTO run_tasks (run, task) VALUES ('{task}-c', '{task}');
        INSERT INTO inputs (at, ended, sender, component, kind, text, bytes, pane, run, version)
            VALUES ({at}, {at}, 'dispatcher', 'dispatcher.start', 'launch', '', 0, '', '{task}-c', '');
        INSERT INTO events (id, task, at, author, kind, text, data) VALUES ('failed-{task}-c', '{task}', {after}, 'dispatcher',
            'move', '', '{{\"from\":\"IN PROGRESS\",\"to\":\"FAILED\"}}')", at = (NOW + 11) * 1000, after = NOW + 12))
        .map_err(crate::tasks::ops::sql)).unwrap();
    pass(NOW + 20);
    big.push(Evidence::Events("event-new".into()));
    let started = std::time::Instant::now();
    diagnose(&big, NOW + 20);
    let held = started.elapsed();
    let counts: Vec<_> = stored().into_iter().filter(|s| s.1 != 1).map(|s| (s.0, s.1)).collect();
    assert_eq!(counts, [("round-1".to_string(), 2), (task.to_string(), 2)], "one new row is one repeat; the rest persist");
    assert!(held < std::time::Duration::from_secs(1), "a 4,000-row signal's write took {held:?}");
}
