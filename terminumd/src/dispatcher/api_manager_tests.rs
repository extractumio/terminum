// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-179: a manager must explicitly reopen DONE; human RUN and FAILED keep their behavior.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;
use crate::tasks::store::{DISPATCHER, Store};
use crate::tmux::{args::quote, server::Link};

#[test]
fn ac179_rechecks_current_done_status_and_keeps_queued_reopen_consent() {
    Store::open(crate::testutil::fresh_dir("mgr-reopen-evidence").join("kanban.sqlite")).with(|tx| {
        let task = ops::create(tx, HUMAN, ops::NewTask { title: "Finished work".into(), ..Default::default() }, 1)?;
        let mut press = Press { manager: Some("manager".into()), ..Default::default() };
        press.manager_allows(tx, &task.id)?;
        // A snapshot taken before another manager finishes must not authorize its later admission.
        ops::move_to(tx, HUMAN, &task.id, "DONE", 2)?;
        let suffix = "To run it again on purpose, call run_task with reopen \"yes\".";
        assert_eq!(press.manager_allows(tx, &task.id).unwrap_err(), format!("{} is DONE. {suffix}", task.id));
        ops::comment(tx, HUMAN, &task.id, "## Outcome:\nDone, committed at abc12345.\nDone: fixed it.", 3)?;
        ops::comment(tx, HUMAN, &task.id, "A later unrelated note", 4)?;
        let why = press.manager_allows(tx, &task.id).unwrap_err();
        assert!(why.contains("Outcome: Done, committed at abc12345.") && !why.contains("unrelated") && !why.contains("fixed it"), "{why}");
        let landing = "Integrated into origin/main at acc9e390 in batch 2.";
        ops::left_queue(tx, DISPATCHER, &task.id, landing, "landed", 5)?;
        let at: String = tx.query_row("SELECT strftime('%H:%M', 5, 'unixepoch', 'localtime')", [], |r| r.get(0)).map_err(ops::sql)?;
        assert_eq!(press.manager_allows(tx, &task.id).unwrap_err(), format!("{} is DONE ({landing}; {at}). {suffix}", task.id));
        ops::comment(tx, HUMAN, &task.id, "**Outcome:** Done, checked again at def45678.", 5)?;
        assert!(press.manager_allows(tx, &task.id).unwrap_err().contains("def45678"), "newest result wins, including equal timestamps");
        for text in ["Outcome: Done, merged at abc12345.", "**Outcome:**\nDone, merged at abc12345.", "__Outcome:__ Done, merged at abc12345."] {
            ops::comment(tx, HUMAN, &task.id, text, 6)?;
            assert!(press.manager_allows(tx, &task.id).unwrap_err().contains("Outcome: Done, merged at abc12345."));
        }
        press.manager_reopen = true;
        let queued: Press = serde_json::from_value(serde_json::to_value(&press).unwrap()).unwrap();
        queued.manager_allows(tx, &task.id)?;
        let legacy: Press = serde_json::from_value(json!({"manager": "manager"})).unwrap();
        assert!(legacy.manager_allows(tx, &task.id).is_err(), "old kept presses carry no reopening consent");
        Press::default().manager_allows(tx, &task.id)?;
        Ok(())
    }).unwrap();
}

