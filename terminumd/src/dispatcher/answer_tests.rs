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

/// A task linked to session `key`, its agent's questions asked there by `conversation`: the task and the question's event.
fn asked_by(store: &Store, dir: &std::path::Path, key: &str, conversation: Option<&str>, extra: Value) -> (Task, String) {
    let who = Who { conversation: conversation.map(str::to_string), key: key.into(), agent: "claude".into(), title: "Agent".into(), project: None, dir: None, supervisor: false };
    let task = store.with(|tx| ops::create(tx, HUMAN, NewTask { title: "Choose the API".into(), session: Some(key.into()), ..Default::default() }, 1)).unwrap();
    let mut args = json!({"id": task.id, "questions": ASKED});
    if let (Some(a), Some(more)) = (args.as_object_mut(), extra.as_object()) {
        a.extend(more.clone());
    }
    tools::run(store, dir, &who, "ask_human", &args).unwrap();
    let question = store.with(|tx| Ok(choices::waiting(tx, &ops::get(tx, &task.id)?)?.unwrap())).unwrap();
    (task, question)
}

fn asked(store: &Store, dir: &std::path::Path, key: &str, extra: Value) -> (Task, String) {
    asked_by(store, dir, key, None, extra)
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
    let who = Who { conversation: None, key: key.into(), agent: "claude".into(), title: "Agent".into(), project: None, dir: None, supervisor: false };
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

    /// The pane's agent says by its hooks that it runs conversation `session`.
    fn hooked(&self, kind: &str, session: &str) {
        self.app.hooks.apply(&self.key(), &json!({"agent": kind, "event": "SessionStart", "session": session}), std::time::Instant::now());
    }

    /// A task linked to this pane, asked there by conversation "conv-1", which its hooks say runs there now.
    fn asked_here(&self, kind: &str, extra: Value) -> (Task, String) {
        let (task, question) = asked_by(&self.app.tasks, &self.root, &self.key(), Some("conv-1"), extra);
        self.hooked(kind, "conv-1");
        (task, question)
    }

    fn typed_nothing(&self) -> bool {
        std::fs::read_to_string(&self.log).unwrap_or_default().lines().all(|l| !l.starts_with("PROMPT:"))
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
        let (task, question) = stage.asked_here(kind, json!({}));
        let (status, said) = stage.press(&task.id.to_lowercase(), json!({"choice": "answer", "question": question, "text": "Use v2, keep v1 a month"})).await;
        assert_eq!(status, StatusCode::OK, "{kind}: {said}");
        assert_eq!(said["typed"], json!(stage.key()));
        let prompts = stage.prompts(1).await;
        assert_eq!(prompts, vec![Choice::Answer.typed(&task.id)], "{kind}: only the fixed words with the task's own ID");
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
    let (task, question) = stage.asked_here("claude", options);
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
    let (task, question) = stage.asked_here("claude", json!({}));
    let before = history(&stage.app.tasks, &task.id).len();
    let (status, said) = stage.press(&task.id, json!({"choice": "answer", "question": question, "text": "v2"})).await;
    assert_eq!(said["error"].as_str(), Some("The session is not at an empty input box: answer there first"), "{status} {said}");
    assert_eq!(history(&stage.app.tasks, &task.id).len(), before, "nothing kept of a press that did not go");
    assert_eq!(stage.app.tasks.with(|tx| ops::get(tx, &task.id)).unwrap().status, "WAIT FOR HUMAN");
    assert!(stage.typed_nothing(), "nothing typed");
}

#[tokio::test]
async fn ac285_another_conversation_in_the_pane_gets_nothing() {
    // a pane number tmux gave again after a restart: another agent, its own conversation
    let stage = Theirs::new("answer-other", "claude", "").await;
    let (task, question) = stage.asked_here("claude", json!({}));
    stage.hooked("claude", "conv-2");
    let (_, said) = stage.press(&task.id, json!({"choice": "answer", "question": question, "text": "v2"})).await;
    assert_eq!(said["error"].as_str(), Some(NOT_THE_ASKER), "{said}");
    // a question whose asker no hook said: refused too
    stage.app.tasks.with(|tx| tx.execute("UPDATE events SET data = json_remove(data, '$.asked_in') WHERE id = ?1", [&question]).map(drop).map_err(ops::sql)).unwrap();
    stage.hooked("claude", "conv-1");
    let (_, said) = stage.press(&task.id, json!({"choice": "recommended", "question": question})).await;
    assert_eq!(said["error"].as_str(), Some(NOT_THE_ASKER), "{said}");
    assert!(stage.typed_nothing());
    assert!(!history(&stage.app.tasks, &task.id).iter().any(|e| e["kind"] == "answer" || e["text"] == "v2"));
}

#[tokio::test]
async fn ac285_the_gate_types_only_into_an_idle_agent_of_a_pane_no_run_marked() {
    let stage = Theirs::new("answer-gate", "claude", "").await;
    let (server, pane, key) = (&stage.app.tmux.local, stage.pane.as_str(), stage.key());
    let typed = |since| gate::deliver_theirs(&stage.app, server, pane, None, "API-1", "API-1: words", since);
    let why = |r: Result<(), gate::Undelivered>| r.err().map(|u| (u.miss.to_string(), u.pasted));
    // words that are not plain
    assert_eq!(why(gate::deliver_theirs(&stage.app, server, pane, None, "API-1", "API-1\r: words", std::time::Instant::now()).await),
        Some(("the words have control or invisible characters".into(), false)));
    // no hook record: its box reads empty, yet nothing says its turn ended
    assert_eq!(why(typed(std::time::Instant::now()).await), Some(("the agent is working".into(), false)));
    stage.hooked("claude", "conv-1");
    // a human typed in it since the press
    let since = std::time::Instant::now();
    stage.app.inputs.human(&key);
    assert_eq!(why(typed(since).await), Some(("a human typed in its pane".into(), false)));
    // the agent at work
    stage.app.hooks.apply(&key, &json!({"agent": "claude", "event": "UserPromptSubmit", "session": "conv-1", "prompt": "work"}), std::time::Instant::now());
    assert_eq!(why(typed(std::time::Instant::now()).await), Some(("the agent is working".into(), false)));
    stage.app.hooks.apply(&key, &json!({"agent": "claude", "event": "Stop", "session": "conv-1"}), std::time::Instant::now());
    // a tmux mode
    stage.tm.read(&["copy-mode", "-t", pane]);
    assert_eq!(why(typed(std::time::Instant::now()).await), Some(("the pane is in a tmux mode".into(), false)));
    stage.tm.read(&["send-keys", "-t", pane, "-X", "cancel"]);
    // a run's mark: Dispatcher's pane, never typed by this path
    stage.tm.read(&["set-option", "-p", "-t", pane, "@terminum-run", "r1"]);
    assert_eq!(why(typed(std::time::Instant::now()).await), Some(("its pane is Dispatcher's".into(), false)));
    stage.tm.read(&["set-option", "-p", "-u", "-t", pane, "@terminum-run"]);
    // a shell in front
    let shell = stage.tm.read(&["new-window", "-d", "-P", "-F", "#{pane_id}", "/bin/sh"]).trim().to_string();
    stage.app.hooks.apply(&stage.app.tmux.local.key(&shell), &json!({"agent": "claude", "event": "SessionStart", "session": "conv-9"}), std::time::Instant::now());
    let missed = gate::deliver_theirs(&stage.app, server, &shell, None, "API-1", "API-1: words", std::time::Instant::now()).await;
    assert_eq!(why(missed), Some(("no coding agent runs in front: answer in its window".into(), false)));
    assert!(stage.typed_nothing());
    // all clear: it goes
    typed(std::time::Instant::now()).await.map_err(|u| u.miss.to_string()).unwrap();
    assert_eq!(stage.prompts(1).await, vec!["API-1: words".to_string()]);
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

#[tokio::test]
async fn ac285_ask_human_through_the_agent_socket_records_the_conversation_that_asks() {
    let stage = Theirs::new("answer-asks", "claude", "").await;
    stage.hooked("claude", "conv-7");
    let task = stage.app.tasks.with(|tx| ops::create(tx, HUMAN, NewTask { title: "Choose".into(), session: Some(stage.key()), ..Default::default() }, 1)).unwrap();
    let server = stage.app.tmux.local.clone();
    let said = crate::agentapi::agent_on(stage.app.clone(), Some(server), json!({"pane": stage.pane, "tool": "ask_human",
        "args": {"id": task.id, "questions": ASKED}}), None).await;
    assert_eq!(said.0["error"], json!(false), "{}", said.0);
    let question = stage.app.tasks.with(|tx| Ok(choices::waiting(tx, &ops::get(tx, &task.id)?)?.unwrap())).unwrap();
    assert_eq!(stage.app.tasks.with(|tx| choices::asked_in(tx, &question)).unwrap().as_deref(), Some("conv-7"));
    let (status, said) = stage.press(&task.id, json!({"choice": "answer", "question": question, "text": "v2"})).await;
    assert_eq!(status, StatusCode::OK, "{said}");
    assert_eq!(stage.prompts(1).await, vec![Choice::Answer.typed(&task.id)]);
}

#[tokio::test]
async fn ac285_a_session_the_human_started_by_plus_session_takes_the_words_under_its_own_mark() {
    let stage = Theirs::new("answer-plus", "claude", "").await;
    let (task, question) = stage.asked_here("claude", json!({}));
    // + Session's run: no task, started as a session (AC-142, AC-204); its mark on the pane
    let run = stage.app.tasks.with(|tx| runs::add(tx, &runs::Run { agent_id: "a".into(), project: task.project.clone(), ..Default::default() },
        &[], runs::Started::of(runs::How::Session), 1)).unwrap();
    stage.app.tasks.with(|tx| runs::change(tx, &run, |r| { r.state = runs::Phase::Working; r.pane_key = Some(stage.key()); }).map(drop)).unwrap();
    stage.tm.read(&["set-option", "-p", "-t", &stage.pane, "@terminum-run", &run]);
    let (status, said) = stage.press(&task.id, json!({"choice": "answer", "question": question, "text": "v2"})).await;
    assert_eq!(status, StatusCode::OK, "{said}");
    assert_eq!(stage.prompts(1).await, vec![Choice::Answer.typed(&task.id)]);
}

#[tokio::test]
async fn ac285_a_supervisor_round_s_pane_gets_nothing() {
    let stage = Theirs::new("answer-round", "claude", "").await;
    let (task, question) = stage.asked_here("claude", json!({}));
    let run = stage.app.tasks.with(|tx| runs::add(tx, &runs::Run { agent_id: "a".into(), project: task.project.clone(), ..Default::default() },
        &[], runs::Started::of(runs::How::Supervisor), 1)).unwrap();
    stage.app.tasks.with(|tx| runs::change(tx, &run, |r| { r.state = runs::Phase::Working; r.pane_key = Some(stage.key()); }).map(drop)).unwrap();
    stage.tm.read(&["set-option", "-p", "-t", &stage.pane, "@terminum-run", &run]);
    let (_, said) = stage.press(&task.id, json!({"choice": "answer", "question": question, "text": "v2"})).await;
    assert_eq!(said["error"].as_str(), Some(OTHER_RUN), "{said}");
    assert!(stage.typed_nothing());
}

#[tokio::test]
async fn ac285_recommended_as_run_keeps_its_choice_only_once_the_run_went() {
    let stage = Theirs::new("answer-going", "claude", "").await;
    let options = json!({"kind": "operational", "options": [{"id": "v1", "label": "v1"}, {"id": "v2", "label": "v2"}], "recommended": "v2"});
    let (task, question) = asked(&stage.app.tasks, &stage.root, &stage.app.tmux.local.key("%999"), options);
    let mark = || Some(answered(Choice::Recommended, &question));
    let chosen = |h: &[Value]| h.iter().any(|e| e["choice"] == json!({"question": question, "option": "v2"}));
    // a RUN refused, or one that only found the queue full: nothing kept
    assert!(going(&stage.app, &task.id, mark(), async { Err("refused".to_string()) }).await.is_err());
    going(&stage.app, &task.id, mark(), async { Ok(json!({"full": {"words": "no room"}})) }).await.unwrap();
    let h = history(&stage.app.tasks, &task.id);
    assert!(!chosen(&h) && !h.iter().any(|e| e["kind"] == "answer"), "nothing kept before it went");
    // it went: the option and the press's line, together
    going(&stage.app, &task.id, mark(), async { Ok(json!({"ok": true})) }).await.unwrap();
    let h = history(&stage.app.tasks, &task.id);
    assert!(chosen(&h), "the recommended option is the choice");
    assert_eq!(h.iter().find(|e| e["kind"] == "answer").unwrap()["answer"]["way"], "run");
}

#[tokio::test]
async fn ac285_the_gate_types_only_once_the_hooks_recorded_the_turn_s_end() {
    let stage = Theirs::new("answer-turn", "claude", "").await;
    let (server, pane, key) = (&stage.app.tmux.local, stage.pane.as_str(), stage.key());
    let typed = || gate::deliver_theirs(&stage.app, server, pane, None, "API-1", "API-1: words", std::time::Instant::now());
    let hook = |ev: Value| stage.app.hooks.apply(&key, &ev, std::time::Instant::now());
    hook(json!({"agent": "claude", "event": "SessionStart", "session": "conv-1"}));
    hook(json!({"agent": "claude", "event": "UserPromptSubmit", "session": "conv-1", "prompt": "work"}));
    // a permission asked, then granted in its window: the tool still runs, its box reads empty
    hook(json!({"agent": "claude", "event": "PermissionRequest", "session": "conv-1", "tool": "Bash"}));
    assert_eq!(typed().await.err().map(|u| u.miss.to_string()).as_deref(), Some("the agent is working"));
    // its turn ended asking the user: it waits for an answer, and takes it
    hook(json!({"agent": "claude", "event": "Stop", "session": "conv-1", "asks": true}));
    typed().await.map_err(|u| u.miss.to_string()).unwrap();
    assert_eq!(stage.prompts(1).await, vec!["API-1: words".to_string()]);
}

#[tokio::test]
async fn ac285_a_choice_refused_refuses_the_press_before_the_run() {
    let stage = Theirs::new("answer-refused", "claude", "").await;
    let options = json!({"kind": "operational", "options": [{"id": "v1", "label": "v1"}, {"id": "v2", "label": "v2"}], "recommended": "v2"});
    let (task, question) = asked(&stage.app.tasks, &stage.root, &stage.app.tmux.local.key("%999"), options);
    // the human already chose the other option by its key (AC-230)
    stage.app.tasks.with(|tx| choices::answer(tx, &task.id, &question, "v1", ops::now())).unwrap();
    let ran = std::sync::atomic::AtomicBool::new(false);
    let done = going(&stage.app, &task.id, Some(answered(Choice::Recommended, &question)), async {
        ran.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(json!({"ok": true}))
    }).await;
    assert_eq!(done.unwrap_err(), "An option was already chosen for this question.");
    assert!(!ran.load(std::sync::atomic::Ordering::SeqCst), "the RUN never went");
    assert!(!history(&stage.app.tasks, &task.id).iter().any(|e| e["kind"] == "answer"));
}
