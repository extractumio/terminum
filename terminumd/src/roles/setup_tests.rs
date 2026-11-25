// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! terminum-agents.md AC-283: a manager's setup read and write; terminum-supervisor.md AC-284 through its tool.

use serde_json::json;

use super::{folder_gap, setup, update};
use crate::roles::Role;
use crate::supervisor::Level;
use crate::tasks::ops::sql;
use crate::tasks::store::Store;

const TOOLS: [&str; 3] = ["get_project_setup", "update_project_setup", "set_supervisor_charter"];

/// A store with project p1 in `dir`, and project p2 holding the letters "WEB".
fn store(name: &str, dir: &std::path::Path) -> Store {
    let s = Store::open(crate::testutil::fresh_dir(name).join("kanban.sqlite"));
    s.with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('p1', 'api', 0, ?1, 1)", [dir.to_str().unwrap()]).map_err(sql)?;
        tx.execute("INSERT INTO projects (id, name, color, dir, place, prefix) VALUES ('p2', 'web', 0, '/srv/web', 2, 'WEB')", []).map_err(sql)?;
        Ok(())
    }).unwrap();
    s
}

fn gaps(s: &Store) -> Vec<String> {
    let mut shown = s.with(|tx| setup(tx, "p1")).unwrap();
    folder_gap(&mut shown);
    shown["gaps"].as_array().unwrap().iter().map(|g| format!("{} | {}", g["gap"].as_str().unwrap(), g["how"].as_str().unwrap())).collect()
}

#[test]
fn ac283_the_setup_read_names_each_gap_and_who_closes_it() {
    let dir = crate::testutil::fresh_dir("setup-gaps-dir");
    let s = store("setup-gaps", &dir);
    let shown = s.with(|tx| setup(tx, "p1")).unwrap();
    assert_eq!(shown["project"]["name"], "api");
    assert_eq!(shown["project"]["folder"], dir.to_str().unwrap());
    assert_eq!(shown["pool"], json!([]));
    assert_eq!(shown["auto_dispatch"]["on"], false);
    assert_eq!(shown["integration_on"], false);
    assert_eq!(shown["supervisor"]["enabled"], false);
    assert_eq!(shown["supervisor"]["revision"], 0);
    let all = gaps(&s);
    for (gap, who) in [("the pool is empty", "you: create_agent"), ("no routines", "you: create_schedule"),
        ("the Supervisor's charter is empty", "you: set_supervisor_charter"), ("Supervisor tiers unset: small, medium, strong, independent", "Supervisor tab"),
        ("the Supervisor is off", "Supervisor tab: Level, then Enabled"), ("Auto-Dispatcher is off", "Auto-Dispatch tab")] {
        assert!(all.iter().any(|g| g.starts_with(gap) && g.contains(who)), "{gap} by {who} in {all:?}");
    }
    assert!(!all.iter().any(|g| g.contains("linked git worktree")), "a plain folder");

    // a submodule's `.git` file is no linked worktree
    std::fs::write(dir.join(".git"), "gitdir: ../.git/modules/api\n").unwrap();
    assert!(!gaps(&s).iter().any(|g| g.contains("linked git worktree")), "a submodule");
    // a linked worktree, an agent in the pool, the Supervisor on
    std::fs::write(dir.join(".git"), "gitdir: /Users/alex/api/.git/worktrees/x\n").unwrap();
    s.with(|tx| {
        tx.execute("INSERT INTO agents (id, name, kind, command, color, project) VALUES ('a1', 'Claude Sonnet', 'claude', 'claude', 0, 'p1')", []).map_err(sql)?;
        tx.execute("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES ('p1', 1, 1, 0)", []).map_err(sql).map(drop)
    }).unwrap();
    let all = gaps(&s);
    assert!(all.iter().any(|g| g.contains("linked git worktree") && g.contains("General tab")), "{all:?}");
    assert!(!all.iter().any(|g| g.starts_with("the pool is empty") || g.starts_with("the Supervisor is off")), "{all:?}");
    assert!(all.iter().any(|g| g.starts_with("the Supervisor's charter is empty") && g.contains("the human")), "on, the charter is the owner's: {all:?}");
}

#[test]
fn ac283_the_setup_write_takes_the_window_s_rules_all_or_none() {
    let s = store("setup-write", &crate::testutil::fresh_dir("setup-write-dir"));
    let changed = s.with(|tx| update(tx, "p1", &json!({"prefix": "API", "agents_at_once": "4", "memory_warn_gib": "8", "memory_end_gib": "12"}))).unwrap();
    assert_eq!(changed, ["ID letters", "agents at once", "memory limits"]);
    let shown = s.with(|tx| setup(tx, "p1")).unwrap();
    assert_eq!(shown["project"]["prefix"], "API");
    assert_eq!(shown["project"]["agents_at_once"], 4);
    assert_eq!(shown["project"]["memory_limits"], json!({"warn_gib": 8.0, "end_gib": 12.0}));

    let refused = |args| s.with(|tx| update(tx, "p1", &args)).unwrap_err();
    assert_eq!(refused(json!({"prefix": "WEB"})), "WEB names another project's tasks.");
    assert_eq!(refused(json!({"language": "Rust 2"})), "Language: a language's name, in letters (spaces, hyphens and brackets too)");
    assert_eq!(refused(json!({"prefix": "APX", "agents_at_once": "0"})), "agents_at_once: a whole number from 1 to 100");
    assert_eq!(refused(json!({"memory_warn_gib": "9", "memory_end_gib": "3"})), "The warning is above the end.");
    assert!(refused(json!({"memory_warn_gib": "9"})).starts_with("memory_warn_gib and memory_end_gib go together"));
    assert!(refused(json!({"auto": {"on": true}, "folder": "/srv/x"})).starts_with("Name a field"), "the human's switches are no field");
    let shown = s.with(|tx| setup(tx, "p1")).unwrap();
    assert_eq!(shown["project"]["prefix"], "API", "a refused call keeps none of its fields");
    assert_eq!(shown["auto_dispatch"]["on"], false);

    s.with(|tx| update(tx, "p1", &json!({"agents_at_once": "", "memory_warn_gib": "", "memory_end_gib": ""}))).unwrap();
    let shown = s.with(|tx| setup(tx, "p1")).unwrap();
    assert_eq!((shown["project"]["agents_at_once"].clone(), shown["project"]["memory_limits"].clone()), (json!(null), json!(null)), "empty: none");
}

#[test]
fn ac283_ac284_only_a_manager_finds_the_setup_tools() {
    for tool in TOOLS {
        assert!(crate::roles::MANAGER_TOOLS.contains(&tool), "{tool}");
        assert!(Role::Manager.allows(tool) && Role::Token { manager: true }.allows(tool), "{tool}: a manager's");
        assert!(!Role::Worker.allows(tool) && !Role::Token { manager: false }.allows(tool), "{tool}: no worker's");
        for level in [Level::Observer, Level::Advisor, Level::Operator] {
            assert!(!Role::Supervisor(Some(level)).allows(tool), "{tool}: the Supervisor never writes its own yardstick (AC-258)");
        }
    }
}
