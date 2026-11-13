// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-02, AC-32: queued reads keep their subscription; every refusal keeps its request ref.
use std::sync::Arc;
use std::time::Duration;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use super::*;
use crate::page::{client::Shown, trace, Pages};
use crate::testutil::{self, Tmux};

fn show(page: &Page, server: &Arc<crate::tmux::Server>, pane: &str, ctl: Arc<Ctl>) {
    page.state.lock().shown = Some(Shown {
        key: server.key(pane), server: server.clone(), pane: pane.into(), ctl, task: tokio::spawn(async {}),
    });
}

async fn got(rx: &mut mpsc::Receiver<String>) -> Value {
    let text = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    serde_json::from_str(&text).unwrap()
}

#[tokio::test]
async fn queued_reads_never_retarget_a_new_subscription_even_to_the_same_pane() {
    let s = Tmux::start("read-target", (80, 10), "sh -c 'seq 1 100; exec cat'").expect("tmux required");
    let (root, app) = testutil::app("read-target");
    let server = crate::tmux::Server::new("local", &s.socket,
        crate::tmux::server::Link::Local { tmux: "tmux".into(), socket: s.socket.clone(), conf: "/dev/null".into() }, Default::default());
    server.sync_clients(&["main".into()]).unwrap();
    let pane = s.read(&["display", "-p", "#{pane_id}"]).trim().to_string();
    let (page, mut rx) = Pages::default().add(Default::default(), String::new(), trace::Recorder::new(root.join("trace"))).unwrap();
    let ctl = || {
        let ctl = Arc::new(Ctl::default());
        let h: i64 = s.read(&["display", "-p", "#{history_size}"]).trim().parse().unwrap();
        ctl.numbering.lock().step(h, 80, &[], false, 2000);
        *ctl.fields.lock() = Some(Fields { width: 80, limit: 2000, ..Default::default() });
        ctl
    };
    for kind in ["more", "unfold"] {
        show(&page, &server, &pane, ctl());
        let lane = format!("{kind} {}", server.id());
        let (hold, release) = oneshot::channel::<()>();
        let (started, ready) = oneshot::channel();
        assert!(crate::page::reads::spawn(&page, &lane, async move { started.send(()).unwrap(); let _ = release.await; }));
        ready.await.unwrap();
        queue(&app, &page, json!({"t": kind, "ref": "old", "before": 80, "n": 2, "to": 5}));
        // A -> B -> A is a new subscription, even when the pane key is the same again.
        show(&page, &server, "%999", ctl());
        show(&page, &server, &pane, ctl());
        let (done, ended) = oneshot::channel();
        assert!(crate::page::reads::spawn(&page, &lane, async move { done.send(()).unwrap(); }));
        hold.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), ended).await.unwrap().unwrap();
        assert!(rx.try_recv().is_err(), "the old read answered the new subscription");
    }
    queue(&app, &page, json!({"t": "more", "ref": "current", "before": 80}));
    let reply = got(&mut rx).await;
    assert_eq!(reply["t"], "hist");
    assert_eq!(reply["ref"], "current");
    assert!(!reply["lines"].as_array().unwrap().is_empty());
    queue(&app, &page, json!({"t": "unfold", "ref": "denied", "n": 2, "to": 5}));
    let reply = got(&mut rx).await;
    assert_eq!(reply["ref"], "denied");
    assert!(reply["error"].is_string());
    page.close(); drop(page); drop(server); drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn full_lane_and_disconnected_read_echo_the_ref() {
    let (root, app) = testutil::app("read-errors");
    let (page, mut rx) = Pages::default().add(Default::default(), String::new(), trace::Recorder::new(root.join("trace"))).unwrap();
    show(&page, &app.tmux.local, "%0", Arc::default());
    queue(&app, &page, json!({"t": "more", "ref": "offline"}));
    let reply = got(&mut rx).await;
    assert_eq!(reply["ref"], "offline");
    assert!(reply["msg"].as_str().unwrap().contains("not connected"));
    let lane = format!("more {}", app.tmux.local.id());
    let (_hold, release) = oneshot::channel::<()>();
    let (started, ready) = oneshot::channel();
    assert!(crate::page::reads::spawn(&page, &lane, async move { started.send(()).unwrap(); let _ = release.await; }));
    ready.await.unwrap();
    for i in 0..=crate::page::reads::WAITING { queue(&app, &page, json!({"t": "more", "ref": format!("queued-{i}")})); }
    assert_eq!(got(&mut rx).await["ref"], format!("queued-{}", crate::page::reads::WAITING));
    page.close(); drop(page); drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

/// AC-07, AC-33, AC-37, AC-88, AC-90: errors and idempotent delete acknowledgements keep their owner.
#[tokio::test]
async fn direct_action_failures_and_delete_acknowledgements_keep_the_ref() {
    let (root, app) = testutil::app("request-replies");
    let (page, mut rx) = Pages::default().add(Default::default(), String::new(), trace::Recorder::new(root.join("trace"))).unwrap();
    for t in ["askno", "sentget"] {
        crate::page::sent::handle(&app, &page, &json!({"t": t, "ref": t}), None).await;
        let reply = got(&mut rx).await;
        assert_eq!(reply["ref"], t);
        assert_eq!(reply["t"], "error");
    }
    crate::page::sessions::handle(&app, &page, &json!({"t": "newsession", "name": "bad:name", "ref": "session"}), None).await;
    assert_eq!(got(&mut rx).await["ref"], "session");
    crate::page::quick::handle(&app, &page, &json!({"t": "quick", "id": "missing", "ref": "quick"})).await;
    assert_eq!(got(&mut rx).await["ref"], "quick");
    crate::page::marks::handle(&app, &page, "unmark", &json!({"t": "unmark", "id": "already-gone", "ref": "delete"})).await;
    let reply = got(&mut rx).await;
    assert_eq!(reply["ref"], "delete");
    assert_eq!(reply["t"], "markdone", "an already deleted bookmark is acknowledged too");
    crate::page::marks::handle(&app, &page, "remind", &json!({"t": "remind", "action": "resolve", "id": "missing", "ref": "resolve"})).await;
    let reply = got(&mut rx).await;
    assert_eq!(reply["ref"], "resolve");
    assert_eq!(reply["t"], "error");
    page.close(); drop(page); drop(app);
    std::fs::remove_dir_all(root).unwrap();
}
