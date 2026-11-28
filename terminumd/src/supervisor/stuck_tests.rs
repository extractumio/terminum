// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213: `flow.stuck` finds an IN PROGRESS task linked to a session nobody works on, past its limit, and its
//! `stuck_work` raise brings any priority to level 1; work that is held, worked on or Dispatcher's stays quiet.

use serde_json::{Value, json};

use super::super::engine_tests::{Fixture, NOW, OTHER, PROJECT};
use super::super::signals::State;
use super::super::{DAY, Rule, signals};
use super::{GONE, STOPPED, UNWORKED, WORKED};

const KEY: &str = "tmux:local:terminum:%1";
const STOPPED_WORDS: &str = "Its agent stopped with it open.";
const GONE_WORDS: &str = "Its session closed, or no agent runs in it.";

fn project(name: &str) -> Fixture {
    let f = Fixture::new(name);
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)", (PROJECT, NOW - 9 * DAY));
    f
}

/// Task `id` of `priority` IN PROGRESS in session `KEY`, its agent stopped with it open `ago` seconds before NOW.
fn stopped(f: &Fixture, id: &str, priority: &str, ago: i64) {
    f.task(id, Some("Feature"), priority, "IN PROGRESS", NOW - 3 * DAY);
    f.write("UPDATE tasks SET session_key = ?2 WHERE id = ?1", (id, KEY));
    f.event(id, NOW - ago, "dispatcher", "told", "The agent stopped with this task open", None);
}

fn sample(f: &Fixture, id: &str, at: i64, value: f64) {
    f.write("INSERT INTO supervisor_points (project, metric, subject, at, value) VALUES (?1, ?2, ?3, ?4, ?5)",
        (PROJECT, UNWORKED, id, at, value));
}

/// `flow.stuck`'s findings at `at`: subject, impact and raises; and what it says is unavailable.
fn stuck(f: &Fixture, at: i64) -> (Vec<(String, u8, Value)>, Vec<&'static str>) {
    let evaluation = f.live(Rule::Stuck, at);
    let detector = &evaluation.detectors[0];
    (detector.findings.iter().map(|found| (found.subject_id.clone(), found.scores.impact,
        serde_json::to_value(&found.score_raises).unwrap())).collect(), detector.unavailable.clone())
}

fn raised(id: &str, words: &str) -> (String, u8, Value) {
    (id.into(), 1, json!([{"score": "impact", "from": 0, "to": 1, "rule": "stuck_work", "reason": words}]))
}

#[test]
fn ac213_a_stopped_agent_or_a_closed_session_reaches_the_owner_and_resolves() {
    let f = project("stuck-fires");
    stopped(&f, "stopped", "P2", 2 * DAY);
    sample(&f, "stopped", NOW - 60, STOPPED);
    stopped(&f, "gone", "P3", 6 * DAY);
    sample(&f, "gone", NOW - 60, GONE);
    assert_eq!(stuck(&f, NOW).0, [raised("gone", GONE_WORDS), raised("stopped", STOPPED_WORDS)]);
    f.live(Rule::Stuck, NOW).record(&f.store).unwrap();
    let all = signals(&f.store, PROJECT).unwrap();
    assert_eq!(all.iter().map(|s| (s.subject_id.as_str(), s.level)).collect::<Vec<_>>(), [("gone", 1), ("stopped", 1)]);
    let evidence = serde_json::to_value(&all[1]).unwrap()["evidence"].to_string();
    assert!(evidence.contains("events") && evidence.contains("tasks") && !evidence.contains("supervisor_points"),
        "the task and its last event; its latest sample, new every 15 minutes, is no evidence: {evidence}");
    let revision = || f.store.with(|tx| tx.query_row("SELECT revision FROM supervisor_signals WHERE subject_id = 'stopped'", [],
        |r| r.get::<_, i64>(0)).map_err(crate::tasks::ops::sql)).unwrap();
    let before = revision();
    sample(&f, "stopped", NOW + 840, STOPPED);
    f.live(Rule::Stuck, NOW + 900).record(&f.store).unwrap();
    assert_eq!(revision(), before, "a new sample of the same stuck task keeps its signal's evidence and revision");

    let later = NOW + 1800;
    sample(&f, "stopped", later - 60, WORKED);
    f.transition("gone", later - 30, "IN PROGRESS", "DONE");
    assert!(stuck(&f, later).0.is_empty(), "its agent works again; the other task moved");
    f.live(Rule::Stuck, later).record(&f.store).unwrap();
    assert!(signals(&f.store, PROJECT).unwrap().iter().all(|s| s.state == State::Resolved));
}

