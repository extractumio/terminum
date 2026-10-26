use serde_json::json;

use super::*;
use crate::notes::{New, Notes};
use crate::plans::Plans;

const K: &str = "tmux:local:terminum:%1";

fn plan(p: &Plans, now: u64) {
    p.hook(K, "claude", Some("s1"), &json!({"op": "add", "id": "1", "text": "alpha"}), now);
    p.hook(K, "claude", Some("s1"), &json!({"op": "add", "id": "2", "text": "beta"}), now);
    p.hook(K, "claude", Some("s1"), &json!({"op": "set", "id": "1", "status": "completed"}), now);
}

fn note(n: &Notes, s: &Scope, text: &str) -> u64 {
    n.add(s, "s1", vec![New { kind: "bug".into(), text: text.into(), file: Some("src/a.rs".into()) }], 100).unwrap()[0]
}

#[test]
fn row_gets_plan_and_open_notes_and_loses_them() {
    let p = Plans::default();
    plan(&p, 100);
    let mut it = json!({"id": K, "plan": {"old": 1}, "notes": 4});
    put(&mut it, p.row(K, false, 100), 2);
    assert_eq!(it["plan"], json!({"done": 1, "total": 2, "pct": 50, "eta": null, "updated": 100, "stale": false, "partial": false}));
    assert_eq!(it["notes"], 2);
    put(&mut it, p.row("tmux:local:terminum:%9", false, 100), 0);
    assert_eq!(it, json!({"id": K}));
}

#[test]
fn row_is_stale_only_while_working() {
    let p = Plans::default();
    plan(&p, 100);
    let late = 100 + 3600;
    let mut it = json!({"id": K});
    put(&mut it, p.row(K, false, late), 0);
    assert_eq!(it["plan"]["stale"], false);
    put(&mut it, p.row(K, true, late), 0);
    assert_eq!(it["plan"]["stale"], true);
}

#[test]
fn reply_without_plan_is_null_with_notes_open_first() {
    let n = Notes::load(None);
    let s = Scope { key: K.into(), project: None };
    let a = note(&n, &s, "first");
    let b = note(&n, &s, "second");
    n.user(b, "done", 200).unwrap();
    let r = reply(K, None, &n.list(&s));
    assert_eq!((r["t"].as_str(), r["key"].as_str()), (Some("plan"), Some(K)));
    assert!(r["plan"].is_null());
    let ids: Vec<u64> = r["notes"].as_array().unwrap().iter().map(|v| v["id"].as_u64().unwrap()).collect();
    assert_eq!(ids, [a, b]);
    assert_eq!(r["notes"][0], json!({"id": a, "kind": "bug", "text": "first", "file": "src/a.rs", "status": "open", "at": 100, "done_at": null}));
    assert_eq!((r["notes"][1]["status"].as_str(), r["notes"][1]["done_at"].as_u64()), (Some("done"), Some(200)));
}

#[test]
fn reply_with_plan_has_the_contract_fields() {
    let p = Plans::default();
    plan(&p, 100);
    let r = reply(K, p.details(K, false, 100), &[]);
    let pl = &r["plan"];
    assert_eq!(pl["items"], json!([{"text": "alpha", "status": "completed"}, {"text": "beta", "status": "pending"}]));
    assert_eq!((pl["done"].as_u64(), pl["total"].as_u64(), pl["pct"].as_u64()), (Some(1), Some(2), Some(50)));
    assert_eq!((pl["source"].as_str(), pl["agent"].as_str(), pl["partial"].as_bool()), (Some("tool"), Some("claude"), Some(false)));
    for k in ["title", "doing", "eta", "eta_given", "updated"] {
        assert!(pl.get(k).is_some(), "{k}");
    }
    assert_eq!(r["notes"], json!([]));
}

#[test]
fn note_acts_and_refusals() {
    let n = Notes::load(None);
    let s = Scope { key: K.into(), project: Some("p1".into()) };
    let id = note(&n, &s, "fix it");
    assert_eq!(n.scope(id), Some(s.clone()));
    n.user(id, "done", 150).unwrap();
    assert_eq!(n.open(&s), 0);
    n.user(id, "open", 160).unwrap();
    assert_eq!(n.open(&s), 1);
    assert!(n.user(id, "archive", 170).unwrap_err().contains("unknown action archive"));
    assert_eq!(n.scope(999), None);
    assert_eq!(n.user(999, "done", 170).unwrap_err(), "No note 999.");
    n.user(id, "delete", 180).unwrap();
    assert_eq!((n.open(&s), n.scope(id)), (0, None));
}

#[test]
fn a_gone_projects_notes_show_with_their_pane() {
    let n = Notes::load(None);
    let (kept, gone) = (Scope { key: K.into(), project: Some("p1".into()) }, Scope { key: "tmux:local:terminum:%2".into(), project: Some("p2".into()) });
    note(&n, &kept, "stays");
    let id = note(&n, &gone, "moves");
    assert!(!n.rescope(&["p1", "p2"]).unwrap());
    assert!(n.rescope(&["p1"]).unwrap());
    assert_eq!(n.scope(id), Some(Scope { key: gone.key.clone(), project: None }));
    assert_eq!((n.open(&kept), n.open(&Scope { key: gone.key, project: None })), (1, 1));
    assert_eq!(n.open(&Scope { key: "tmux:local:terminum:%9".into(), project: Some("p2".into()) }), 0, "not another member's");
    assert!(!n.rescope(&["p1"]).unwrap());
}

#[test]
fn plannew_reaches_every_page_once_per_key() {
    let pages = super::super::Pages::default();
    let dir = std::env::temp_dir().join(format!("tm-plans-{}", std::process::id()));
    let (_a, mut ra) = pages.add(false, crate::page::trace::Recorder::new(dir.clone())).unwrap();
    let (_b, mut rb) = pages.add(true, crate::page::trace::Recorder::new(dir)).unwrap();
    tell(&pages, &[K.into(), "tmux:local:terminum:%2".into()]);
    for rx in [&mut ra, &mut rb] {
        let got: Vec<Value> = std::iter::from_fn(|| rx.try_recv().ok()).map(|t| serde_json::from_str(&t).unwrap()).collect();
        assert_eq!(got, [json!({"t": "plannew", "key": K}), json!({"t": "plannew", "key": "tmux:local:terminum:%2"})]);
    }
    tell(&pages, &[]);
    assert!(ra.try_recv().is_err());
}
