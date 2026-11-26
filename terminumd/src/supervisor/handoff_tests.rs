// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213, AC-231, AC-233: a flow.stuck signal starts a round, and only that round hands its own task, nobody working
//! on it, to WAIT FOR HUMAN with its questions, the Supervisor's, recording the conversation its session holds.

use serde_json::{Value, json};

use super::super::engine_tests::{Fixture, PROJECT};
use super::super::samples::{STOPPED, UNWORKED};
use super::super::{DAY, Rule, rounds};
use super::{HAND_TO_HUMAN, run};

const KEY: &str = "tmux:local:terminum:%1";
const PANE: &str = "tmux:local:terminum:%90";
const QUESTIONS: &str = "Why you are needed: only the check on the iPad is left.\nRecommended:\n1. Check the page on the iPad: \
    the agent says the rest is done.\nDetails: the agent stopped with the task open after its last comment.";

struct Setup { f: Fixture, app: crate::app::Shared }

/// Task `stuck` IN PROGRESS in session KEY, its agent stopped two days ago, its flow.stuck signal recorded and a live
/// round of it marked in PANE.
fn setup(tag: &str, detector: &str) -> Setup {
    let (root, app) = crate::testutil::app_with_tasks(tag);
    let f = Fixture::in_path(root.join("state").join("kanban.sqlite"));
    let at = crate::tasks::ops::now();
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)", (PROJECT, at - 9 * DAY));
    f.task("stuck", Some("Feature"), "P2", "IN PROGRESS", at - 3 * DAY);
    f.write("UPDATE tasks SET session_key = ?1, slug = 'API-7' WHERE id = 'stuck'", [KEY]);
    f.event("stuck", at - 2 * DAY, "human", "comment", "Done but the check on the owner's iPad.", None);
    f.write("INSERT INTO supervisor_points (project, metric, subject, at, value) VALUES (?1, ?2, 'stuck', ?3, ?4)",
        (PROJECT, UNWORKED, at - 60, STOPPED));
    f.live(Rule::Stuck, at).record(&f.store).unwrap();
    let fingerprint: String = f.store.with(|tx| tx.query_row("SELECT fingerprint FROM supervisor_signals WHERE detector = 'flow.stuck'",
        [], |r| r.get(0)).map_err(crate::tasks::ops::sql)).unwrap();
    f.agent("small");
    f.run("stuck", "round-run", "small", at - 60);
    f.write("DELETE FROM run_tasks WHERE run = 'round-run'", []);
    f.write("UPDATE supervisor_signals SET detector = ?1 WHERE fingerprint = ?2", (detector, &fingerprint));
    let eligible = f.store.with(|tx| rounds::waiting(tx, at, None)).unwrap();
    assert_eq!(eligible.len(), usize::from(detector == "flow.stuck"), "a level 1 flow.stuck signal starts a round at once (AC-233)");
    f.write("INSERT INTO supervisor_rounds (id, run, project, generation, fingerprint, signal_revision, case_file, deadline)
        SELECT 'case_round-run', 'round-run', ?1, 1, ?2, revision, '{}', ?3 FROM supervisor_signals WHERE fingerprint = ?2",
        (PROJECT, &fingerprint, at + 900));
    f.write("INSERT INTO supervisor_panes (pane_key, project, generation, round) VALUES (?1, ?2, 1, 'case_round-run')", (PANE, PROJECT));
    *app.layout.write() = json!([{"items": [{"id": KEY, "state": "done", "agent": "claude"}]}]);
    Setup { f, app }
}

async fn hand(s: &Setup, args: Value) -> Result<String, String> {
    run(&s.app, PANE.into(), PROJECT.into(), HAND_TO_HUMAN.into(), args).await
}

fn status(s: &Setup) -> String {
    s.f.store.with(|tx| tx.query_row("SELECT status FROM tasks WHERE id = 'stuck'", [], |r| r.get(0)).map_err(crate::tasks::ops::sql)).unwrap()
}

