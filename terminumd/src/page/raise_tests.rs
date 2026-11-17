// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-92: a session that took a message goes to the top of its project, or of its tmux group.

use serde_json::{json, Value};

use super::*;
use crate::testutil::{self, Tmux};

/// A layout row: pane `pane` (0 alone in its window) of window `window`.
fn row(id: &str, window: &str, pane: u64) -> Value {
    json!({"id": id, "window": window, "pane": pane})
}

fn layout() -> Value {
    json!([
        {"wid": "devbox/main", "items": [row("a", "@1", 0), row("b", "@2", 1), row("b2", "@2", 2), row("c", "@3", 0),
            {"id": "q", "window": "@9", "pane": 0, "quickOf": "c"}, row("d", "@4", 0)]},
        {"wid": "devbox/notes", "items": [row("n", "@5", 0)]},
    ])
}

#[test]
fn ac92_a_panes_row_is_its_windows_first_pane_and_a_quick_shells_its_parent() {
    let l = layout();
    assert_eq!(row_of(&l, "a").as_deref(), Some("a"));
    assert_eq!(row_of(&l, "b2").as_deref(), Some("b"), "a split's other pane: its window's row");
    assert_eq!(row_of(&l, "q").as_deref(), Some("c"), "a quick shell: the row it is under");
    assert_eq!(row_of(&l, "n").as_deref(), Some("n"), "another group's");
    assert_eq!(row_of(&l, "gone"), None);
}

#[test]
fn ac92_a_groups_windows_are_its_rows_in_no_project() {
    let l = layout();
    let (wid, top, free) = group_windows(&l, "d", |k| k == "b").unwrap();
    assert_eq!((wid.as_str(), top.as_str()), ("devbox/main", "@4"));
    assert_eq!(free, ["@1", "@3", "@4"], "a project's window, a split's second pane and a quick shell's window are no rows of it");
    assert_eq!(group_windows(&l, "b", |k| k == "b"), None, "a project's session is not raised in its group");
    assert_eq!(group_windows(&l, "gone", |_| false), None);
}

/// `p`'s panes in the order of their places in it.
fn pick(p: &[String], at: [usize; 4]) -> Vec<String> {
    at.iter().map(|&i| p[i].clone()).collect()
}

fn panes_in_order(s: &Tmux) -> Vec<String> {
    s.read(&["list-windows", "-t", "main", "-F", "#{pane_id}"]).lines().map(str::to_string).collect()
}

/// The whole way on a private tmux: a row outside projects goes to its group's top, the windows
/// of a project's sessions keep their places; a project's session goes to its project's top only.
#[tokio::test]
async fn ac92_a_session_outside_projects_goes_to_its_groups_top() {
    let Some(s) = Tmux::start("raise-group", (80, 24), "sleep 60") else { return };
    let (root, app) = testutil::app("raise-group");
    let link = crate::tmux::server::Link::Local { tmux: "tmux".into(), socket: s.socket.clone(), conf: "/dev/null".into() };
    let server = crate::tmux::Server::new("devbox", &s.socket, link, Default::default());
    app.tmux.add_host(server.clone());
    server.sync_clients(&["main".to_string()]).unwrap();
    for _ in 0..3 {
        s.read(&["new-window", "-t", "main:", "sleep 60"]);
    }
    let split = s.read(&["split-window", "-d", "-t", "main:1", "-P", "-F", "#{pane_id}", "sleep 60"]).trim().to_string();
    let p = panes_in_order(&s);
    let windows: Vec<String> = s.read(&["list-windows", "-t", "main", "-F", "#{window_id}"]).lines().map(str::to_string).collect();
    let key = |pane: &str| server.key(pane);
    let pane_of = |i: usize| if i == 1 { 1 } else { 0 };
    let mut items: Vec<Value> = (0..4).map(|i| row(&key(&p[i]), &windows[i], pane_of(i))).collect();
    items.push(row(&key(&split), &windows[1], 2));
    *app.layout.write() = json!([{"wid": "devbox/main", "items": items}]);
    let id = app.projects.lock().create("api", Some("devbox".into()), String::new()).unwrap();
    app.projects.lock().add_to(&key(&p[2]), &id, None).unwrap();

    raise(&app, &key(&p[3])).await;
    assert_eq!(panes_in_order(&s), pick(&p, [3, 0, 2, 1]), "to the group's top, the project's window kept in its place");

    raise(&app, &key(&p[2])).await;
    assert_eq!(panes_in_order(&s), pick(&p, [3, 0, 2, 1]), "a project's session moves no window");

    raise(&app, &key(&p[3])).await;
    assert_eq!(panes_in_order(&s), pick(&p, [3, 0, 2, 1]), "at the top already: nothing moves");

    raise(&app, &key(&split)).await;
    assert_eq!(panes_in_order(&s), pick(&p, [1, 3, 2, 0]), "a split's other pane raises its window, the others keep their order");
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}
