// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! An agent grooms its project's board (AC-154).

use serde_json::json;

use super::*;
use crate::tasks::ops::NewTask;
use crate::tasks::store::{Store, HUMAN};
use crate::tasks::tools::run;

const KEY: &str = "tmux:local:terminum:%7";
const OTHER: &str = "tmux:local:terminum:%8";
const SECTIONS: &str = "## Problem or goal\nx\n## Context\ny\n## Expected result\nz\n## Prerequisites\nNone";

fn who() -> Who {
    Who { key: KEY.into(), agent: "claude".into(), title: "api".into(), project: Some("api".into()), dir: None }
}

fn fresh(name: &str) -> (Store, std::path::PathBuf) {
    let dir = crate::testutil::fresh_dir(name);
    let s = Store::open(dir.join("kanban.sqlite"));
    s.with(|tx| { tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('api', 'api', 1, '', 0), ('web', 'web', 2, '', 1)", []).map_err(ops::sql)?; crate::tasks::slugs::fill(tx) }).unwrap();
    (s, dir)
}

fn call(s: &Store, dir: &std::path::Path, args: Value) -> Result<(String, Vec<String>), String> {
    run(s, dir, &who(), "update_task", &args)
}

/// A task an agent made, in project api.
fn agents_task(s: &Store, dir: &std::path::Path, title: &str) -> String {
    let said = run(s, dir, &who(), "create_task", &json!({"title": title, "severity": "P2", "text": SECTIONS})).unwrap().0;
    said.trim_start_matches("Kept as ").split(',').next().unwrap().to_string()
}

fn humans_task(s: &Store, project: &str) -> String {
    s.with(|tx| ops::create(tx, HUMAN, NewTask { title: "a human's".into(), project: Some(project.into()), ..Default::default() }, 1)).unwrap().id
}

fn task(s: &Store, id: &str) -> Task {
    s.with(|tx| ops::get(tx, id)).unwrap()
}

fn history(s: &Store, id: &str) -> String {
    s.with(|tx| ops::history(tx, id)).unwrap().to_string()
}

#[test]
fn ac154_severity_and_parent_of_an_agents_task_change_its_author_the_agent() {
    let (s, dir) = fresh("gr-move");
    let (a, parent, other) = (agents_task(&s, &dir, "Bug: a"), agents_task(&s, &dir, "Bug: parent"), agents_task(&s, &dir, "Bug: other"));
    let (said, cards) = call(&s, &dir, json!({"id": a, "severity": "P1", "parent": parent})).unwrap();
    assert_eq!(said, format!("{a} is changed."));
    assert_eq!(cards, vec![a.clone(), parent.clone()], "the parent's card changes too");
    let t = task(&s, &a);
    assert_eq!((t.severity.as_str(), t.parent.as_deref()), ("P1", Some(parent.as_str())));
    assert!(history(&s, &a).contains("claude · api set severity P1"));
    let (_, cards) = call(&s, &dir, json!({"id": a, "parent": other})).unwrap();
    assert_eq!(cards, vec![a.clone(), parent.clone(), other.clone()], "the parent it left and the one it joined");
    call(&s, &dir, json!({"id": a, "parent": "none"})).unwrap();
    assert_eq!(task(&s, &a).parent, None);
}

#[test]
fn ac154_a_humans_task_changes_only_with_its_reason() {
    let (s, dir) = fresh("gr-human");
    let h = humans_task(&s, "api");
    assert_eq!(call(&s, &dir, json!({"id": h, "severity": "P3"})).unwrap_err(), format!("{h} is a human's: give the reason for the change."));
    assert_eq!(task(&s, &h).severity, "P2", "nothing changed");
    call(&s, &dir, json!({"id": h, "severity": "P3", "reason": "only a log line"})).unwrap();
    assert_eq!(task(&s, &h).severity, "P3");
    assert!(history(&s, &h).contains("only a log line"), "the reason is its comment");
    let refused = call(&s, &dir, json!({"id": h, "title": "Bug: mine now", "reason": "clearer"})).unwrap_err();
    assert_eq!(refused, format!("{h} holds a human's words: they decide its title, text and archive. Comment instead."));
    assert_eq!(task(&s, &h).title, "a human's");
}

#[test]
fn ac154_words_change_only_on_an_agents_task_no_human_reworded_and_no_other_session_holds() {
    let (s, dir) = fresh("gr-words");
    let a = agents_task(&s, &dir, "Bug: first words");
    call(&s, &dir, json!({"id": a, "title": "Bug: better words", "text": SECTIONS.replace("\nx\n", "\nthe cause\n")})).unwrap();
    assert_eq!(task(&s, &a).title, "Bug: better words");
    assert_eq!(call(&s, &dir, json!({"id": a, "title": "better"})).unwrap_err(), "Start the title with Bug:, Feat: or Debt:.");
    assert!(call(&s, &dir, json!({"id": a, "text": "no sections"})).unwrap_err().starts_with("Not kept: the text lacks the sections"));
    s.with(|tx| ops::edit(tx, HUMAN, &a, ops::Edit { title: Some("Bug: the human's words".into()), ..Default::default() }, 2)).unwrap();
    assert!(call(&s, &dir, json!({"id": a, "title": "Bug: back"})).unwrap_err().contains("holds a human's words"));
    assert!(call(&s, &dir, json!({"id": a, "severity": "P0"})).is_ok(), "its severity is still the agents'");
    let purged = agents_task(&s, &dir, "Bug: a secret pasted");
    s.with(|tx| ops::purge_text(tx, HUMAN, &purged, 2)).unwrap();
    assert!(call(&s, &dir, json!({"id": purged, "text": SECTIONS})).unwrap_err().contains("holds a human's words"), "a purge is a human's word too");
    let older = agents_task(&s, &dir, "Bug: reworded before data.words");
    s.with(|tx| ops::log(tx, &ops::get(tx, &older)?.key, HUMAN, "edit", "Alex renamed it", None, 2).map(drop)).unwrap();
    assert!(call(&s, &dir, json!({"id": older, "title": "Bug: x"})).unwrap_err().contains("holds a human's words"), "an older event, by its text");
    let held = agents_task(&s, &dir, "Bug: held");
    s.with(|tx| ops::link(tx, HUMAN, &held, Some(OTHER), 3)).unwrap();
    assert_eq!(call(&s, &dir, json!({"id": held, "text": SECTIONS})).unwrap_err(), format!("{held} belongs to session %8: comment instead."));
}

#[test]
fn ac154_an_agents_task_leaves_the_board_with_its_reason_its_done_subtasks_with_it() {
    let (s, dir) = fresh("gr-archive");
    let (parent, kid) = (agents_task(&s, &dir, "Bug: parent"), agents_task(&s, &dir, "Bug: kid"));
    call(&s, &dir, json!({"id": kid, "parent": parent})).unwrap();
    assert_eq!(call(&s, &dir, json!({"id": parent, "archive": "yes"})).unwrap_err(), "Give the reason it leaves the board.");
    assert_eq!(call(&s, &dir, json!({"id": parent, "archive": "yes", "severity": "P1", "reason": "x"})).unwrap_err(), "archive goes alone, with its reason.");
    let open = call(&s, &dir, json!({"id": parent, "archive": "yes", "reason": "a duplicate"})).unwrap_err();
    assert_eq!(open, format!("{parent} has an open subtask, {kid}: finish it, archive it or take it out first."));
    s.with(|tx| ops::move_to(tx, HUMAN, &kid, "DONE", 4)).unwrap();
    let (said, cards) = call(&s, &dir, json!({"id": parent, "archive": "yes", "reason": "a duplicate of API-9"})).unwrap();
    assert_eq!(said, format!("{parent} is archived: a human can bring it back from the Archived list."));
    assert_eq!(cards, vec![kid.clone(), parent.clone()]);
    assert!(task(&s, &parent).archived_at.is_some() && task(&s, &kid).archived_at.is_some());
    assert!(history(&s, &parent).contains("Archived: a duplicate of API-9"));
    assert_eq!(call(&s, &dir, json!({"id": parent, "archive": "yes", "reason": "again"})).unwrap().0, format!("{parent} is archived already."));
}

#[test]
fn ac154_a_humans_task_or_another_project_never_leaves_the_board_by_an_agent() {
    let (s, dir) = fresh("gr-keep");
    let h = humans_task(&s, "api");
    s.with(|tx| ops::move_to(tx, HUMAN, &h, "DONE", 2)).unwrap();
    assert!(call(&s, &dir, json!({"id": h, "archive": "yes", "reason": "done"})).unwrap_err().contains("holds a human's words"), "a human's DONE too");
    let far = humans_task(&s, "web");
    assert_eq!(call(&s, &dir, json!({"id": far, "severity": "P0", "reason": "x"})).unwrap_err(), format!("{far} is outside this session's project."));
    let held = agents_task(&s, &dir, "Bug: held open");
    s.with(|tx| ops::link(tx, HUMAN, &held, Some(OTHER), 3)).unwrap();
    assert!(call(&s, &dir, json!({"id": held, "archive": "yes", "reason": "x"})).unwrap_err().contains("belongs to session %8"));
    s.with(|tx| ops::move_to(tx, HUMAN, &held, "DONE", 4)).unwrap();
    assert!(call(&s, &dir, json!({"id": held, "archive": "yes", "reason": "finished"})).is_ok(), "another session's agents' task, once DONE");
    assert_eq!(call(&s, &dir, json!({"id": h})).unwrap_err(), "Give severity, parent, title, text or archive.");
}
