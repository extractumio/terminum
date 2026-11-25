// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-285: Answer, Continue with Recommended and Archive through their route, a private tmux server and a native
//! stand-in for the agent of a session the human opened.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use crate::dispatcher::screen::Screen;
use crate::tasks::closeout::ASKED;
use crate::tasks::ops::{NewTask, Task};
use crate::tasks::store::Store;
use crate::tasks::tools::{self, Who};
use crate::tmux::{args::quote, server::Link};

/// A task linked to session `key`, its agent's questions asked there: the task and the question's event.
fn asked(store: &Store, dir: &std::path::Path, key: &str, extra: Value) -> (Task, String) {
    let who = Who { key: key.into(), agent: "claude".into(), title: "Agent".into(), project: None, dir: None, supervisor: false };
    let task = store.with(|tx| ops::create(tx, HUMAN, NewTask { title: "Choose the API".into(), session: Some(key.into()), ..Default::default() }, 1)).unwrap();
    let mut args = json!({"id": task.id, "questions": ASKED});
    if let (Some(a), Some(more)) = (args.as_object_mut(), extra.as_object()) {
        a.extend(more.clone());
    }
    tools::run(store, dir, &who, "ask_human", &args).unwrap();
    let question = store.with(|tx| Ok(choices::waiting(tx, &ops::get(tx, &task.id)?)?.unwrap())).unwrap();
    (task, question)
}

fn history(store: &Store, id: &str) -> Vec<Value> {
    store.with(|tx| ops::history(tx, id)).unwrap().as_array().unwrap().clone()
}

fn answered(choice: Choice, question: &str) -> Answered {
    Answered { choice, question: question.into() }
}

#[test]
fn ac285_archive_fails_archives_and_keeps_what_was_chosen() {
    let dir = crate::testutil::fresh_dir("answer-archive");
    let store = Store::open(dir.join("kanban.sqlite"));
    let (task, question) = asked(&store, &dir, "tmux:local:terminum:%7", json!({}));
    store.with(|tx| archive(tx, &task.id, &answered(Choice::Archive, &question), "Not needed after all", &[])).unwrap();
    let t = store.with(|tx| ops::get(tx, &task.id)).unwrap();
    assert_eq!(t.status, "FAILED");
    assert!(t.archived_at.is_some());
    let h = history(&store, &task.id);
    assert!(h.iter().any(|e| e["kind"] == "comment" && e["text"] == "Not needed after all" && e["author_kind"] == "human"));
    let line = h.iter().find(|e| e["kind"] == "answer").expect("the press is kept");
    assert_eq!(line["answer"], json!({"choice": "archive", "question": question, "from": "WAIT FOR HUMAN", "way": "archive"}), "{line}");
    let moved: String = store.with(|tx| tx.query_row("SELECT data FROM events WHERE task = ?1 AND kind = 'move' ORDER BY rowid DESC LIMIT 1",
        [&task.key], |r| r.get(0)).map_err(ops::sql)).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&moved).unwrap(), json!({"from": "WAIT FOR HUMAN", "to": "FAILED", "archived": true}), "its move is kept");
    // a second press, as from another browser: refused, nothing more kept
    let again = store.with(|tx| archive(tx, &task.id, &answered(Choice::Archive, &question), "", &[]));
    assert_eq!(again.unwrap_err(), "That question is no longer waiting for an answer.");
    assert_eq!(history(&store, &task.id).len(), h.len());
}

#[test]
fn ac285_a_question_no_longer_waiting_is_refused() {
    let dir = crate::testutil::fresh_dir("answer-stale");
    let store = Store::open(dir.join("kanban.sqlite"));
    let key = "tmux:local:terminum:%7";
    let (task, first) = asked(&store, &dir, key, json!({}));
    // the agent asks again: the first question is answered by nothing
    let who = Who { key: key.into(), agent: "claude".into(), title: "Agent".into(), project: None, dir: None, supervisor: false };
    tools::run(&store, &dir, &who, "ask_human", &json!({"id": task.id, "questions": ASKED})).unwrap();
    store.with(|tx| {
        let t = ops::get(tx, &task.id)?;
        let second = choices::waiting(tx, &t)?.unwrap();
        assert_ne!(second, first);
        assert!(choices::still_waits(tx, &t, &first).is_err());
        choices::still_waits(tx, &t, &second)?;
        // once a key answered it, it waits no more, though the task may still wait for room
        keep(tx, &task.id, &answered(Choice::Answer, &second), "run")?;
        assert_eq!(choices::waiting(tx, &t)?, None);
        Ok(())
    }).unwrap();
}

