// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A red batch's integration agent (terminum-integration.md AC-194) on real repositories: the engine wants it in a
//! worktree of its own, its prompt holds the batch's facts alone, and its three tools decide only for its batch.
use std::path::{Path, PathBuf};

use serde_json::json;

use super::*;
use crate::integrate::engine::tests::{batch, go, on_origin, queued, round_row, sh, status, world, Sh, World};
use crate::tasks::integration::Settings;
use crate::tasks::ops::INTEGRATION;
use crate::tasks::store::HUMAN;
use crate::tasks::tools::Who;

const PANE: &str = "tmux:local:terminum:%9";

fn who(key: &str) -> Who {
    Who { key: key.into(), agent: "claude".into(), title: "api".into(), project: Some("api".into()), dir: None }
}

/// Api's queue, `check` its Check, with Opus reviewer as its integration agent unless `agent` is false.
fn api(name: &str, check: &str, agent: bool, red_rounds_most: i64) -> World {
    let w = world(name, Settings { check: check.into(), ..Settings::default() });
    w.s.with(|tx| {
        tx.execute("INSERT INTO agents (id, project, name, kind, command, color) VALUES ('a1', 'api', 'Opus reviewer', 'claude', 'claude', 0)", []).map_err(sql)?;
        let s = integration::read(tx, "api")?;
        integration::keep(tx, "api", &Settings { agent: agent.then(|| "a1".into()), red_rounds_most, agent_budget_minutes: 45, ..s })
    }).unwrap();
    w
}

/// Two tasks queued, b.txt and c.txt, and their batch red.
fn red_with_agent(name: &str, check: &str, red_rounds_most: i64) -> (World, String, String, String) {
    let w = api(name, check, true, red_rounds_most);
    let (a, b) = (queued(&w, 1, "b.txt", "b\n"), queued(&w, 2, "c.txt", "c\n"));
    go(&w, &Sh::default(), 4);
    assert_eq!(batch(&w).state, "red");
    let i = i_of(&w);
    (w, a, b, i)
}

fn i_of(w: &World) -> String {
    let b = batch(w);
    w.s.with(|tx| bs::slug(tx, b.task.as_deref().unwrap())).unwrap()
}

/// What start_agents and the press do: the wanted taken, its task IN PROGRESS, its run in PANE.
fn pressed(w: &World) -> String {
    let i = i_of(w);
    w.s.with(|tx| {
        assert_eq!(take_wanted(tx, 200)?.len(), 1, "its agent is wanted");
        let t = ops::get(tx, &i)?;
        ops::place(tx, DISPATCHER, &t, "IN PROGRESS", 200)
    }).unwrap();
    crate::tasks::tools::tests::running(&w.s, &i, PANE);
    i
}

fn call(w: &World, tool: fn(&Transaction, &Who, &Value, i64) -> Result<String, String>, args: Value) -> Result<String, String> {
    w.s.with(|tx| tool(tx, &who(PANE), &args, 300))
}

fn fixing(w: &World, commit: &str) -> Result<String, String> {
    add_fix(&w.s, &who(PANE), &json!({"commit": commit}), 300)
}

/// A commit of `file` in the agent's worktree; its hash.
fn commit_in(tree: &Path, file: &str) -> String {
    std::fs::write(tree.join(file), "fixed\n").unwrap();
    sh(tree, &["add", file]);
    sh(tree, &["commit", "-qm", "the agent's fix"]);
    sh(tree, &["rev-parse", "HEAD"])
}

fn agent_tree(w: &World) -> PathBuf {
    PathBuf::from(batch(w).agent_tree.expect("its worktree"))
}

const ANALYSED: &str = "Failing step: check\nEvidence: exit 3\nCommit and lines: c.txt:1\nWhat to fix: the test";

