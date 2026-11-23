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
}

// AC-211: a persisting condition only moves last seen; a new occurrence row and a return after resolving are repeats.
#[test]
fn ac211_count_is_repeats_not_sweeps() {
    let f = Fixture::new("signal-count");
    f.task("stuck", Some("Bug"), "P2", "IN PROGRESS", NOW - 200);
    f.failed_run("stuck", "run-a", NOW - 50);
    f.failed_run("stuck", "run-b", NOW - 20);
    let d = Fixture::detector("repeated_failures");
    let sweep = |at| evaluate(&f.store, PROJECT, at, std::slice::from_ref(&d)).unwrap().record(&f.store).unwrap();
    let seen = || signals(&f.store, PROJECT).unwrap().into_iter().map(|s| (s.state, s.last_seen, s.count)).collect::<Vec<_>>();
    for at in [NOW, NOW + 60, NOW + 120] { sweep(at); }
    assert_eq!(seen(), [(State::New, NOW + 120, 1)], "three sweeps of one condition are one occurrence");

    f.failed_run("stuck", "run-c", NOW + 130);
    sweep(NOW + 180);
    sweep(NOW + 240);
    assert_eq!(seen(), [(State::New, NOW + 240, 2)], "a new failed run is one repeat, however often it is swept");

    let later = NOW + 5 * WEEK;
    sweep(later);
    sweep(later + 60);
    assert_eq!(seen(), [(State::Resolved, NOW + 240, 2)]);
    f.failed_run("stuck", "run-d", later + 70);
    f.failed_run("stuck", "run-e", later + 80);
    sweep(later + 120);
    sweep(later + 180);
    assert_eq!(seen(), [(State::Recurred, later + 180, 3)], "the return is one repeat");
}

// AC-211: a retaken probe sample or snapshot point measures the same occurrence; an occurrence row joining is a repeat.
#[test]
fn ac211_retaken_samples_are_not_repeats() {
    let f = Fixture::new("signal-samples");
    let diagnose = |evidence: &[Evidence], at| f.store.with(|tx| signals::diagnostic(tx, PROJECT, "self.round_failed",
        ("round", "round-1", "failed_rounds"), evidence, (1., 0.), at)).unwrap();
    let seen = || signals(&f.store, PROJECT).unwrap().into_iter().map(|s| (s.last_seen, s.count)).collect::<Vec<_>>();
    diagnose(&[Evidence::SupervisorProbes(1)], NOW);
    diagnose(&[Evidence::SupervisorProbes(2)], NOW + 15);
    diagnose(&[Evidence::SupervisorPoints(3)], NOW + 30);
    assert_eq!(seen(), [(NOW + 30, 1)]);
    diagnose(&[Evidence::SupervisorProbes(4), Evidence::Events("event-a".into())], NOW + 45);
    diagnose(&[Evidence::SupervisorProbes(5), Evidence::Events("event-a".into())], NOW + 60);
    assert_eq!(seen(), [(NOW + 60, 2)]);
    diagnose(&[Evidence::Events("event-a".into()), Evidence::Events("event-b".into())], NOW + 75);
    assert_eq!(seen(), [(NOW + 75, 3)]);
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
