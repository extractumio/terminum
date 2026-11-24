//! AC-138: Give up preserves human input across its read, cancellation and paste boundaries.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{tests::Stage, was_given_up, words};
use crate::dispatcher::{gate, letgo, runs::{self, Phase}, screen::Screen};
use crate::tasks::ops;

async fn owners(typing: &Arc<tokio::sync::Mutex<()>>, wanted: usize) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while Arc::strong_count(typing) < wanted {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("the request reached its typing queue");
}

async fn draft(stage: &Stage, text: &str) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if gate::read(&stage.app.tmux.local, &stage.pane, "claude").await == Ok(Screen::Holds(text.into())) { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("the human's draft remains");
}

#[tokio::test]
async fn ac138_human_input_during_foreground_read_prevents_escape() {
    let stage = Stage::new("giveup-human-read", "claude", "idle", "").await;
    let barrier = "giveup-read";
    stage.tm.read(&["wait-for", "-L", barrier]);
    let waiting = stage.app.tmux.local.bulk().unwrap().submit_all(vec![format!("wait-for -L {barrier}")]);
    let blocked = tokio::spawn(waiting);
    let typing = stage.app.tmux.local.typing(&stage.pane);
    let baseline = Arc::strong_count(&typing);
    let (app, run, pane) = (stage.app.clone(), stage.run.clone(), stage.pane.clone());
    let delivery = tokio::spawn(async move {
        gate::interrupt_and_deliver(&app, &run, &app.tmux.local, &pane, &words(&run.ids), Instant::now()).await
    });
    owners(&typing, baseline + 1).await;
    let (app, pane) = (stage.app.clone(), stage.pane.clone());
    let human = tokio::spawn(async move {
        let server = &app.tmux.local;
        let _held = app.inputs.human_queue(server, &pane, &server.key(&pane)).await;
        server.run(format!("send-keys -l -t {pane} 'human draft'")).await
    });
    owners(&typing, baseline + 2).await;
    stage.tm.read(&["wait-for", "-U", barrier]);
    assert!(blocked.await.unwrap().iter().all(Result::is_ok));
    assert!(delivery.await.unwrap().is_err());
    assert!(human.await.unwrap().is_ok());
    assert_eq!(stage.events(), "");
    draft(&stage, "human draft").await;
}

#[tokio::test]
async fn ac138_same_run_gaining_a_pane_refuses_without_cancelling() {
    let stage = Stage::new("giveup-pane-published", "claude", "idle", "").await;
    stage.app.tasks.with(|tx| runs::change(tx, &stage.run.id, |run| run.pane_key = None).map(drop)).unwrap();
    let (run, key) = (stage.run.id.clone(), stage.app.tmux.local.key(&stage.pane));
    let result = letgo::gave_up_checked(&stage.app, &stage.tasks[0].id, letgo::Giver::Human, move |tx, _| {
        runs::change(tx, &run, |run| run.pane_key = Some(key)).map(drop)
    }, |_, _| Ok(())).await;
    assert_eq!(result.unwrap_err(), "Its run changed while Give up waited. Read the task and try again.");
    stage.app.tasks.with(|tx| {
        assert_eq!(ops::get(tx, &stage.tasks[0].id)?.status, "IN PROGRESS");
        assert!(!was_given_up(tx, &stage.run.id)?);
        Ok(())
    }).unwrap();
    assert_eq!(stage.events(), "");
}

#[tokio::test]
async fn ac138_human_queued_between_cancellation_and_stop_keeps_its_new_turn() {
    let stage = Stage::new("giveup-human-handoff", "claude", "idle", "").await;
    let typing = stage.app.tmux.local.typing(&stage.pane);
    let held = typing.clone().lock_owned().await;
    let baseline = Arc::strong_count(&typing);
    let (app, id) = (stage.app.clone(), stage.tasks[0].id.clone());
    let cancellation = tokio::spawn(async move { letgo::gave_up(&app, &id).await });
    owners(&typing, baseline + 1).await;
    let (app, pane) = (stage.app.clone(), stage.pane.clone());
    let human = tokio::spawn(async move {
        let server = &app.tmux.local;
        let _held = app.inputs.human_queue(server, &pane, &server.key(&pane)).await;
        server.run(format!("send-keys -l -t {pane} 'human new turn'")).await?;
        server.run(format!("send-keys -t {pane} Enter")).await
    });
    owners(&typing, baseline + 2).await;
    drop(held);
    assert!(human.await.unwrap().is_ok());
    assert!(cancellation.await.unwrap().is_ok());
    stage.app.tasks.with(|tx| {
        assert_eq!(runs::by_id(tx, &stage.run.id)?.state, Phase::Left);
        assert_eq!(ops::get(tx, &stage.tasks[0].id)?.status, "FAILED");
        assert!(runs::holding_agent(tx, "a")?.is_none());
        assert!(was_given_up(tx, &stage.run.id)?);
        Ok(())
    }).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(stage.events(), "PROMPT:human new turn\n");
}

#[tokio::test]
async fn ac138_human_after_stop_paste_waits_for_its_enter() {
    let stage = Stage::new("giveup-human-after-paste", "claude", "idle", "").await;
    let barrier = "giveup-paste";
    stage.tm.read(&["wait-for", "-L", barrier]);
    stage.tm.read(&["set-hook", "-g", "after-paste-buffer", &format!("wait-for -L {barrier}")]);
    let (app, run, pane) = (stage.app.clone(), stage.run.clone(), stage.pane.clone());
    let delivery = tokio::spawn(async move {
        gate::interrupt_and_deliver(&app, &run, &app.tmux.local, &pane, &words(&run.ids), Instant::now()).await
    });
    let text = words(&stage.run.ids);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let shown = gate::read(&stage.app.tmux.local, &stage.pane, "claude").await.unwrap();
            if crate::dispatcher::screen::shows(&shown, &text) { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("the accepted stop paste is visible");
    let typing = stage.app.tmux.local.typing(&stage.pane);
    let baseline = Arc::strong_count(&typing);
    let (app, pane) = (stage.app.clone(), stage.pane.clone());
    let human = tokio::spawn(async move {
        let server = &app.tmux.local;
        let _held = app.inputs.human_queue(server, &pane, &server.key(&pane)).await;
        server.run(format!("send-keys -l -t {pane} 'human draft'")).await
    });
    owners(&typing, baseline + 1).await;
    stage.tm.read(&["set-hook", "-ug", "after-paste-buffer"]);
    stage.tm.read(&["wait-for", "-U", barrier]);
    assert_eq!(delivery.await.unwrap(), Ok(()));
    assert!(human.await.unwrap().is_ok());
    draft(&stage, "human draft").await;
    assert_eq!(stage.events(), format!("ESC\nPROMPT:{text}\n"));
}
