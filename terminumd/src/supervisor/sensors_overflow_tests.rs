// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use serde_json::json;
use super::*;
use crate::supervisor::{engine_tests::{Fixture, NOW, OTHER, PROJECT}, evaluate, Rule};

fn setup(name: &str) -> (Fixture, Sensors) {
    let f = Fixture::new(name);
    let sensors = Sensors::default();
    for (id, pane) in [(PROJECT, "p"), (OTHER, "other-pane")] {
        f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1,1,1,?2)", (id, NOW - 100));
        f.write("INSERT INTO project_members VALUES (?1, ?2, 0)", (id, pane));
        sensors.set(id, 1, true);
        sensors.heard(id, pane, &event("first", "PreCompact"), NOW - 5);
    }
    (f, sensors)
}

fn event(session: &str, name: &str) -> Value {
    json!({"agent":"claude", "event":name, "session":session})
}

fn flush(f: &Fixture, sensors: &Sensors, project: &str, generation: i64, at: i64) {
    let pending = sensors.take(project, generation);
    f.store.with(|tx| pending.write(tx, at, pending.encode()?)).unwrap();
    pending.committed();
}

fn incomplete(f: &Fixture, project: &str, rule: Rule, at: i64) -> bool {
    let result = evaluate(&f.store, project, at, &[Fixture::detector(rule.id())]).unwrap();
    result.detectors[0].unavailable.iter().any(|why| why.contains("overflow"))
}

#[test]
fn ac218_session_overflow_marks_only_the_affected_project_detector_and_window() {
    let (f, sensors) = setup("hook-session-overflow");
    for index in 1..MAX_SESSIONS {
        sensors.heard(PROJECT, "p", &event(&index.to_string(), "Stop"), NOW - 5);
    }
    sensors.heard(PROJECT, "p", &event("extra", "SubagentStart"), NOW);
    sensors.heard(PROJECT, "p", &event("extra", "PreCompact"), NOW + 1);
    flush(&f, &sensors, PROJECT, 1, NOW + 30);
    flush(&f, &sensors, OTHER, 1, NOW + 30);
    assert!(incomplete(&f, PROJECT, Rule::Subagents, NOW), "a dropped subagent start makes the event's window incomplete");
    assert!(!incomplete(&f, PROJECT, Rule::Loop, NOW));
    assert!(incomplete(&f, PROJECT, Rule::Loop, NOW + 1));
    let lost = f.store.with(|tx| tx.query_row(
        "SELECT sum(count) FROM supervisor_hook_overflow WHERE project = ?1 AND reason = 'sessions'",
        [PROJECT], |r| r.get::<_, i64>(0)).map_err(sql)).unwrap();
    assert_eq!(lost, 2, "each event contributes one loss to its project");
    assert!(!incomplete(&f, OTHER, Rule::Subagents, NOW + 30));
    assert!(!incomplete(&f, PROJECT, Rule::Subagents, NOW - 1));
    let days = Fixture::detector(Rule::Subagents.id()).window_days;
    assert!(!incomplete(&f, PROJECT, Rule::Subagents, NOW + i64::from(days) * crate::supervisor::DAY + 1));
}

fn tool(thread: usize) -> Value {
    json!({"agent":"codex","session":"one","event":"PreToolUse",
        "sub":format!("worker-{thread}"),"tool_call_hash":"abcdef1234567890"})
}

#[test]
fn ac218_thread_overflow_preserves_tools_and_retries_only_rolled_back_counts() {
    let (f, sensors) = setup("hook-thread-overflow");
    for thread in 0..=MAX_THREADS {
        sensors.heard(PROJECT, "p", &tool(thread), NOW);
    }
    let failed = sensors.take(PROJECT, 1);
    assert!(failed.overflow.affects(Rule::Loop, Window::week(NOW)));
    assert!(!failed.overflow.affects(Rule::Subagents, Window::week(NOW)));
    let result: Result<(), String> = f.store.with(|tx| {
        failed.write(tx, NOW, failed.encode()?)?;
        Err("fixture rollback".into())
    });
    assert_eq!(result.unwrap_err(), "fixture rollback");
    sensors.heard(PROJECT, "p", &tool(MAX_THREADS), NOW + 1);
    drop(failed);
    flush(&f, &sensors, PROJECT, 1, NOW + 2);
    flush(&f, &sensors, PROJECT, 1, NOW + 3);
    assert!(incomplete(&f, PROJECT, Rule::Loop, NOW));
    assert!(!incomplete(&f, PROJECT, Rule::Subagents, NOW));
    f.store.with(|tx| {
        assert_eq!(tx.query_row("SELECT count, first_at, last_at FROM supervisor_hook_overflow
            WHERE project = ?1 AND reason = 'threads'", [PROJECT],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))).map_err(sql)?, (2, NOW, NOW + 1));
        assert_eq!(tx.query_row("SELECT sum(count) FROM supervisor_hook_counts
            WHERE project = ?1 AND session = 'one' AND event = 'tool'", [PROJECT], |r| r.get::<_, i64>(0)).map_err(sql)?,
            MAX_THREADS as i64 + 2);
        Ok(())
    }).unwrap();

    sensors.heard(PROJECT, "p", &tool(MAX_THREADS), NOW + 3);
    let stale = sensors.take(PROJECT, 1);
    sensors.set(PROJECT, 2, true);
    drop(stale);
    let fresh = sensors.take(PROJECT, 2);
    assert!(!fresh.overflow.affects(Rule::Loop, Window::week(NOW + 3)));
}
