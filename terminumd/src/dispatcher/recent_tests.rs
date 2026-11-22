// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-122: Save, Save, Run through the task routes, the typing gate and a private tmux pane.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::super::runs::{self, Phase, Run};
use crate::tasks::ops::{self, Task};
use crate::tasks::store::{DISPATCHER, HUMAN, Store};
use crate::tmux::args::quote;
use crate::tmux::server::Link;

async fn post(api: &Router, task: &Task, path: &str, given: Value) -> (StatusCode, Value) {
    let response = api.clone().oneshot(Request::post(format!("/api/tasks/{}/{path}", task.id))
        .header("Content-Type", "application/json")
        .body(Body::from(given.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64_000).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn prompts(log: &Path, count: usize) -> Vec<String> {
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            let lines: Vec<_> = std::fs::read_to_string(log).unwrap_or_default().lines().map(str::to_string).collect();
            if lines.len() >= count {
                return lines;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("agent took the prompt")
}

#[tokio::test]
async fn ac122_save_twice_then_run_sends_each_human_comment_once_in_order() {
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/fake_agent.py");
    let tm = crate::testutil::Tmux::start("run-saves", (120, 24), "/bin/sh").expect("private tmux");
    let log = tm.dir.join("prompts");
    let command = format!("exec python3 -I {} /usr/bin/true --prompts {}", quote(fake.to_str().unwrap()), quote(log.to_str().unwrap()));
    let pane = tm.read(&["new-window", "-d", "-P", "-F", "#{pane_id}", &command]).trim().to_string();
    let tmux = crate::config::find_program("tmux", "TM_TMUX").expect("tmux on PATH");
    let link = Link::Local { tmux, socket: tm.socket.clone(), conf: "/dev/null".into() };
    let (root, mut app) = crate::testutil::app_linked("run-saves", link);
    Arc::get_mut(&mut app).unwrap().tasks = Arc::new(Store::open(root.join("kanban.sqlite")));
    let server = &app.tmux.local;
    server.sync_clients(&["main".to_string()]).unwrap();
    let api = crate::tasks::api::routes().with_state(app.clone());
    let (task, run) = app.tasks.with(|tx| {
        let task = ops::create(tx, HUMAN, ops::NewTask { title: "Continue the task".into(), ..Default::default() }, 1)?;
        ops::comment(tx, HUMAN, &task.id, "Already part of the first run", 1)?;
        super::record(tx, &super::through(tx, std::slice::from_ref(&task))?)?;
        let like = Run { project: task.project.clone(), agent_id: "test-agent".into(), kind: "claude".into(), ..Default::default() };
        let id = runs::add(tx, &like, std::slice::from_ref(&task.key), Default::default(), 1)?;
        ops::link(tx, DISPATCHER, &task.id, Some(&server.key(&pane)), 1)?;
        Ok((task, id))
    }).unwrap();
    tm.read(&["set-option", "-p", "-t", &pane, "@terminum-run", &run]);
    tokio::time::timeout(Duration::from_secs(6), async {
        while !server.connected() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        crate::lists::refresh_server(&app, server).await;
        while super::super::gate::read(server, &pane, "claude").await != Ok(super::super::screen::Screen::Empty) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("stand-in agent's empty box");

    let ended = || app.tasks.with(|tx| {
        ops::move_to(tx, DISPATCHER, &task.id, "DONE", ops::now())?;
        runs::change(tx, &run, |r| { r.state = Phase::Working; r.pane_key = Some(server.key(&pane)); r.turn_over(); }).map(drop)
    }).unwrap();
    ended();
    assert_eq!(post(&api, &task, "comments", json!({"text": "Use the v2 API"})).await.0, StatusCode::OK);
    app.tasks.with(|tx| {
        let agent = ops::pool_user(tx, "test-agent", "Agent")?;
        ops::comment_as(tx, &agent, true, &task.id, "Manager words", ops::now())?;
        ops::comment(tx, DISPATCHER, &task.id, "Dispatcher words", ops::now()).map(drop)
    }).unwrap();
    assert_eq!(post(&api, &task, "comments", json!({"text": "Keep the old route"})).await.0, StatusCode::OK);
    let (status, value) = post(&api, &task, "run", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let first = &prompts(&log, 1).await[0];
    assert_eq!(first.matches("Use the v2 API").count(), 1, "{first}");
    assert_eq!(first.matches("Keep the old route").count(), 1, "{first}");
    assert!(first.find("Use the v2 API").unwrap() < first.find("Keep the old route").unwrap(), "{first}");
    for excluded in ["Already part", "Manager words", "Dispatcher words"] {
        assert!(!first.contains(excluded), "{first}");
    }

    ended();
    let (status, value) = post(&api, &task, "run", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let both = prompts(&log, 2).await;
    assert!(!both[1].contains("Use the v2 API") && !both[1].contains("Keep the old route"), "{both:?}");
    ended();
    assert_eq!(post(&api, &task, "comments", json!({"text": "A new saved comment"})).await.0, StatusCode::OK);
    tm.read(&["send-keys", "-l", "-t", &pane, "An open terminal draft"]);
    tokio::time::timeout(Duration::from_secs(6), async {
        while !matches!(super::super::gate::read(server, &pane, "claude").await, Ok(super::super::screen::Screen::Holds(_))) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("terminal draft visible");
    let (status, value) = post(&api, &task, "run", json!({"comment": "Keep this draft too"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
    assert_eq!(prompts(&log, 2).await.len(), 2, "refused Run submitted nothing");
    tm.read(&["send-keys", "-t", &pane, "C-u"]);
    tokio::time::timeout(Duration::from_secs(6), async {
        while super::super::gate::read(server, &pane, "claude").await != Ok(super::super::screen::Screen::Empty) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("terminal draft cleared");
    let (status, value) = post(&api, &task, "run", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let retry = &prompts(&log, 3).await[2];
    assert_eq!(retry.matches("A new saved comment").count(), 1, "{retry}");
    assert_eq!(retry.matches("Keep this draft too").count(), 1, "{retry}");
    assert!(retry.find("A new saved comment").unwrap() < retry.find("Keep this draft too").unwrap(), "{retry}");
    server.disconnect();
}


#[test]
fn ac122_queued_answer_keeps_its_draft_last_and_respects_the_prepared_cutoff() {
    let dir = crate::testutil::fresh_dir("saved-start");
    let store = Store::open(dir.join("kanban.sqlite"));
    store.with(|tx| {
        let task = ops::create(tx, HUMAN, ops::NewTask { title: "Continue from a queue".into(), ..Default::default() }, 10)?;
        let tasks = &[task];
        let task = &tasks[0];
        let agent = ops::pool_user(tx, "test-agent", "Agent")?;
        super::record(tx, &super::through(tx, tasks)?)?;
        ops::comment(tx, HUMAN, &task.id, "An early note", 10)?;
        ops::questions(tx, &agent, &task.id, "Which version?", 10)?;
        let draft = ops::comment(tx, HUMAN, &task.id, "The queued draft", 10)?;
        ops::comment(tx, HUMAN, &task.id, "An answer saved during the wait", 10)?;
        let drafts = super::legacy_drafts(tx, tasks, "The queued draft", 10)?;
        assert_eq!(drafts, [draft]);
        let saved = super::saved(tx, tasks, &drafts, true)?;
        let words = saved.words(&task.id, "Human", Some("The queued draft"))?;
        assert!(words.contains("answered your questions"), "{words}");
        assert!(words.find("An early note").unwrap() < words.find("An answer saved during the wait").unwrap(), "{words}");
        assert!(words.ends_with("The queued draft") && words.matches("The queued draft").count() == 1, "{words}");
        let cutoff = super::through(tx, tasks)?;
        let like = Run { project: task.project.clone(), agent_id: "test-agent".into(), kind: "claude".into(), ..Default::default() };
        let run = runs::add(tx, &like, std::slice::from_ref(&task.key), Default::default(), 10)?;
        super::stage(tx, &run, &cutoff)?;
        let before_delivery = super::saved(tx, tasks, &[], false)?;
        assert_eq!(before_delivery.comments.len(), 3, "the pending run has not received any comments");
        ops::comment(tx, HUMAN, &task.id, "Saved after preparation", 10)?;
        super::delivered(tx, &run)?;
        let next = super::saved(tx, tasks, &[], false)?;
        assert_eq!(next.comments, ["Saved after preparation"]);
        ops::comment(tx, HUMAN, &task.id, "Unsafe \u{001b} comment", 10)?;
        assert_eq!(super::saved(tx, tasks, &[], false).err().as_deref(), Some(super::BAD_COMMENT));
        Ok(())
    }).unwrap();
}

/// terminum-dispatcher.md AC-264: a RUN after a report says the human read it and carries their comments.
#[test]
fn ac264_a_run_after_a_report_says_the_human_read_it() {
    let store = Store::open(crate::testutil::fresh_dir("saved-report").join("kanban.sqlite"));
    store.with(|tx| {
        let task = ops::create(tx, HUMAN, ops::NewTask { title: "Audit the logs".into(), ..Default::default() }, 10)?;
        let tasks = &[task];
        super::record(tx, &super::through(tx, tasks)?)?;
        ops::move_to(tx, crate::tasks::store::DISPATCHER, &tasks[0].id, "IN PROGRESS", 10)?;
        crate::tasks::report::wait(tx, crate::tasks::store::DISPATCHER, &ops::get(tx, &tasks[0].id)?, 11)?;
        ops::comment(tx, HUMAN, &tasks[0].id, "Add the March numbers", 12)?;
        let words = super::saved(tx, tasks, &[], true)?.words(&tasks[0].id, "Human", None)?;
        assert_eq!(words, format!("{}: the human read your report and commented below. Go on with the task, taking it into account.\n\nAdd the March numbers", tasks[0].id));
        Ok(())
    }).unwrap();
}

fn codex_run(tm: &crate::testutil::Tmux, script: &Path, saved: &str) -> (crate::app::Shared, Router, Task) {
    tm.read(&["set-environment", "-g", "SHELL", "/bin/sh"]);
    tm.read(&["set-environment", "-g", "HOME", tm.dir.to_str().unwrap()]);
    let tmux = crate::config::find_program("tmux", "TM_TMUX").expect("tmux on PATH");
    let link = Link::Local { tmux, socket: tm.socket.clone(), conf: "/dev/null".into() };
    let (root, mut app) = crate::testutil::app_linked(&tm.socket, link);
    Arc::get_mut(&mut app).unwrap().tasks = Arc::new(Store::open(root.join("kanban.sqlite")));
    app.tmux.local.sync_clients(&["main".to_string()]).unwrap();
    let api = crate::tasks::api::routes().with_state(app.clone());
    let task = app.tasks.with(|tx| {
        let task = ops::create(tx, HUMAN, ops::NewTask { title: "Continue after answering".into(), ..Default::default() }, 1)?;
        let like = Run {
            project: task.project.clone(), agent_id: "test-agent".into(), kind: "codex".into(),
            command: format!("exec /bin/sh {}", quote(script.to_str().unwrap())),
            folder: tm.dir.display().to_string(), agent_session: Some(saved.into()), ..Default::default()
        };
        let id = runs::add(tx, &like, std::slice::from_ref(&task.key), Default::default(), 1)?;
        let agent = ops::pool_user(tx, "test-agent", "Agent")?;
        ops::questions(tx, &agent, &task.id, "Which route should stay?", 2)?;
        ops::move_to(tx, DISPATCHER, &task.id, "WAIT FOR HUMAN", 2)?;
        runs::change(tx, &id, |r| { r.state = Phase::Closed; r.asked = true; })?;
        Ok(task)
    }).unwrap();
    (app, api, task)
}

#[tokio::test]
async fn ac122_ac157_codex_run_reuses_the_conversation_after_hook_review() {
    use super::super::hooks;
    use crate::tmux::hookstate::Whose;

    let tm = crate::testutil::Tmux::start("codex-resume", (120, 24), "/bin/sh").expect("private tmux");
    let log = tm.dir.join("arguments");
    let script = tm.dir.join("codex.sh");
    std::fs::write(&script, format!(r#"printf '%s %s\n' "$1" "$2" >> {}
printf '› 1. Review hooks\n'
read -r reply
exit 1
"#, quote(log.to_str().unwrap()))).unwrap();
    let (app, api, task) = codex_run(&tm, &script, "saved-id");
    let hearing = tokio::spawn(hooks::hearing(app.clone()));

    let (status, value) = post(&api, &task, "run", json!({"comment": "Keep the old route"})).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let id = value["run"].as_str().unwrap();
    let read = || app.tasks.with(|tx| runs::by_id(tx, id)).unwrap();
    let key = read().pane_key.unwrap();
    let pane = key.rsplit(':').next().unwrap();
    let event = |name, prompt| hooks::heard(&app, &key, &json!({"agent": "codex", "event": name, "session": "unsaved-id", "prompt": prompt}), Whose::Main);
    event("SessionStart", "");
    tokio::time::timeout(Duration::from_secs(6), async {
        while read().state != Phase::Asking {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("Dispatcher saw hook review");
    assert_eq!(prompts(&log, 1).await, ["resume saved-id"]);
    event("SessionEnd", "");
    tokio::time::timeout(Duration::from_secs(6), app.dispatcher.soon.notified()).await.expect("Dispatcher drained through SessionEnd");
    assert_eq!(read().agent_session.as_deref(), Some("saved-id"));

    tm.read(&["send-keys", "-t", pane, "Enter"]);
    tokio::time::timeout(Duration::from_secs(6), async {
        while read().state != Phase::Gone {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("Dispatcher saw the exit");
    let (status, value) = post(&api, &task, "run", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(prompts(&log, 2).await, ["resume saved-id", "resume saved-id"]);
    let retried = app.tasks.with(|tx| runs::by_id(tx, value["run"].as_str().unwrap())).unwrap();
    let prompt = retried.prompt.unwrap();
    assert!(prompt.contains("the human answered your questions"), "{prompt}");
    assert_eq!(prompt.matches("Keep the old route").count(), 1, "{prompt}");
    event("UserPromptSubmit", &prompt);
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            let row = app.tasks.with(|tx| runs::by_id(tx, &retried.id)).unwrap();
            if row.state == Phase::Working {
                assert_eq!(row.agent_session.as_deref(), Some("unsaved-id"));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("Dispatcher accepted the prompt under the new ID");
    hearing.abort();
    let _ = hearing.await;
}

#[tokio::test]
async fn ac122_codex_run_reports_a_missing_conversation_on_the_task() {
    const MISSING: &str = "a1111111-2222-4333-8444-555555555555";
    let tm = crate::testutil::Tmux::start("codex-missing", (80, 24), "/bin/sh").expect("private tmux");
    let script = tm.dir.join("codex.sh");
    std::fs::write(&script, r#"printf 'ERROR: No saved session found with ID %s. Run `codex resume` without an ID to choose from existing sessions.\n' "$2"
exit 1
"#).unwrap();
    let (app, api, task) = codex_run(&tm, &script, MISSING);
    app.tasks.with(|tx| ops::move_to(tx, DISPATCHER, &task.id, "IN PROGRESS", 3).map(drop)).unwrap();
    let (status, value) = post(&api, &task, "run", json!({"comment": "My saved answer"})).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let id = value["run"].as_str().unwrap();
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            let run = app.tasks.with(|tx| runs::by_id(tx, id)).unwrap();
            if !run.is_starting() {
                assert_eq!(run.state, Phase::Closed);
                assert_eq!(run.agent_session.as_deref(), Some(MISSING));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("Dispatcher handled Codex's refusal");
    let old_pane = app.tasks.with(|tx| {
        let card = ops::get(tx, &task.id)?;
        assert_eq!(card.status, "WAIT FOR HUMAN");
        let pane = card.session.unwrap();
        assert_eq!(runs::by_id(tx, id)?.pane_key.as_deref(), Some(pane.as_str()));
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('resume-demo', 'Resume demo', 2, '', 1)", []).map_err(ops::sql)?;
        tx.execute("INSERT INTO project_members (project, pane_key, place) VALUES ('resume-demo', ?1, 0)", [&pane]).map_err(ops::sql)?;
        assert!(crate::roles::manages(tx, &pane)?.is_none());
        let history = ops::history(tx, &task.id)?;
        let texts: Vec<_> = history.as_array().unwrap().iter().filter_map(|event| event["text"].as_str()).collect();
        assert!(texts.contains(&"My saved answer"));
        assert!(texts.contains(&format!("Codex could not find saved conversation {MISSING}. Restore it and press RUN to try again, or choose Run from scratch.").as_str()));
        Ok(pane)
    }).unwrap();
    let (status, retry) = post(&api, &task, "run", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{retry}");
    app.tasks.with(|tx| {
        assert!(crate::roles::manages(tx, &old_pane)?.is_none());
        let retried = runs::by_id(tx, retry["run"].as_str().unwrap())?;
        assert_eq!(retried.agent_session.as_deref(), Some(MISSING));
        assert_ne!(retried.pane_key.as_deref(), Some(old_pane.as_str()));
        Ok(())
    }).unwrap();
}