#[test]
fn ac194_red_wants_its_agent_in_a_worktree_of_its_own_at_the_candidate() {
    let w = api("ia-want", "exit 3", true, 3);
    let a = queued(&w, 1, "b.txt", "b\n");
    w.s.with(|tx| {
        tx.execute("UPDATE tasks SET title = 'a title of the task''s own' WHERE slug = ?1", [&a]).map_err(sql)?;
        ops::comment(tx, HUMAN, &a, "words of the task's own, never the agent's", 3).map(drop)
    }).unwrap();
    go(&w, &Sh::default(), 4);
    let (b, i) = (batch(&w), i_of(&w));
    let tree = agent_tree(&w);
    assert!(tree.ends_with("agent-1"), "{}", tree.display());
    assert_eq!(sh(&tree, &["rev-parse", "HEAD"]), b.candidate.clone().unwrap(), "at the candidate");
    let prompt = w.s.with(|tx| prompt_of(tx, b.task.as_deref().unwrap())).unwrap().unwrap();
    for fact in [i.as_str(), "exit 3", &tree.display().to_string(), &a, "never call finish_task", "45 minutes"] {
        assert!(prompt.contains(fact), "{fact} in {prompt}");
    }
    assert!(prompt.contains(&format!("{a} at {} (b.txt)", short(&queued_commit(&w, &a)))), "each task with its commit and files: {prompt}");
    assert!(!prompt.contains("of the task's own"), "no task's title or words in its prompt: {prompt}");
    assert!(w.s.with(|tx| finish_refused(tx, b.task.as_deref().unwrap())).unwrap().unwrap().contains("ends when its batch 1 does"));
    assert_eq!(w.s.with(|tx| take_wanted(tx, 200)).unwrap(), vec![(b.id, i.clone(), "a1".to_string())]);
    assert_eq!(w.s.with(|tx| ops::get(tx, &i)).unwrap().budget_seconds, Some(45 * 60), "its budget the project's");
    assert!(w.s.with(|tx| take_wanted(tx, 201)).unwrap().is_empty(), "taken once");
}

#[test]
fn ac194_without_an_agent_or_with_its_red_rounds_spent_a_human_decides() {
    let (w, _, _, i) = red_with_agent("ia-spent", "exit 3", 1);
    assert_eq!(status(&w, &i), "WAIT FOR HUMAN");
    assert!(w.s.with(|tx| take_wanted(tx, 200)).unwrap().is_empty(), "no agent started");
    let w = api("ia-none", "exit 3", false, 3);
    queued(&w, 1, "b.txt", "b\n");
    go(&w, &Sh::default(), 4);
    let i = i_of(&w);   // read outside: the store's lock is not taken twice
    assert_eq!(status(&w, &i), "WAIT FOR HUMAN");
    assert!(w.s.with(|tx| ops::history(tx, &i)).unwrap().to_string().contains("has no integration agent"));
}

#[test]
fn ac194_several_go_back_before_one_recheck_and_the_rest_is_built_anew() {
    let w = api("ia-return", "exit 3", true, 3);
    let (a, b, c) = (queued(&w, 1, "b.txt", "b\n"), queued(&w, 2, "c.txt", "c\n"), queued(&w, 3, "d.txt", "d\n"));
    go(&w, &Sh::default(), 4);
    let i = pressed(&w);
    assert_eq!(w.s.with(|tx| give_back(tx, &who("tmux:local:terminum:%1"), &json!({"task": a, "analysis": ANALYSED}), 300)).unwrap_err(),
               "Only a batch's integration task decides for it.", "another session decides nothing");
    assert!(call(&w, give_back, json!({"task": a, "analysis": "it broke"})).unwrap_err().contains("lacks Failing step, Evidence, Commit and lines, What to fix"));
    assert!(call(&w, give_back, json!({"task": i, "analysis": ANALYSED})).unwrap_err().contains("is not a task of batch 1"));
    call(&w, give_back, json!({"task": a, "analysis": ANALYSED})).unwrap();
    call(&w, give_back, json!({"task": b, "analysis": ANALYSED})).unwrap();
    assert_eq!((status(&w, &a), status(&w, &b)), ("IN PROGRESS".into(), "IN PROGRESS".into()));
    assert!(w.s.with(|tx| ops::history(tx, &a)).unwrap().to_string().contains("returned: red check"));
    assert_eq!(batch(&w).state, "red", "the batch waits for integration_recheck");
    assert!(call(&w, recheck, json!({})).unwrap().contains("built anew"));
    assert_eq!(status(&w, &i), INTEGRATION, "its turn ends, its budget stops");
    go(&w, &Sh::default(), 1);
    let again = batch(&w);
    assert_eq!((again.round, again.tasks.len(), status(&w, &c)), (2, 1, INTEGRATION.to_string()), "one more round, not one a task");
    assert!(call(&w, recheck, json!({})).unwrap_err().contains("nothing to decide now"), "a batch under check is not the agent's");
}