#[tokio::test]
async fn ac179_done_requires_explicit_reopen_then_runs_in_private_tmux() {
    let tm = crate::testutil::Tmux::start("mgr-reopen", (120, 24), "/bin/sh").expect("private tmux");
    let log = tm.dir.join("prompts");
    let fake = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/fake_agent.py");
    let command = format!("exec python3 -I {} /usr/bin/true --prompts {}", quote(fake.to_str().unwrap()), quote(log.to_str().unwrap()));
    let pane = tm.read(&["new-window", "-d", "-P", "-F", "#{pane_id}", &command]).trim().to_string();
    let link = Link::Local { tmux: crate::config::find_program("tmux", "TM_TMUX").unwrap(), socket: tm.socket.clone(), conf: "/dev/null".into() };
    let (root, mut app) = crate::testutil::app_linked("mgr-reopen", link);
    Arc::get_mut(&mut app).unwrap().tasks = Arc::new(Store::open(root.join("kanban.sqlite")));
    let server = &app.tmux.local;
    server.sync_clients(&["main".into()]).unwrap();
    let boss = "tmux:local:test:%manager";
    let task = app.tasks.with(|tx| {
        let task = ops::create(tx, HUMAN, ops::NewTask { title: "Finished work".into(), ..Default::default() }, 1)?;
        let like = Run { project: task.project.clone(), agent_id: "test-agent".into(), kind: "claude".into(), ..Default::default() };
        let manager = runs::add(tx, &like, &[], Default::default(), 1)?;
        runs::change(tx, &manager, |r| { r.state = Phase::Working; r.pane_key = Some(boss.into()); })?;
        let run = runs::add(tx, &like, std::slice::from_ref(&task.key), Default::default(), 1)?;
        ops::link(tx, DISPATCHER, &task.id, Some(&server.key(&pane)), 1)?;
        runs::change(tx, &run, |r| { r.state = Phase::Working; r.pane_key = Some(server.key(&pane)); r.turn_over(); })?;
        ops::move_to(tx, DISPATCHER, &task.id, "DONE", 2)?;
        ops::left_queue(tx, DISPATCHER, &task.id, "Integrated into origin/main at acc9e390 in batch 2.", "landed", 2)?;
        ops::get(tx, &task.id)
    }).unwrap();
    let run = app.tasks.with(|tx| Ok(runs::last_of(tx, &task.key)?.unwrap().id)).unwrap();
    tm.read(&["set-option", "-p", "-t", &pane, "@terminum-run", &run]);
    tokio::time::timeout(Duration::from_secs(6), async {
        while !server.connected() { tokio::time::sleep(Duration::from_millis(20)).await; }
        crate::lists::refresh_server(&app, server).await;
        while gate::read(server, &pane, "claude").await != Ok(Screen::Empty) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("stand-in agent's empty input");

    let before = app.tasks.with(|tx| ops::history(tx, &task.id)).unwrap();
    for options in [json!({}), json!({"reopen": "no"}), json!({"fresh": "yes", "queue": "yes"})] {
        let mut args = options;
        args["id"] = json!(task.id);
        args["comment"] = json!("Do it again");
        let why = crate::roles::in_pane(&app, boss, true, false, "run_task", &args).await.unwrap_err();
        assert!(why.starts_with(&format!("{} is DONE (", task.id)) && why.contains("acc9e390") && why.ends_with("call run_task with reopen \"yes\"."), "{why}");
        app.tasks.with(|tx| {
            assert_eq!(ops::get(tx, &task.id)?.status, "DONE");
            assert_eq!(ops::history(tx, &task.id)?, before, "refusal changes nothing");
            assert!(super::super::waits::of_task(tx, &task.key)?.is_none());
            assert_eq!(runs::last_of(tx, &task.key)?.unwrap().id, run);
            Ok(())
        }).unwrap();
        assert!(!log.exists(), "nothing was typed");
    }
    let args = json!({"id": task.id, "reopen": "yes"});
    assert!(crate::roles::in_pane(&app, boss, true, false, "run_task", &args).await.unwrap().contains("runs"));
    assert_eq!(app.tasks.with(|tx| Ok(ops::get(tx, &task.id)?.status)).unwrap(), "IN PROGRESS");
    tokio::time::timeout(Duration::from_secs(6), async {
        while !std::fs::read_to_string(&log).unwrap_or_default().contains("ran it again") {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("explicit reopen sends the prompt to the real private pane");

    for status in ["FAILED", "DONE"] {
        app.tasks.with(|tx| {
            ops::move_to(tx, DISPATCHER, &task.id, status, ops::now())?;
            runs::change(tx, &run, |r| r.turn_over()).map(drop)
        }).unwrap();
        if status == "FAILED" {
            crate::roles::in_pane(&app, boss, true, false, "run_task", &json!({"id": task.id})).await.unwrap();
        } else {
            let api = routes().with_state(app.clone());
            let response = api.oneshot(Request::post(format!("/api/tasks/{}/run", task.id)).header("Content-Type", "application/json").body(Body::from("{}")).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "a human's RUN reopens DONE");
        }
        assert_eq!(app.tasks.with(|tx| Ok(ops::get(tx, &task.id)?.status)).unwrap(), "IN PROGRESS");
    }
}