/// A session the human opened: a private tmux, the stand-in for `kind` in front, unmarked, its box idle with `draft`.
struct Theirs {
    tm: crate::testutil::Tmux,
    app: Shared,
    pane: String,
    log: PathBuf,
    root: PathBuf,
}

impl Theirs {
    async fn new(name: &str, kind: &str, draft: &str) -> Self {
        let tm = crate::testutil::Tmux::start(name, (120, 30), "/bin/sh").expect("private tmux");
        let binary = tm.dir.join(kind);
        std::fs::copy(super::super::giveup::tests::native_agent(), &binary).unwrap();
        let log = tm.dir.join("events");
        let command = [binary.to_str().unwrap(), kind, log.to_str().unwrap(), "idle", draft].map(quote).join(" ");
        let pane = tm.read(&["new-window", "-d", "-P", "-F", "#{pane_id}", &command]).trim().to_string();
        let link = Link::Local { tmux: crate::config::find_program("tmux", "TM_TMUX").unwrap(), socket: tm.socket.clone(), conf: "/dev/null".into() };
        let (root, mut app) = crate::testutil::app_linked(name, link);
        Arc::get_mut(&mut app).unwrap().tasks = Arc::new(Store::open(root.join("kanban.sqlite")));
        let server = &app.tmux.local;
        server.sync_clients(&["main".into()]).unwrap();
        tokio::time::timeout(Duration::from_secs(6), async {
            while !server.connected() { tokio::time::sleep(Duration::from_millis(20)).await; }
            crate::lists::refresh_server(&app, server).await;
            while matches!(gate::read(server, &pane, kind).await, Ok(Screen::Unknown) | Err(_)) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.expect("native stand-in ready");
        Self { tm, app, pane, log, root }
    }

    fn key(&self) -> String {
        self.app.tmux.local.key(&self.pane)
    }

    async fn press(&self, id: &str, body: Value) -> (StatusCode, Value) {
        let api = crate::dispatcher::api::routes().with_state(self.app.clone());
        let response = api.oneshot(Request::post(format!("/api/tasks/{id}/answer"))
            .header("Content-Type", "application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = response.status();
        (status, serde_json::from_slice(&to_bytes(response.into_body(), 64_000).await.unwrap()).unwrap())
    }

    /// What the stand-in took as prompts, once `count` came.
    async fn prompts(&self, count: usize) -> Vec<String> {
        tokio::time::timeout(Duration::from_secs(6), async {
            loop {
                let said: Vec<String> = std::fs::read_to_string(&self.log).unwrap_or_default().lines()
                    .filter_map(|l| l.strip_prefix("PROMPT:")).map(str::to_string).collect();
                if said.len() >= count { return said; }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }).await.expect("the stand-in took the prompt")
    }
}

impl Drop for Theirs {
    fn drop(&mut self) {
        self.app.tmux.local.disconnect();
        let _ = &self.tm;
    }
}

#[tokio::test]
async fn ac285_answer_into_a_live_session_the_human_opened_types_fixed_words_only() {
    for kind in ["claude", "codex"] {
        let stage = Theirs::new(&format!("answer-typed-{kind}"), kind, "").await;
        let (task, question) = asked(&stage.app.tasks, &stage.root, &stage.key(), json!({}));
        let (status, said) = stage.press(&task.id, json!({"choice": "answer", "question": question, "text": "Use v2, keep v1 a month"})).await;
        assert_eq!(status, StatusCode::OK, "{kind}: {said}");
        assert_eq!(said["typed"], json!(stage.key()));
        let prompts = stage.prompts(1).await;
        assert_eq!(prompts, vec![Choice::Answer.typed(&task.id)], "{kind}: only the fixed words with the task's ID");
        assert!(!prompts[0].contains("Use v2"), "the human's text is never typed");
        let h = history(&stage.app.tasks, &task.id);
        assert!(h.iter().any(|e| e["kind"] == "comment" && e["text"] == "Use v2, keep v1 a month"), "the answer the agent reads by get_task");
        let line = h.iter().find(|e| e["kind"] == "answer").expect("the press is kept");
        assert_eq!(line["answer"], json!({"choice": "answer", "question": question, "from": "WAIT FOR HUMAN", "way": "typed"}));
        assert_eq!(stage.app.tasks.with(|tx| ops::get(tx, &task.id)).unwrap().status, "IN PROGRESS");
        // the same press again, as from another browser: refused
        let (status, said) = stage.press(&task.id, json!({"choice": "recommended", "question": question})).await;
        assert_eq!((status, said["error"].as_str()), (StatusCode::BAD_REQUEST, Some("That question is no longer waiting for an answer.")), "{said}");
    }
}

#[tokio::test]
async fn ac285_recommended_into_a_live_session_records_the_recommended_option() {
    let stage = Theirs::new("answer-recommended", "claude", "").await;
    let options = json!({"kind": "operational", "options": [{"id": "v1", "label": "v1"}, {"id": "v2", "label": "v2"}], "recommended": "v2"});
    let (task, question) = asked(&stage.app.tasks, &stage.root, &stage.key(), options);
    let (status, said) = stage.press(&task.id, json!({"choice": "recommended", "question": question, "text": "a draft that stays unsent"})).await;
    assert_eq!(status, StatusCode::OK, "{said}");
    assert_eq!(stage.prompts(1).await, vec![Choice::Recommended.typed(&task.id)]);
    let h = history(&stage.app.tasks, &task.id);
    assert!(h.iter().any(|e| e["choice"] == json!({"question": question, "option": "v2"})), "the recommended option is the choice");
    assert!(!h.iter().any(|e| e["text"] == "a draft that stays unsent"));
    assert_eq!(h.iter().find(|e| e["kind"] == "answer").unwrap()["answer"]["choice"], "recommended");
}

#[tokio::test]
async fn ac285_a_session_that_cannot_take_it_now_keeps_nothing() {
    let stage = Theirs::new("answer-draft", "claude", "my own draft").await;
    let (task, question) = asked(&stage.app.tasks, &stage.root, &stage.key(), json!({}));
    let before = history(&stage.app.tasks, &task.id).len();
    let (status, said) = stage.press(&task.id, json!({"choice": "answer", "question": question, "text": "v2"})).await;
    assert_eq!(said["error"].as_str(), Some("The session is not at an empty input box: answer there first"), "{status} {said}");
    assert_eq!(history(&stage.app.tasks, &task.id).len(), before, "nothing kept of a press that did not go");
    assert_eq!(stage.app.tasks.with(|tx| ops::get(tx, &task.id)).unwrap().status, "WAIT FOR HUMAN");
    assert!(std::fs::read_to_string(&stage.log).unwrap_or_default().lines().all(|l| !l.starts_with("PROMPT:")), "nothing typed");
    // a Dispatcher's mark on the pane: not the human's session, nothing typed there by this path
    stage.tm.read(&["set-option", "-p", "-t", &stage.pane, "@terminum-run", "r1"]);
    stage.tm.read(&["send-keys", "-t", &stage.pane, "C-u"]);
    let server = &stage.app.tmux.local;
    let miss = gate::deliver_theirs(&stage.app, server, &stage.pane, &task.id, "words", std::time::Instant::now()).await;
    assert_eq!(miss.unwrap_err().to_string(), "its pane is Dispatcher's");
}

#[tokio::test]
async fn ac285_elsewhere_the_answer_goes_on_as_run() {
    let stage = Theirs::new("answer-run", "claude", "").await;
    // its session closed: the page goes on with RUN, the answer its comment
    let (task, question) = asked(&stage.app.tasks, &stage.root, &stage.app.tmux.local.key("%999"), json!({}));
    let (status, said) = stage.press(&task.id, json!({"choice": "recommended", "question": question})).await;
    assert_eq!(status, StatusCode::OK, "{said}");
    assert_eq!(said, json!({"run": true, "comment": RECOMMENDED}));
    let (status, said) = stage.press(&task.id, json!({"choice": "answer", "question": question, "text": "  "})).await;
    assert_eq!((status, said["error"].as_str()), (StatusCode::BAD_REQUEST, Some("Write the answer first.")), "{said}");
    // RUN for a question no longer waiting is refused before anything starts, and keeps no line
    let api = crate::dispatcher::api::routes().with_state(stage.app.clone());
    let response = api.oneshot(Request::post(format!("/api/tasks/{}/run", task.id)).header("Content-Type", "application/json")
        .body(Body::from(json!({"comment": "v2", "answer": {"choice": "answer", "question": "gone"}}).to_string())).unwrap()).await.unwrap();
    assert_ne!(response.status(), StatusCode::OK);
    assert!(!history(&stage.app.tasks, &task.id).iter().any(|e| e["kind"] == "answer" || e["text"] == "v2"));
}