#[test]
fn ac194_ac190_a_red_round_built_anew_keeps_its_result_words_and_end() {
    let (w, a, _, _) = red_with_agent("ia-kept", "exit 3", 3);
    let red = round_row(&w, 1);
    assert_eq!(red.0, "red");
    assert!(red.1.as_deref().unwrap().starts_with("The check ended with status 3."), "{red:?}");
    pressed(&w);
    call(&w, give_back, json!({"task": a, "analysis": ANALYSED})).unwrap();
    call(&w, recheck, json!({})).unwrap();
    // the engine builds it anew long after the red check ended: the agent's analysis is no check time
    super::super::engine::step(&w.s, &Sh::default(), &w.root.join("home"), 5000).unwrap();
    assert_eq!(batch(&w).round, 2, "built anew");
    assert_eq!(round_row(&w, 1), red, "not rewritten moved: the target never moved");
    assert_eq!(round_row(&w, 2).0, "built");
}

#[test]
fn ac194_every_task_returned_ends_the_batch_with_nothing_published() {
    let (w, a, b, _) = red_with_agent("ia-all", "exit 3", 3);
    let i = pressed(&w);
    call(&w, give_back, json!({"task": a, "analysis": ANALYSED})).unwrap();
    call(&w, give_back, json!({"task": b, "analysis": ANALYSED})).unwrap();
    assert_eq!((batch(&w).state.as_str(), status(&w, &i)), ("given up", "DONE".to_string()));
    assert_eq!(on_origin(&w), "a.txt");
}

#[test]
fn ac194_checked_again_unchanged_once_with_why() {
    let (w, _, _, _) = red_with_agent("ia-again", "exit 3", 3);
    let candidate = batch(&w).candidate;
    pressed(&w);
    assert!(call(&w, recheck, json!({})).unwrap_err().contains("Say why"));
    call(&w, recheck, json!({"why": "a port was held"})).unwrap();
    assert_eq!((batch(&w).state.as_str(), batch(&w).round, batch(&w).candidate), ("checking", 2, candidate));
    go(&w, &Sh::default(), 2);
    pressed(&w);
    assert_eq!(call(&w, recheck, json!({"why": "again"})).unwrap_err(), "Batch 1 was checked again unchanged once: return a task, add a fix, or ask a human.");
}

