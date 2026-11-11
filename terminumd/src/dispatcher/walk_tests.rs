// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What a walk's transaction does with what it read (terminum-dispatcher.md AC-123, AC-131).

use super::*;
use crate::tasks::store::{Store, HUMAN};

/// A list read before a tool's hook came says the turn ended: the tool stays open while a hook says it works; only
/// once none does (an interrupt cut the tool off, with no end) is it forgotten.
#[test]
fn ac123_an_open_tool_is_forgotten_only_when_no_hook_says_it_works() {
    let store = Store::open(crate::testutil::fresh_dir("walk-tools").join("kanban.sqlite"));
    let key = "tmux:local:t:%7";
    let id = store.with(|tx| {
        let t = ops::create(tx, HUMAN, ops::NewTask { title: "one".into(), ..Default::default() }, 1)?;
        let id = runs::add(tx, &Run { agent_id: "a".into(), kind: "claude".into(), ..Default::default() }, &[t.key], Default::default(), 1)?;
        runs::change(tx, &id, |r| { r.state = runs::Phase::Working; r.pane_key = Some(key.into()); r.open_tools = 1; }).map(|_| id)
    }).unwrap();
    let hooks = Hooks::default();
    let now = std::time::Instant::now();
    hooks.apply(key, &serde_json::json!({"agent": "claude", "event": "PreToolUse", "session": "s1", "tool": "Bash: sleep"}), now);
    let walk = |hooks: &Hooks| store.with(|tx| {
        walk_store(tx, hooks, vec![Read { id: id.clone(), facts: Facts { pane: true, marked: true, alive: true, ..Default::default() }, ended: true, key: key.into() }], 2, now)?;
        runs::by_id(tx, &id)
    }).unwrap();
    let r = walk(&hooks);
    assert_eq!((r.turn_ended, r.open_tools), (true, 1), "its hook says it works: the tool is open");
    let r = walk(&Hooks::default());
    assert_eq!(r.open_tools, 0, "no hook says it works: a tool an interrupt cut off is not open");
}
