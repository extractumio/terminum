use super::*;
use crate::testutil::fresh_dir;

fn pane(key: &str) -> Scope {
    Scope { key: key.into(), project: None }
}

fn project(key: &str, p: &str) -> Scope {
    Scope { key: key.into(), project: Some(p.into()) }
}

fn new(kind: &str, text: &str) -> New {
    New { kind: kind.into(), text: text.into(), file: None }
}

#[test]
fn adds_lists_open_first_newest_first() {
    let n = Notes::load(None);
    let s = pane("tmux:local:t:%1");
    assert_eq!(n.add(&s, "s1", vec![new("bug", "a"), new("question", "b")], 10).unwrap(), vec![1, 2]);
    assert_eq!(n.add(&s, "s1", vec![new("followup", "c")], 11).unwrap(), vec![3]);
    n.update(&s, 3, Some("done"), None, None, 12).unwrap();
    let ids: Vec<u64> = n.list(&s).iter().map(|x| x.id).collect();
    assert_eq!(ids, vec![2, 1, 3]);
    let done = n.list(&s).into_iter().find(|x| x.id == 3).unwrap();
    assert_eq!((done.status.as_str(), done.done_at), ("done", Some(12)));
}

#[test]
fn refuses_bad_kind_long_text_empty_and_too_many() {
    let n = Notes::load(None);
    let s = pane("k");
    assert!(n.add(&s, "", vec![new("idea", "x")], 1).unwrap_err().contains("bug, deferred, followup, question"));
    assert!(n.add(&s, "", vec![new("bug", &"x".repeat(1001))], 1).unwrap_err().contains("over 1000"));
    assert!(n.add(&s, "", vec![new("bug", " \u{7} ")], 1).unwrap_err().contains("empty"));
    let file = New { kind: "bug".into(), text: "x".into(), file: Some("f".repeat(301)) };
    assert!(n.add(&s, "", vec![file], 1).unwrap_err().contains("over 300"));
    assert!(n.add(&s, "", (0..21).map(|_| new("bug", "x")).collect(), 1).is_err());
    assert!(n.add(&s, "", vec![], 1).is_err());
    assert!(n.list(&s).is_empty());
}

#[test]
fn drops_control_characters_and_secrets() {
    let n = Notes::load(None);
    let s = pane("k");
    n.add(&s, "", vec![new("bug", "a\u{1b}[31mb\nc API_TOKEN=abc123secret")], 1).unwrap();
    let t = &n.list(&s)[0].text;
    assert!(t.starts_with("a[31mb\nc"), "{t}");
    assert!(!t.contains("abc123secret"), "{t}");
}

#[test]
fn caps_open_notes_per_scope() {
    let n = Notes::load(None);
    let s = pane("k");
    for _ in 0..5 {
        n.add(&s, "", (0..20).map(|_| new("bug", "x")).collect(), 1).unwrap();
    }
    assert!(n.add(&s, "", vec![new("bug", "x")], 1).unwrap_err().contains("at most 100"));
    assert!(n.add(&pane("other"), "", vec![new("bug", "x")], 1).is_ok(), "another scope has its own cap");
    n.update(&s, 1, Some("done"), None, None, 2).unwrap();
    assert!(n.add(&s, "", vec![new("bug", "x")], 1).is_ok());
    assert!(n.update(&s, 1, Some("open"), None, None, 3).unwrap_err().contains("100"));
}

#[test]
fn project_scope_spans_its_sessions_session_scope_does_not() {
    let n = Notes::load(None);
    n.add(&project("k1", "p1"), "", vec![new("bug", "in project")], 1).unwrap();
    n.add(&pane("k1"), "", vec![new("bug", "alone")], 1).unwrap();
    assert_eq!(n.list(&project("k2", "p1")).len(), 1, "another session of the project sees it");
    assert_eq!(n.list(&pane("k1"))[0].text, "alone");
    assert!(n.list(&pane("k2")).is_empty());
    assert!(n.update(&pane("k2"), 2, Some("done"), None, None, 2).unwrap_err().contains("no note 2"));
    n.update(&project("k2", "p1"), 1, None, Some("edited"), Some("deferred"), 2).unwrap();
    let all = n.list(&project("k1", "p1"));
    assert_eq!(all.len(), 2, "k1 also sees its own notes made before it joined");
    let x = all.iter().find(|x| x.id == 1).unwrap();
    assert_eq!((x.text.as_str(), x.kind.as_str()), ("edited", "deferred"));
}

#[test]
fn update_refuses_bad_values() {
    let n = Notes::load(None);
    let s = pane("k");
    n.add(&s, "", vec![new("bug", "x")], 1).unwrap();
    assert!(n.update(&s, 1, Some("closed"), None, None, 2).is_err());
    assert!(n.update(&s, 1, None, None, Some("idea"), 2).is_err());
    assert!(n.update(&s, 1, None, Some(""), None, 2).is_err());
    assert!(n.update(&s, 9, Some("done"), None, None, 2).is_err());
}

#[test]
fn user_actions() {
    let n = Notes::load(None);
    let s = pane("k");
    n.add(&s, "", vec![new("bug", "x"), new("bug", "y")], 1).unwrap();
    n.user(1, "done", 2).unwrap();
    assert_eq!(n.list(&s)[1].status, "done");
    n.user(1, "open", 3).unwrap();
    assert_eq!(n.list(&s)[1].done_at, None);
    n.user(2, "delete", 4).unwrap();
    assert_eq!(n.list(&s).len(), 1);
    assert!(n.user(2, "done", 5).is_err());
    assert!(n.user(1, "archive", 5).is_err());
}