#[test]
fn ac194_a_fix_of_its_own_joins_first_outlives_a_red_round_and_is_published() {
    // green only with the fix and without c.txt: round 2 is red again, and its agent returns the task that wrote it
    let (w, a, b, i) = red_with_agent("ia-fix", "test -f fix.txt && test ! -f c.txt", 3);
    pressed(&w);
    let tree = agent_tree(&w);
    let fix = commit_in(&tree, "fix.txt");
    assert!(fixing(&w, "--upload-pack=x").unwrap_err().contains("hex digits"));
    assert!(fixing(&w, "abcdef1234").unwrap_err().contains("is no commit in your worktree"));
    fixing(&w, &fix).unwrap();
    assert_eq!(sh(&tree, &["rev-parse", "HEAD"]), fix, "its worktree stands where it stood");
    assert!(!PathBuf::from(format!("{}-fix-{}", tree.display(), short(&fix))).exists(), "the pick's worktree is gone");
    call(&w, recheck, json!({})).unwrap();
    go(&w, &Sh::default(), 3);
    assert_eq!((batch(&w).state.as_str(), batch(&w).round, batch(&w).fixes.len()), ("red", 2, 1), "red again, its fix kept");
    pressed(&w);   // the press moves its task out of INTEGRATION: the fix stays with the batch
    call(&w, give_back, json!({"task": b, "analysis": ANALYSED})).unwrap();
    call(&w, recheck, json!({})).unwrap();
    go(&w, &Sh::default(), 6);
    assert_eq!((status(&w, &a), status(&w, &b), status(&w, &i)), ("DONE".into(), "IN PROGRESS".into(), "DONE".into()));
    assert_eq!(on_origin(&w), "a.txt\nb.txt\nfix.txt");
    assert!(!tree.exists(), "its worktree removed when its batch ended");
}

#[test]
fn ac194_a_fix_that_needs_the_batch_does_not_apply_alone() {
    let (w, _, _, _) = red_with_agent("ia-nofix", "exit 3", 3);
    pressed(&w);
    let tree = agent_tree(&w);
    std::fs::write(tree.join("b.txt"), "b, changed\n").unwrap();   // b.txt is a task's, not the target's
    sh(&tree, &["commit", "-qam", "changes a task's file"]);
    let head = sh(&tree, &["rev-parse", "HEAD"]);
    assert!(fixing(&w, &head).unwrap_err().contains("does not apply alone to the target"));
    assert_eq!(sh(&tree, &["rev-parse", "HEAD"]), head);
    assert!(batch(&w).fixes.is_empty());
}

#[test]
fn ac194_each_integration_agent_keeps_a_slot_of_its_own() {
    let (w, _, _, i) = red_with_agent("ia-slot", "exit 3", 3);
    pressed(&w);   // a1 runs api's integration task in its kept slot
    w.s.with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('web', 'web', 2, '/srv/web', 1)", []).map_err(sql)?;
        tx.execute("INSERT INTO agents (id, project, name, kind, command, color, at_once) VALUES ('a2', 'web', 'Web reviewer', 'claude', 'claude', 1, 1)", []).map_err(sql)?;
        integration::keep(tx, "web", &Settings { on: true, agent: Some("a2".into()), ..Settings::default() })?;
        let t = ops::create(tx, DISPATCHER, ops::NewTask { title: "Integration batch 1: WEB-1".into(), project: Some("web".into()), integration: true, ..Default::default() }, 5)?;
        tx.execute("INSERT INTO integration_batches (project, number, task, tasks, base_sha, state, created_at) VALUES ('web', 1, ?1, '[]', 'x', 'red', 5)", [&t.key]).map_err(sql)?;
        let kept = crate::tasks::schedules::kept_by_agent(tx)?;
        assert_eq!(kept.get("a2").copied().unwrap_or_default(), crate::tasks::schedules::Kept { slots: 0, schedules: 0, integration: true });
        let a2 = crate::tasks::agents::chosen(tx, "a2", "web")?;
        assert_eq!(crate::dispatcher::pool::check_for(tx, &a2, "web", Some(&t.key), None)?, None, "a1's run stands in a1's slot, not a2's");
        Ok(())
    }).unwrap();
    assert_eq!(status(&w, &i), "IN PROGRESS");
}