#[test]
fn ac213_stuck_work_ages_from_its_last_event_or_the_last_sample_that_saw_it_worked() {
    let f = project("stuck-age");
    stopped(&f, "fresh", "P2", 3600);
    sample(&f, "fresh", NOW - 60, STOPPED);
    stopped(&f, "worked", "P2", 2 * DAY);
    sample(&f, "worked", NOW - 2 * 3600, WORKED);
    sample(&f, "worked", NOW - 60, STOPPED);
    stopped(&f, "nudged", "P2", 2 * DAY);
    f.write("INSERT INTO users (id, name, kind, role) VALUES ('supervisor:default', 'Supervisor', 'agent', 'agent')", []);
    f.event("nudged", NOW - 600, "supervisor:default", "comment", "Supervisor: nobody works on this task.", None);
    sample(&f, "nudged", NOW - 60, STOPPED);
    stopped(&f, "priority", "P1", 2 * DAY);
    sample(&f, "priority", NOW - 60, STOPPED);
    let (found, _) = stuck(&f, NOW);
    assert_eq!(found.iter().map(|(id, _, _)| id.as_str()).collect::<Vec<_>>(), ["nudged", "priority"],
        "an hour since its last event or its last worked sample is under the limit; the Supervisor's comment is no activity");
    assert!(found[1].1 >= 1, "a P1 task keeps its share or the raise");
}

#[test]
fn ac213_held_dispatched_queued_or_other_work_is_not_stuck_and_an_ended_run_holds_nothing() {
    let f = project("stuck-quiet");
    f.agent("agent");
    stopped(&f, "run", "P2", 2 * DAY);
    f.run("run", "its-run", "agent", NOW - 3 * DAY);
    f.write("UPDATE runs SET pane_key = ?1, state = 'typed' WHERE id = 'its-run'", [KEY]);
    sample(&f, "run", NOW - 60, STOPPED);
    stopped(&f, "queued", "P2", 2 * DAY);
    f.write("INSERT INTO run_waits (task, agent_id, project, status, body, at) VALUES ('queued', 'agent', ?1, 'IN PROGRESS', '{}', ?2)",
        (PROJECT, NOW - DAY));
    sample(&f, "queued", NOW - 60, STOPPED);
    stopped(&f, "worked", "P2", 2 * DAY);
    sample(&f, "worked", NOW - 60, WORKED);
    f.task("unlinked", None, "P2", "IN PROGRESS", NOW - 3 * DAY);
    f.task_in(OTHER, "elsewhere", None, "P2", "IN PROGRESS", NOW - 3 * DAY);
    f.write("UPDATE tasks SET session_key = ?1 WHERE id = 'elsewhere'", [KEY]);
    assert_eq!(stuck(&f, NOW), (vec![], vec![]));
    f.write("UPDATE runs SET state = 'closed' WHERE id = 'its-run'", []);
    assert_eq!(stuck(&f, NOW).0, [raised("run", STOPPED_WORDS)], "taken again in the pane its ended run left");
}

