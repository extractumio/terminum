// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What a walk's transaction does with what it read (terminum-dispatcher.md AC-123, AC-131).

use super::*;
use std::time::Duration;
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
        walk_store(tx, hooks, &Dispatcher::default(), vec![Read { id: id.clone(), facts: Facts { pane: true, marked: true, alive: true, ..Default::default() }, ended: true, busy: false, key: key.into() }], 2, now)?;
        runs::by_id(tx, &id)
    }).unwrap();
    let r = walk(&hooks);
    assert_eq!((r.turn_ended, r.open_tools), (true, 1), "its hook says it works: the tool is open");
    let r = walk(&Hooks::default());
    assert_eq!(r.open_tools, 0, "no hook says it works: a tool an interrupt cut off is not open");
}

#[test]
fn ac123_every_footer_kind_holds_the_idle_clock_like_the_card() {
    let footer = |pill: &str| ["✻ Worked for 2m · 1 shell still running", "────", "❯ ", "────", pill].map(String::from).to_vec();
    for pill in ["1 shell", "1 monitor", "2 local agents", "1 team", "1 background dynamic workflow",
                 "1 remote dynamic workflow", "1 cloud session", "1 MCP task", "1 background task"] {
        let read = background_tasks(&footer(pill));
        assert_ne!(read, Background::None, "the card's parser counts {pill}");
        let mut r = Run { task: "one".into(), status: "IN PROGRESS".into(), state: runs::Phase::Working, turn_ended: true, idle_for: plan::NOTHING - 1, ..Default::default() };
        let facts = Facts { pane: true, marked: true, alive: true, active: read != Background::None, dt: plan::NOTHING + 1, ..Default::default() };
        assert!(plan::step(&mut r, &facts).is_empty(), "{pill}: no nudge or failure past an hour");
        assert_eq!(r.idle_for, 0);
        let quiet = Facts { active: background_tasks(&footer("0 monitors")) != Background::None, ..facts };
        assert!(plan::step(&mut r, &quiet).iter().any(|a| matches!(a, Act::Move { to: "FAILED", .. })), "work ended: the idle clock runs");
    }
}

/// AC-123, AC-198: the same return and ten-second walk that failed a task after a long Verify;
/// stale clocks model a run queued before the fix was installed, with every request already used.
#[test]
fn ac198_ac123_a_red_verify_after_an_hour_restarts_the_idle_clock() {
    red_verify_return(3);    // no walk saw the integration wait
    red_verify_return(7290); // an older server kept walking and accumulated idle time
}

fn red_verify_return(last_walk: i64) {
    use crate::integrate::engine::tests::{queued_with, world, Sh};
    use crate::integrate::stages::verify_step;
    use crate::tasks::integration::Settings;

    let w = world("walk-return", Settings { verify: "exit 1".into(), check_minutes: 180, ..Settings::default() });
    let task = queued_with(&w, 1, "b.txt", "b\n", true);
    let key = "tmux:local:t:%7";
    let id = w.s.with(|tx| {
        tx.execute("UPDATE tasks SET budget_seconds = 7200 WHERE slug = ?1", [&task]).map_err(ops::sql)?;
        let id = runs::add(tx, &Run { agent_id: "a".into(), kind: "claude".into(), ..Default::default() }, &[ops::key_of(tx, &task)?], Default::default(), 3)?;
        runs::change(tx, &id, |r| {
            r.state = runs::Phase::Working;
            r.pane_key = Some(key.into());
            r.turn_ended = true;
            r.idle_for = plan::NOTHING + 100;
            r.nudged = plan::NUDGES;
            r.nudge_turn = true;
            r.last_walk = last_walk;
            r.budget_used = 600;
            r.warned = true;
        })?;
        Ok(id)
    }).unwrap();
    let runner = Sh { hang: true, ..Default::default() };
    let home = w.root.join("home");
    verify_step(&w.s, &runner, &home, 100).unwrap();
    runner.ran.lock().unwrap().values_mut().for_each(|(code, _)| *code = Some(1));
    // The transition must reset persisted clocks whether the last walk was recent or before the two-hour wait.
    verify_step(&w.s, &runner, &home, 7300).unwrap();
    let store = Store::open(w.root.join("kanban.sqlite")); // a restart reads the reset, too
    let mut r = store.with(|tx| runs::by_id(tx, &id)).unwrap();
    assert_eq!((r.status.as_str(), r.idle_for, r.nudged, r.nudge_turn, r.last_walk), ("IN PROGRESS", 0, 0, false, 7300));
    assert_eq!((r.budget_used, r.warned), (600, true), "return preserves the budget already spent and its warning");
    let said = store.with(|tx| ops::history(tx, &task)).unwrap().to_string();
    assert!(said.contains("returned: red check: its Prepare or Verify ended with status 1"), "{said}");
    let walk = |now| store.with(|tx| {
        walk_store(tx, &Hooks::default(), &Dispatcher::default(), vec![Read { id: id.clone(), facts: Facts { pane: true, marked: true, alive: true, ..Default::default() }, ended: true, busy: false, key: key.into() }], now, Instant::now())
    }).unwrap();
    assert!(walk(7310).0.is_empty(), "ten seconds after return: no immediate failure or request");
    assert_eq!(store.with(|tx| runs::by_id(tx, &id)).unwrap().budget_used, 610, "only the ten seconds after return count");
    for now in (7340..7600).step_by(30) {
        assert!(walk(now).0.is_empty());
    }
    let later = walk(7600).0;
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].act, Act::Nudge, "five fresh minutes of quiet get a new request");
    store.with(|tx| runs::change(tx, &id, |r| { r.nudged = 1; r.idle_for = 0; })).unwrap(); // request delivered
    for now in (7630..11200).step_by(30) {
        assert!(walk(now).0.is_empty(), "the request gets its whole hour");
    }
    walk(11200);
    r = store.with(|tx| runs::by_id(tx, &id)).unwrap();
    assert_eq!(r.status, "FAILED");
    let said = store.with(|tx| ops::history(tx, &task)).unwrap().to_string();
    assert!(said.contains("No result from the agent within an hour of Dispatcher's request."), "{said}");
}