/// TERM-500: an ended integration task's session, its agent idle until its close (terminum-dispatcher.md AC-125),
/// keeps no slot: the next red batch's agent starts at once beside a driver task's run, then stands in the kept slot.
#[test]
fn ac194_an_ended_integration_task_s_closing_session_leaves_the_kept_slot() {
    use crate::dispatcher::pool::{self, Slots};
    let (w, _, _, first) = red_with_agent("ia-closing", "exit 3", 3);
    pressed(&w);
    let driver = w.s.with(|tx| ops::create(tx, HUMAN, ops::NewTask { title: "a driver task".into(), project: Some("api".into()), ..Default::default() }, 5)).unwrap();
    crate::tasks::tools::tests::running(&w.s, &driver.id, "tmux:local:terminum:%10");
    let second = w.s.with(|tx| {
        tx.execute("UPDATE agents SET at_once = 2 WHERE id = 'a1'", []).map_err(sql)?;
        tx.execute("UPDATE tasks SET status = 'DONE' WHERE slug = ?1", [&first]).map_err(sql)?;   // batch 1 published, its session not closed yet
        let t = ops::create(tx, DISPATCHER, ops::NewTask { title: "Integration batch 2: API-9".into(), project: Some("api".into()), integration: true, ..Default::default() }, 5)?;
        tx.execute("INSERT INTO integration_batches (project, number, task, tasks, base_sha, state, created_at) VALUES ('api', 2, ?1, '[]', 'x', 'red', 5)", [&t.key]).map_err(sql)?;
        let a1 = crate::tasks::agents::chosen(tx, "a1", "api")?;
        assert!(pool::check_for(tx, &a1, "api", Some(&t.key), None)?.is_some(), "its agent still in its turn after DONE holds the slot");
        let run = crate::dispatcher::runs::last_of(tx, &ops::key_of(tx, &first)?)?.unwrap();
        crate::dispatcher::runs::change(tx, &run.id, |r| r.turn_ended = true)?;
        assert_eq!(pool::check_for(tx, &a1, "api", Some(&t.key), None)?, None, "the kept slot is free while batch 1's idle session closes");
        assert!(pool::check_for(tx, &a1, "api", None, None)?.is_some(), "an ordinary run still finds its one slot taken by the driver");
        Ok(t.id)
    }).unwrap();
    w.s.with(|tx| { let t = ops::get(tx, &second)?; ops::place(tx, DISPATCHER, &t, "IN PROGRESS", 6) }).unwrap();   // as its press does
    crate::tasks::tools::tests::running(&w.s, &second, "tmux:local:terminum:%11");
    w.s.with(|tx| {
        let a1 = crate::tasks::agents::chosen(tx, "a1", "api")?;
        let key = ops::key_of(tx, &second)?;
        let run = crate::dispatcher::runs::last_of(tx, &key)?.unwrap();
        assert_eq!(Slots::read(tx)?.slot_of(&run.id), Some(pool::Slot::Integration), "batch 2's run stands in the kept slot");
        assert_eq!(Slots::read(tx)?.free_of(&a1), 0);
        let shown = pool::board(tx)?;
        let rows = shown["agents"]["a1"].as_array().unwrap();
        assert_eq!(rows.len(), 3, "the closing session stays shown");
        assert_eq!(rows.iter().filter(|r| r["ended"] == true).map(|r| r["tasks"].clone()).collect::<Vec<_>>(), vec![json!(first)]);
        assert_eq!(shown["full"], json!(["a1"]), "driver and kept slot: at its limit, the closing session not counted");
        Ok(())
    }).unwrap();
}

