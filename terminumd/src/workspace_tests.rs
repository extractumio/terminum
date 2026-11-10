// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! workspace.rs: a failed write's outcome, its retry, and the shape a PUT must have (AC-71).

use std::fs;
use std::time::Duration;

use super::*;

async fn unsaved_event(rx: &mut tokio::sync::broadcast::Receiver<Event>) -> Option<String> {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.expect("an unsaved event").expect("open bus") {
            Event::Unsaved { error } => return error,
            _ => continue,
        }
    }
}

/// AC-71 failure: a write that fails is said to the panels (the event and the state frame),
/// tried again by itself, and said again once it worked.
#[tokio::test]
async fn ac71_a_failed_flush_is_said_and_retried_until_it_works() {
    let dir = crate::testutil::fresh_dir("ws-unsaved");
    let path = dir.join("workspaces.json");
    fs::create_dir(&path).unwrap();                 // a folder in its place: the rename onto it fails
    fs::write(path.join("in-the-way"), "").unwrap();
    let bus = Bus::new();
    let mut rx = bus.subscribe();
    let s = Store::load(path.clone(), 14, bus);
    s.set_root("p1", "/a");
    let error = unsaved_event(&mut rx).await.expect("the failure is said");
    assert!(!error.is_empty());
    assert_eq!(s.unsaved().as_deref(), Some(error.as_str()), "a panel that connects gets it too");
    fs::remove_dir_all(&path).unwrap();
    assert_eq!(unsaved_event(&mut rx).await, None, "the retry worked and says so");
    assert_eq!(s.unsaved(), None);
    let saved: File = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved.panes["p1"].root, "/a", "what was accepted is on disk");
    fs::remove_dir_all(dir).unwrap();
}

/// AC-71 failure: a PUT that is not a pane's state is refused with why, and changes nothing.
#[tokio::test]
async fn ac71_a_put_must_be_a_panes_state() {
    let dir = crate::testutil::fresh_dir("ws-shape");
    let s = Store::load(dir.join("workspaces.json"), 14, Bus::new());
    let tab = |view: &str| Tab { path: "/a/x.md".into(), view: view.into() };
    let bad = [
        Pane { root: "relative/dir".into(), ..Pane::default() },
        Pane { root: "/a".into(), tabs: vec![tab("hex")], ..Pane::default() },
        Pane { root: "/a".into(), tabs: vec![tab("source")], active_tab: Some(1), ..Pane::default() },
    ];
    for p in bad {
        assert!(matches!(s.put("p1", p.clone(), None), Err(Refused::Bad(_))), "{p:?}");
    }
    assert_eq!(s.get("p1", None).rev, 0, "nothing changed");
    let good = Pane { root: "/a".into(), tabs: vec![tab("rendered"), tab("auto")], active_tab: Some(1), ..Pane::default() };
    assert_eq!(s.put("p1", good, None).ok(), Some((1, true)));
    assert!(s.set_prefs(serde_json::json!([1, 2])).is_err(), "preferences are an object");
    assert!(s.set_prefs(serde_json::json!({"hidden": true})).is_ok());
    fs::remove_dir_all(dir).unwrap();
}
