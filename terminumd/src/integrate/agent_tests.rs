// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A red batch's integration agent (terminum-integration.md AC-194) on real repositories: the engine wants it in a
//! worktree of its own, its prompt holds the batch's facts alone, and its three tools decide only for its batch.
use std::path::{Path, PathBuf};

use serde_json::json;

use super::*;
use crate::integrate::engine::tests::{batch, go, on_origin, queued, sh, status, world, Sh, World};
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
    assert!(!PathBuf::from(format!("{}-fix", tree.display())).exists(), "the pick's worktree is gone");
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
        assert_eq!(kept.get("a2").cloned().unwrap_or_default(), vec!["integration:a2".to_string()]);
        let a2 = crate::tasks::agents::chosen(tx, "a2", "web")?;
        assert_eq!(crate::dispatcher::pool::check_for(tx, &a2, "web", Some(&t.key), None)?, None, "a1's run stands in a1's slot, not a2's");
        Ok(())
    }).unwrap();
    assert_eq!(status(&w, &i), "IN PROGRESS");
}