/// TERM-518: a project at its own agent limit holds every run of it but a red batch's integration task in its kept
/// slot, which starts at once and then counts against that limit as any run.
#[test]
fn ac194_a_red_batch_s_agent_starts_at_once_with_its_project_at_its_limit() {
    use crate::dispatcher::pool;
    let (w, _, _, i) = red_with_agent("ia-limit", "exit 3", 3);
    let driver = w.s.with(|tx| ops::create(tx, HUMAN, ops::NewTask { title: "a driver task".into(), project: Some("api".into()), ..Default::default() }, 5)).unwrap();
    crate::tasks::tools::tests::running(&w.s, &driver.id, "tmux:local:terminum:%10");
    w.s.with(|tx| {
        tx.execute("UPDATE agents SET at_once = 3 WHERE id = 'a1'", []).map_err(sql)?;
        tx.execute("UPDATE projects SET agent_limit = 1 WHERE id = 'api'", []).map_err(sql)?;
        let a1 = crate::tasks::agents::chosen(tx, "a1", "api")?;
        let full = pool::check_for(tx, &a1, "api", None, None)?.expect("an ordinary run waits for the project's limit");
        assert!(full.words.contains("its limit"), "{}", full.words);
        assert_eq!(pool::check_for(tx, &a1, "api", Some(&ops::key_of(tx, &i)?), None)?, None, "the red batch's agent starts at once");
        Ok(())
    }).unwrap();
    pressed(&w);
    w.s.with(|tx| {
        let a1 = crate::tasks::agents::chosen(tx, "a1", "api")?;
        assert!(pool::check_for(tx, &a1, "api", None, None)?.is_some(), "its run counts against the project's limit as any run");
        Ok(())
    }).unwrap();
    assert_eq!(status(&w, &i), "IN PROGRESS");
}

fn queued_commit(w: &World, id: &str) -> String {
    w.s.with(|tx| {
        let key = ops::key_of(tx, id)?;
        tx.query_row("SELECT commit_sha FROM integration_entries WHERE task = ?1", [&key], |r| r.get(0)).map_err(sql)
    }).unwrap()
}

/// The target moved: `file` committed on origin's main, holding `text`; the new target.
fn target_moves(w: &World, file: &str, text: &str) -> String {
    sh(&w.api, &["checkout", "-q", "--detach", "origin/main"]);
    std::fs::write(w.api.join(file), text).unwrap();
    sh(&w.api, &["add", file]);
    sh(&w.api, &["commit", "-qm", "the target moves"]);
    sh(&w.api, &["push", "-q", "origin", "HEAD:main"]);
    sh(&w.api, &["checkout", "-q", "main"]);
    sh(&w.api, &["fetch", "-q", "origin"]);
    sh(&w.origin, &["rev-parse", "main"])
}

#[test]
fn ac194_red_again_the_same_session_hears_the_round_s_own_words() {
    let (w, a, b, i) = red_with_agent("ia-redagain", "exit 3", 3);
    let key = batch(&w).task.unwrap();
    pressed(&w);
    call(&w, recheck, json!({"why": "a port was held"})).unwrap();
    go(&w, &Sh::default(), 2);
    assert_eq!((batch(&w).state.as_str(), batch(&w).round), ("red", 2));
    let home = w.root.join("home");
    let log = integration::log_in(&home, "api", 1, 2, "check");
    assert!(log.exists(), "round 2's log kept");
    let (b2, ids) = (batch(&w), format!("{a}, {b}"));
    let words = format!("[terminum Dispatcher] {i}: batch 1's round 2 ended red: The check ended with status 3. Its log: {}. It checked {} with {ids}; your worktree is at it now. Decide as before.",
                        log.display(), short(b2.candidate.as_deref().unwrap()));
    assert_eq!(w.s.with(|tx| red_again(tx, &key, &home, None)).unwrap().unwrap(), words);
    assert_eq!(w.s.with(|tx| red_again(tx, &key, &home, Some(&[]))).unwrap().unwrap(), words, "asked, no answer yet: the round's words alone");
    let answered = w.s.with(|tx| red_again(tx, &key, &home, Some(&["keep API-14".to_string()]))).unwrap().unwrap();
    assert!(answered.starts_with(&words) && answered.ends_with("The human answered your questions, in the comments below. Take them into account.\n\nkeep API-14"), "{answered}");
    w.s.with(|tx| tx.execute("UPDATE integration_batches SET why = 'The check ran over 90 minutes.'", []).map(drop).map_err(sql)).unwrap();
    assert!(w.s.with(|tx| red_again(tx, &key, &home, None)).unwrap().unwrap().contains(&format!("round 2 ended red: The check ran over 90 minutes. Its log: {}. It checked", log.display())),
            "why it was red, and its log beside it");
    assert_eq!(w.s.with(|tx| red_again(tx, "no-such-key", &home, None)).unwrap(), None, "no batch's task: RUN's words");
}

