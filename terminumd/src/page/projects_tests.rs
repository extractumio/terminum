// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Projects (AC-92): names, colors, members, order and closed panes, in memory.

use super::*;

const A: &str = "tmux:local:terminum:%1";
const B: &str = "tmux:local:terminum:%2";
const C: &str = "tmux:devbox:default:%1";

fn store() -> Projects {
    Projects::default()
}

#[test]
fn a_new_project_is_named_and_colored_apart_from_the_others() {
    let mut ps = store();
    let p1 = ps.add(A, None, None, None, "/Users/alex/api".into()).unwrap();
    let p2 = ps.add(B, None, None, None, "/Users/alex/web".into()).unwrap();
    let names: Vec<_> = ps.list().iter().map(|p| (p.name.clone(), p.color, p.dir.clone())).collect();
    assert_eq!(names, [("Group 1".to_string(), 0, "/Users/alex/api".to_string()), ("Group 2".into(), 1, "/Users/alex/web".into())]);
    ps.delete(&p1).unwrap();
    let p3 = ps.add(A, None, None, None, "/tmp".into()).unwrap();
    let p = ps.list().iter().find(|p| p.id == p3).unwrap();
    assert_eq!((p.name.as_str(), p.color), ("Group 1", 0), "the lowest free name and color again");
    assert_ne!(p2, p3);
}

#[test]
fn a_pane_is_in_one_project_and_an_empty_one_goes() {
    let mut ps = store();
    let p1 = ps.add(A, None, None, None, "/a".into()).unwrap();
    ps.add(B, Some(&p1), None, None, "/a".into()).unwrap();
    let p2 = ps.add(B, None, None, None, "/b".into()).unwrap();
    assert_eq!(ps.of(B).unwrap().id, p2, "moved, not copied");
    assert_eq!(ps.of(A).unwrap().members, [A]);
    ps.remove(A).unwrap();
    assert!(ps.list().iter().all(|p| p.id != p1), "its last session left: the project goes");
    assert!(ps.add(A, Some("nope"), None, None, "/".into()).is_err());
}

#[test]
fn order_members_rename_and_after() {
    let mut ps = store();
    let p1 = ps.add(A, None, None, None, "/a".into()).unwrap();
    ps.add(B, Some(&p1), None, None, "/a".into()).unwrap();
    ps.add(C, Some(&p1), Some(A), None, "/a".into()).unwrap();
    assert_eq!(ps.of(A).unwrap().members, [A, C, B], "a clone goes right after its session");
    ps.members(&p1, &[B.into(), A.into()]).unwrap();
    assert_eq!(ps.of(A).unwrap().members, [B, A, C]);
    ps.rename(&p1, "  API  ").unwrap();
    assert_eq!(ps.of(A).unwrap().name, "API");
    assert!(ps.rename(&p1, " ").is_err() && ps.rename(&p1, "a\u{7}").is_err());
    let p2 = ps.add("tmux:local:terminum:%9", None, None, None, "/z".into()).unwrap();
    ps.order(&[p2.clone(), p1.clone()]).unwrap();
    assert_eq!(ps.list().iter().map(|p| p.id.clone()).collect::<Vec<_>>(), [p2, p1]);
}

#[test]
fn closed_panes_leave_when_their_server_is_read() {
    let mut ps = store();
    let p1 = ps.add(A, None, None, None, "/a".into()).unwrap();
    ps.add(C, Some(&p1), None, None, "/a".into()).unwrap();
    let live: HashSet<&str> = [B].into_iter().collect();
    assert!(ps.seen("tmux:local:terminum:", Some("m1"), &live), "the first read adopts the server");
    assert_eq!(ps.of(A).unwrap().members, [A, C], "added just now: spared (a read on its way may not have it)");
    ps.fresh.clear();
    assert!(ps.seen("tmux:local:terminum:", Some("m1"), &live));
    assert_eq!(ps.of(C).unwrap().members, [C], "another server's pane stays until that server is read");
    assert!(!ps.seen("tmux:local:terminum:", Some("m1"), &live), "nothing more to change");
    ps.seen("tmux:devbox:default:", Some("d1"), &HashSet::new());
    ps.fresh.clear();
    assert!(ps.seen("tmux:devbox:default:", Some("d1"), &HashSet::new()));
    assert!(ps.list().is_empty());
}

/// A tmux server started anew numbers its panes anew: its old members never name new panes.
#[test]
fn a_server_started_anew_keeps_projects_only_through_its_restore() {
    let mut ps = store();
    let p1 = ps.add(A, None, None, None, "/a".into()).unwrap();
    ps.add(B, Some(&p1), None, None, "/a".into()).unwrap();
    ps.fresh.clear();
    let both: HashSet<&str> = [A, B].into_iter().collect();
    ps.seen("tmux:local:terminum:", Some("m1"), &both);
    // restored: %1 -> %7, %2 not made again; the next read (with the old keys still) adopts it
    assert!(ps.restarted("tmux:local:terminum:", &[("%1".into(), "%7".into())]));
    assert_eq!(ps.of("tmux:local:terminum:%7").unwrap().members, ["tmux:local:terminum:%7"]);
    ps.seen("tmux:local:terminum:", Some("m2"), &HashSet::new());
    assert!(ps.of("tmux:local:terminum:%7").is_some(), "a read right after the restore takes nothing");
    // started anew without a restore: its %7 is another pane now
    let seven: HashSet<&str> = ["tmux:local:terminum:%7"].into_iter().collect();
    assert!(ps.seen("tmux:local:terminum:", Some("m3"), &seven));
    assert!(ps.list().is_empty(), "another server's pane %7 is not the project's");
}