#[test]
fn ac213_a_task_without_a_fresh_session_sample_is_unavailable_not_guessed() {
    let f = project("stuck-unsampled");
    stopped(&f, "never", "P2", 2 * DAY);
    stopped(&f, "stale", "P2", 2 * DAY);
    sample(&f, "stale", NOW - 1800, GONE);
    assert_eq!(stuck(&f, NOW), (vec![], vec!["Some IN PROGRESS tasks have no session sample in the last 30 minutes."]));
}

#[tokio::test]
async fn ac213_the_sampler_reads_each_linked_task_as_its_card_does() {
    let (_, app) = crate::testutil::app_with_tasks("stuck-sampler");
    let rows = [("working", json!({"state": "working", "agent": "claude"})), ("asking", json!({"state": "waiting", "agent": "claude"})),
        ("background", json!({"state": "done", "agent": "claude", "background": true})),
        ("subagents", json!({"state": "done", "agent": "claude", "subagents": 1})),
        ("idle", json!({"state": "done", "agent": "claude"})), ("failed", json!({"state": "failed", "agent": "codex"})),
        ("shell", json!({"state": null})), ("umbrella", json!({"state": "done", "agent": "claude"}))];
    let mut items: Vec<Value> = rows.iter().map(|(id, row)| {
        let mut row = row.clone();
        row["id"] = json!(format!("tmux:local:terminum:%{id}"));
        row
    }).collect();
    *app.layout.write() = json!([{"items": items.clone()}]);
    let tasks = ["working", "asking", "background", "subagents", "idle", "failed", "shell", "umbrella", "closed"];
    let sampled = |app: &crate::app::Shared| app.tasks.with(|tx| {
        Ok(super::unworked(tx, app, "api")?.into_iter().map(|p| (p.subject, p.value)).collect::<Vec<_>>())
    }).unwrap();
    app.tasks.with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('api', 'API', 1, '', 1)", []).map_err(crate::tasks::ops::sql)?;
        tx.execute("INSERT INTO project_members (project, pane_key, place) VALUES ('api', 'tmux:local:terminum:%umbrella', 0)", [])
            .map_err(crate::tasks::ops::sql)?;
        for (n, id) in tasks.iter().chain(["part", "unlinked"].iter()).enumerate() {
            let status = if *id == "part" { "TODO" } else { "IN PROGRESS" };
            let key = (*id != "unlinked").then(|| format!("tmux:local:terminum:%{id}"));
            tx.execute("INSERT INTO tasks (id, number, project, status, severity, rank, owner, created_at, updated_at, title, body, session_key)
                VALUES (?1, ?2, 'api', ?3, 'P2', 0, 'human', ?4, ?4, '', '', ?5)", (id, n as i64 + 1, status, NOW, key))
                .map_err(crate::tasks::ops::sql)?;
        }
        tx.execute("UPDATE tasks SET parent = 'umbrella' WHERE id = 'part'", []).map_err(crate::tasks::ops::sql).map(drop)
    }).unwrap();
    let expected = |umbrella: f64| -> Vec<(String, f64)> {
        let mut all: Vec<_> = [("working", WORKED), ("asking", WORKED), ("background", WORKED), ("subagents", WORKED),
            ("idle", STOPPED), ("failed", STOPPED), ("shell", GONE), ("umbrella", umbrella), ("closed", GONE)]
            .into_iter().map(|(id, v)| (id.to_string(), v)).collect();
        all.sort_by(|a, b| a.0.cmp(&b.0));
        all
    };
    let sorted = |mut v: Vec<(String, f64)>| { v.sort_by(|a, b| a.0.cmp(&b.0)); v };
    assert_eq!(sorted(sampled(&app)), expected(WORKED), "the umbrella is held in its open manager session");
    items.retain(|it| it["id"] != "tmux:local:terminum:%umbrella");
    *app.layout.write() = json!([{"items": items}]);
    assert_eq!(sorted(sampled(&app)), expected(GONE), "its manager session closed: nobody holds it");
}