#[test]
fn ac194_a_fix_is_picked_onto_the_target_as_it_is_when_its_round_is_built() {
    let (w, a, b, i) = red_with_agent("ia-fixmoved", "test -f fix.txt && test ! -f c.txt", 3);
    pressed(&w);
    let fix = commit_in(&agent_tree(&w), "fix.txt");
    fixing(&w, &fix).unwrap();
    assert_eq!(batch(&w).fixes, vec![fix.clone()], "the agent's own commit is kept, picked anew for each round");
    let moved = target_moves(&w, "z.txt", "z\n");
    call(&w, give_back, json!({"task": b, "analysis": ANALYSED})).unwrap();
    call(&w, recheck, json!({})).unwrap();
    go(&w, &Sh::default(), 6);
    assert_eq!((status(&w, &a), status(&w, &i)), ("DONE".into(), "DONE".into()));
    assert_eq!(on_origin(&w), "a.txt\nb.txt\nfix.txt\nz.txt");
    let log = sh(&w.origin, &["log", "--format=%P %s", "main"]);
    assert!(log.lines().any(|l| l == format!("{moved} the agent's fix")), "its change alone, on the moved target: {log}");
    assert!(log.contains(&format!("Integrate {i}")), "joined as the integration task's merge: {log}");
}

#[test]
fn ac194_a_fix_that_no_longer_applies_to_the_moved_target_is_left_out_and_said() {
    let (w, _, b, i) = red_with_agent("ia-fixgone", "exit 3", 3);
    pressed(&w);
    let fix = commit_in(&agent_tree(&w), "fix.txt");
    fixing(&w, &fix).unwrap();
    target_moves(&w, "fix.txt", "the target's own\n");
    call(&w, give_back, json!({"task": b, "analysis": ANALYSED})).unwrap();
    call(&w, recheck, json!({})).unwrap();
    go(&w, &Sh::default(), 1);
    let again = batch(&w);
    assert_eq!((again.round, again.fixes.len(), again.tasks.len()), (2, 0, 1), "built without its fix");
    assert!(w.s.with(|tx| ops::history(tx, &i)).unwrap().to_string().contains(&format!("its fix {} is left out on origin/main", short(&fix))));
}

#[test]
fn ac194_a_fix_must_be_its_own_on_the_candidate_and_change_something() {
    let (w, a, _, _) = red_with_agent("ia-fixown", "exit 3", 3);
    pressed(&w);
    let tree = agent_tree(&w);
    let task_commit = queued_commit(&w, &a);
    assert!(fixing(&w, &task_commit).unwrap_err().contains("is no commit of yours on the candidate"), "a queued task's commit is no fix");
    assert!(fixing(&w, batch(&w).candidate.as_deref().unwrap()).unwrap_err().contains("is no commit of yours"), "the candidate itself is no fix");
    target_moves(&w, "fix.txt", "fixed\n");
    std::fs::write(tree.join("fix.txt"), "fixed\n").unwrap();
    sh(&tree, &["add", "fix.txt"]);
    sh(&tree, &["commit", "-qm", "the agent's fix"]);
    // the round's target has not moved for it: it applies; once the target holds the same change, a rebuild says so
    let fix = sh(&tree, &["rev-parse", "HEAD"]);
    fixing(&w, &fix).unwrap();
    let b = batch(&w);
    let i = i_of(&w);
    call(&w, recheck, json!({})).unwrap();
    go(&w, &Sh::default(), 1);
    assert_eq!(batch(&w).fixes.len(), 0, "left out of round {}", b.round + 1);
    assert!(w.s.with(|tx| ops::history(tx, &i)).unwrap().to_string().contains(&format!("its fix {} is left out on origin/main at", short(&fix))));
    assert!(w.s.with(|tx| ops::history(tx, &i)).unwrap().to_string().contains("its change is already there"));
}