/// AC-127: the watchdog retains silence from before a wait, and ignores time and hooks during it.
#[test]
fn ac127_silence_resumes_from_the_time_before_a_wait() {
    let start = Instant::now();
    let paused = start + Duration::from_secs(420);
    let resumed = paused + Duration::from_secs(2400);
    for status in ["WAIT FOR HUMAN", "TODO"] {
        let mut watch = PaneWatch::default();
        for id in ["quiet", "heard"] {
            watch.subs.insert(id.into(), super::super::Sub { kind: "reviewer".into(), last: start, told: plan::SubStep::Nothing });
        }
        let mut run = Run { status: status.into(), asked: true, ..Default::default() };
        assert!(subagent_steps(&mut watch, &run, false, paused).is_empty());
        assert!(subagent_steps(&mut watch, &run, false, resumed).is_empty());
        assert_eq!(watch.subs["quiet"].last, start);
        watch.subs.get_mut("heard").unwrap().last = paused + Duration::from_secs(1);
        run.status = "IN PROGRESS".into();
        subagent_steps(&mut watch, &run, false, resumed);
        assert_eq!(resumed.duration_since(watch.subs["quiet"].last), Duration::from_secs(420));
        assert_eq!(watch.subs["heard"].last, resumed);
    }
}

/// AC-127, AC-123: an active sibling can still fail, but the task that waits keeps its status.
#[test]
fn ac127_a_watchdog_failure_moves_only_active_tasks_in_a_mixed_run() {
    let root = crate::testutil::fresh_dir("walk-subagents");
    let store = Store::open(root.join("kanban.sqlite"));
    let mut watch = PaneWatch::default();
    let start = Instant::now();
    watch.subs.insert("sub-1".into(), super::super::Sub {
        kind: "reviewer".into(),
        last: start,
        told: plan::SubStep::Tell,
    });
    store.with(|tx| {
        let task = |title: &str| ops::create(tx, HUMAN, ops::NewTask { title: title.into(), ..Default::default() }, 1);
        let (active, waiting) = (task("active")?, task("waiting sibling")?);
        let id = runs::add(tx, &Run::default(), &[active.key.clone(), waiting.key.clone()], Default::default(), 1)?;
        ops::move_to(tx, HUMAN, &active.key, "IN PROGRESS", 2)?;
        ops::move_to(tx, HUMAN, &waiting.key, "WAIT FOR HUMAN", 2)?;
        let run = runs::by_id(tx, &id)?;
        let at = start + Duration::from_secs(plan::SUB_END as u64);
        let (_, act) = watch_subagents(tx, &mut watch, &run, false, at)?.expect("failure decided");
        assert!(matches!(act, Act::SubagentFailed(_)));
        assert_eq!(ops::get(tx, &active.key)?.status, "FAILED");
        assert_eq!(ops::get(tx, &waiting.key)?.status, "WAIT FOR HUMAN");
        Ok(())
    }).unwrap();
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