#[test]
fn persists_round_trip_private() {
    let dir = fresh_dir("notes-rt");
    let n = Notes::load(Some(&dir));
    n.add(&project("k", "p"), "sess", vec![New { kind: "question".into(), text: "why?".into(), file: Some("src/a.rs".into()) }], 7).unwrap();
    let again = Notes::load(Some(&dir));
    let x = &again.list(&project("k2", "p"))[0];
    assert_eq!((x.id, x.session.as_str(), x.file.as_deref(), x.at), (1, "sess", Some("src/a.rs"), 7));
    assert_eq!(again.add(&pane("k"), "", vec![new("bug", "x")], 8).unwrap(), vec![2], "ids go on after a restart");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(dir.join("notes.json")).unwrap().permissions().mode() & 0o777, 0o600);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unreadable_file_is_set_aside_never_overwritten() {
    let dir = fresh_dir("notes-bad");
    std::fs::write(dir.join("notes.json"), "{not json").unwrap();
    let n = Notes::load(Some(&dir));
    assert!(n.list(&pane("k")).is_empty());
    assert_eq!(std::fs::read_to_string(dir.join("notes.json.bak")).unwrap(), "{not json");
    n.add(&pane("k"), "", vec![new("bug", "x")], 1).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("notes.json.bak")).unwrap(), "{not json", "a save leaves the set-aside file");
    std::fs::write(dir.join("notes.json"), r#"{"version": 9}"#).unwrap();
    assert!(Notes::load(Some(&dir)).list(&pane("k")).is_empty(), "another version is set aside too");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_panes_own_notes_stay_when_its_session_joins_a_project() {
    let n = Notes::load(None);
    let k = "tmux:local:t:%1";
    n.add(&pane(k), "s1", vec![new("bug", "mine")], 1).unwrap();
    n.add(&pane("tmux:local:t:%2"), "s2", vec![new("bug", "other pane")], 1).unwrap();
    n.add(&project("tmux:local:t:%2", "P"), "s2", vec![new("bug", "project")], 2).unwrap();
    let texts = |s: &Scope| n.list(s).into_iter().map(|x| x.text).collect::<Vec<_>>();
    assert_eq!(texts(&project(k, "P")), vec!["project", "mine"]);
    assert_eq!(n.open(&project(k, "P")), 2);
    assert!(n.update(&project(k, "P"), 1, Some("done"), None, None, 3).is_ok(), "and its agent can still mark them");
    assert_eq!(texts(&project("tmux:local:t:%3", "P")), vec!["project"], "another member sees the project's alone");
}

#[test]
fn a_tmux_server_started_anew_hides_its_panes_notes() {
    let d = fresh_dir("notes-seen");
    let n = Notes::load(Some(d.as_path()));
    let s = pane("tmux:local:t:%1");
    assert!(n.seen("tmux:local:t:", "id1").unwrap());
    n.add(&s, "s1", vec![new("bug", "yesterday")], 1).unwrap();
    n.add(&pane("tmux:local:u:%1"), "s1", vec![new("bug", "other server")], 1).unwrap();
    assert!(!n.seen("tmux:local:t:", "id1").unwrap(), "the same server keeps them");
    let back = Notes::load(Some(d.as_path()));
    assert!(back.seen("tmux:local:t:", "id2").unwrap());
    assert!(back.list(&s).is_empty(), "a new %1 is not yesterday's");
    assert!(back.update(&s, 1, Some("done"), None, None, 2).is_err(), "nor can its agent mark them");
    assert_eq!(back.list(&pane("tmux:local:u:%1")).len(), 1, "another server's notes stay");
    let again = Notes::load(Some(d.as_path()));
    assert!(again.list(&s).is_empty() && again.scope(1).is_some(), "kept stored, not shown");
}

#[test]
fn only_the_newest_done_notes_are_kept() {
    let n = Notes::load(None);
    let s = pane("k");
    let mut id = 0;
    for round in 0..((DONE_MOST + 20) / 20) as u64 {
        let ids = n.add(&s, "", (0..20).map(|_| new("bug", "x")).collect(), round).unwrap();
        for i in ids {
            n.update(&s, i, Some("done"), None, None, 1000 + i).unwrap();
            id = i;
        }
    }
    let list = n.list(&s);
    assert_eq!(list.len(), DONE_MOST);
    assert_eq!((list[0].id, list.last().unwrap().id), (id, id - DONE_MOST as u64 + 1), "the oldest went");
    n.add(&s, "", vec![new("bug", "open")], 5).unwrap();
    assert_eq!(n.list(&s).len(), DONE_MOST + 1, "open notes do not count");
}

#[test]
fn a_failed_save_adds_nothing() {
    let d = fresh_dir("notes-unsaved");
    std::fs::write(d.as_path().join("blocker"), b"").unwrap();
    let n = Notes::load(Some(&d.as_path().join("blocker")));
    let s = pane("k");
    let e = n.add(&s, "", vec![new("bug", "x")], 1).unwrap_err();
    assert!(e.contains("not saved"), "{e}");
    assert!(n.list(&s).is_empty());
    let ok = Notes::load(Some(d.as_path()));
    assert_eq!(ok.add(&s, "", vec![new("bug", "x")], 1).unwrap(), vec![1], "the id was not used up");
}

#[test]
fn bidi_controls_are_dropped_and_length_counts_after_cleaning() {
    let n = Notes::load(None);
    let s = pane("k");
    n.add(&s, "", vec![new("bug", "a\u{202E}b\u{2066}")], 1).unwrap();
    assert_eq!(n.list(&s)[0].text, "ab");
    let padded = format!("{}{}", "x".repeat(1000), "\u{200F}".repeat(10));
    assert!(n.add(&s, "", vec![new("bug", &padded)], 1).is_ok());
}