#[tokio::test]
async fn ac213_a_stuck_round_hands_its_task_to_the_human_with_its_question() {
    let s = setup("handoff-happy", "flow.stuck");
    assert_eq!(hand(&s, json!({"questions": QUESTIONS})).await.unwrap(), "API-7 waits for the human, with your questions.");
    assert_eq!(status(&s), "WAIT FOR HUMAN");
    let (author, text, data): (String, String, String) = s.f.store.with(|tx| tx.query_row("SELECT author, text, data FROM events
        WHERE task = 'stuck' AND kind = 'comment' ORDER BY rowid DESC LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(crate::tasks::ops::sql)).unwrap();
    assert_eq!((author.as_str(), text.as_str()), ("supervisor:default", QUESTIONS), "the Supervisor asks, in its words");
    assert!(data.contains("\"questions\":true"), "{data}");
    assert!(hand(&s, json!({"questions": QUESTIONS})).await.unwrap_err().contains("it stays as it is"), "once");
}

#[tokio::test]
async fn ac231_only_its_own_stuck_signal_while_nobody_works_on_it() {
    let s = setup("handoff-other", "flow.starved");
    assert_eq!(hand(&s, json!({"questions": QUESTIONS})).await.unwrap_err(), "Only a flow.stuck investigation hands its own task to a human.");
    let s = setup("handoff-args", "flow.stuck");
    assert!(hand(&s, json!({"questions": QUESTIONS, "id": "other"})).await.unwrap_err().contains("not id"), "the task is the signal's");
    assert!(hand(&s, json!({"questions": "Is it done?"})).await.is_err(), "questions in the three parts");
    *s.app.layout.write() = json!([{"items": [{"id": KEY, "state": "working", "agent": "claude"}]}]);
    assert_eq!(hand(&s, json!({"questions": QUESTIONS})).await.unwrap_err(), "Someone works on API-7 now: it stays as it is.");
    assert_eq!(status(&s), "IN PROGRESS");
}

#[tokio::test]
async fn ac213_a_task_pressed_or_written_on_since_its_signal_stays_as_it_is() {
    let s = setup("handoff-changed", "flow.stuck");
    s.f.write("INSERT INTO run_waits (task, agent_id, project, status, body, at) VALUES ('stuck', 'small', ?1, 'IN PROGRESS', '{}', 1)", [PROJECT]);
    let refused = "Its task moved, was pressed, run or written on since its signal: it stays as it is.";
    assert_eq!(hand(&s, json!({"questions": QUESTIONS})).await.unwrap_err(), refused, "a human's RUN press waits for room");
    s.f.write("DELETE FROM run_waits", []);
    s.f.event("stuck", crate::tasks::ops::now(), "dispatcher", "told", "The agent stopped with this task open", None);
    assert_eq!(hand(&s, json!({"questions": QUESTIONS})).await.unwrap_err(), refused, "an event since its signal");
    assert_eq!(status(&s), "IN PROGRESS");
}

#[tokio::test]
async fn ac233_a_stuck_signal_gets_one_round_while_its_task_stays_stuck() {
    let s = setup("handoff-once", "flow.stuck");
    let at = crate::tasks::ops::now();
    s.f.write("UPDATE supervisor_rounds SET ended_at = ?1, failure = 'deadline' WHERE id = 'case_round-run'", [at]);
    s.f.write("DELETE FROM supervisor_panes", []);
    s.f.write("INSERT INTO supervisor_points (project, metric, subject, at, value) VALUES (?1, ?2, 'stuck', ?3, ?4)",
        (PROJECT, UNWORKED, at + 840, STOPPED));
    s.f.live(Rule::Stuck, at + 900).record(&s.f.store).unwrap();
    assert!(s.f.store.with(|tx| rounds::waiting(tx, at + 900, None)).unwrap().is_empty(),
        "a round that ended without a finding is not started again by the next sample");
}
