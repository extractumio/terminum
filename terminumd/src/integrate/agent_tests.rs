// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A red batch's integration agent (terminum-integration.md AC-194) on real repositories: the engine wants it in a
//! worktree of its own, its prompt holds the batch's facts alone, and its three tools decide only for its batch.
use serde_json::json;

use super::*;
use crate::integrate::engine::tests::{batch, go, on_origin, queued, sh, status, world, Sh, World};
use std::path::{Path, PathBuf};

use crate::tasks::integration::Settings;
use crate::tasks::ops::INTEGRATION;
use crate::tasks::store::HUMAN;
use crate::tasks::tools::Who;

const PANE: &str = "tmux:local:terminum:%9";

fn who(key: &str) -> Who {
    Who { key: key.into(), agent: "claude".into(), title: "api".into(), project: Some("api".into()), dir: None }
}

/// Api's queue with Opus reviewer as its integration agent, `check` its Check, two tasks queued and their batch red.
fn red_with_agent(name: &str, check: &str, red_rounds_most: i64) -> (World, String, String, String) {
    let w = world(name, Settings { check: check.into(), ..Settings::default() });
    w.s.with(|tx| {
        tx.execute("INSERT INTO agents (id, project, name, kind, command, color) VALUES ('a1', 'api', 'Opus reviewer', 'claude', 'claude', 0)", []).map_err(sql)?;
        let s = integration::read(tx, "api")?;
        integration::keep(tx, "api", &Settings { agent: Some("a1".into()), red_rounds_most, ..s })
    }).unwrap();
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

/// What start_agents and the press do: the agent pressed, its task IN PROGRESS, its run in PANE.
fn pressed(w: &World) -> String {
    let (b, i) = (batch(w), i_of(w));
    w.s.with(|tx| {
        set_agent_state(tx, b.id, Some("pressed"))?;
        let t = ops::get(tx, &i)?;
        ops::place(tx, DISPATCHER, &t, "IN PROGRESS", 200)
    }).unwrap();
    crate::tasks::tools::tests::running(&w.s, &i, PANE);
    i
}

fn call(w: &World, tool: fn(&Transaction, &Who, &Value, i64) -> Result<String, String>, args: Value) -> Result<String, String> {
    w.s.with(|tx| tool(tx, &who(PANE), &args, 300))
}

const ANALYSED: &str = "Failing step: check\nEvidence: exit 3\nCommit and lines: c.txt:1\nWhat to fix: the test";

#[test]
fn ac194_red_wants_its_agent_in_a_worktree_of_its_own_at_the_candidate() {
    let w = world("ia-want", Settings { check: "exit 3".into(), ..Settings::default() });
    w.s.with(|tx| {
        tx.execute("INSERT INTO agents (id, project, name, kind, command, color) VALUES ('a1', 'api', 'Opus reviewer', 'claude', 'claude', 0)", []).map_err(sql)?;
        let s = integration::read(tx, "api")?;
        integration::keep(tx, "api", &Settings { agent: Some("a1".into()), agent_budget_minutes: 45, ..s })
    }).unwrap();
    let a = queued(&w, 1, "b.txt", "b\n");
    w.s.with(|tx| ops::comment(tx, HUMAN, &a, "words of the task's own, never the agent's", 3).map(drop)).unwrap();
    go(&w, &Sh::default(), 4);
    let b = batch(&w);
    let i = i_of(&w);
    assert_eq!(w.s.with(wanted).unwrap(), vec![(b.id, i.clone(), "a1".to_string(), 45)]);
    let tree = w.s.with(|tx| tree_of_task(tx, b.task.as_deref().unwrap())).unwrap().expect("its worktree");
    assert!(tree.ends_with("agent-1"), "{tree}");
    assert_eq!(sh(Path::new(&tree), &["rev-parse", "HEAD"]), b.candidate.clone().unwrap(), "at the candidate");
    let prompt = w.s.with(|tx| prompt_of(tx, b.task.as_deref().unwrap())).unwrap().unwrap();
    for fact in [i.as_str(), "exit 3", &tree, &a, "never call finish_task", "45 minutes"] {
        assert!(prompt.contains(fact), "{fact} in {prompt}");
    }
    assert!(!prompt.contains("words of the task's own"), "no task's words in its prompt");
    assert!(w.s.with(|tx| finish_refused(tx, b.task.as_deref().unwrap())).unwrap().unwrap().contains("ends when its batch 1 does"));
}

#[test]
fn ac194_without_an_agent_or_with_its_red_rounds_spent_a_human_decides() {
    let (w, _, _, i) = red_with_agent("ia-spent", "exit 3", 1);
    assert_eq!(status(&w, &i), "WAIT FOR HUMAN");
    assert!(w.s.with(wanted).unwrap().is_empty(), "no agent started");
}

#[test]
fn ac194_several_go_back_before_one_recheck_and_the_rest_is_built_anew() {
    let w = world("ia-return", Settings { check: "exit 3".into(), ..Settings::default() });
    w.s.with(|tx| {
        tx.execute("INSERT INTO agents (id, project, name, kind, command, color) VALUES ('a1', 'api', 'Opus reviewer', 'claude', 'claude', 0)", []).map_err(sql)?;
        let s = integration::read(tx, "api")?;
        integration::keep(tx, "api", &Settings { agent: Some("a1".into()), ..s })
    }).unwrap();
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
    assert_eq!(w.s.with(wanted).unwrap().len(), 1, "red again: its agent once more");
    pressed(&w);
    assert_eq!(call(&w, recheck, json!({"why": "again"})).unwrap_err(), "Batch 1 was checked again unchanged once: return a task, add a fix, or ask a human.");
}

#[test]
fn ac194_a_fix_of_its_own_joins_the_candidate_first_and_is_published() {
    let (w, a, b, i) = red_with_agent("ia-fix", "test -f fix.txt", 3);
    pressed(&w);
    let key = batch(&w).task.unwrap();   // read outside: the store's lock is not taken twice
    let tree = PathBuf::from(w.s.with(|tx| tree_of_task(tx, &key)).unwrap().unwrap());
    std::fs::write(tree.join("fix.txt"), "fixed\n").unwrap();
    sh(&tree, &["add", "fix.txt"]);
    sh(&tree, &["commit", "-qm", "the target's fix"]);
    let fix = sh(&tree, &["rev-parse", "HEAD"]);
    let fixing = |c: &str| add_fix(&w.s, &who(PANE), &json!({"commit": c}), 300);
    assert!(fixing("--upload-pack=x").unwrap_err().contains("hex digits"));
    assert!(fixing("abcdef1234").unwrap_err().contains("is no commit in your worktree"));
    fixing(&fix).unwrap();
    assert_eq!(sh(&tree, &["rev-parse", "HEAD"]), fix, "its worktree stands where it stood");
    call(&w, recheck, json!({})).unwrap();
    go(&w, &Sh::default(), 6);
    assert_eq!((status(&w, &a), status(&w, &b), status(&w, &i)), ("DONE".into(), "DONE".into(), "DONE".into()));
    assert_eq!(on_origin(&w), "a.txt\nb.txt\nc.txt\nfix.txt");
    assert!(!tree.exists(), "its worktree removed when its batch ended");
}

#[test]
fn ac194_the_integration_agent_keeps_a_slot() {
    let (w, _, _, _) = red_with_agent("ia-slot", "exit 3", 3);
    let kept = w.s.with(crate::tasks::schedules::kept_by_agent).unwrap();
    assert_eq!(kept.get("a1").cloned().unwrap_or_default(), vec![crate::tasks::schedules::INTEGRATION_SLOT.to_string()]);
}
