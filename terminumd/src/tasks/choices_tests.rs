// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-230 and AC-157: recording a choice uses the existing human-answer path.

use super::*;
use crate::tasks::{closeout::ASKED, ops::NewTask, store::Store, tools::{self, Who}};

fn fixture(name: &str) -> (Store, std::path::PathBuf, Who, String) {
    let dir = crate::testutil::fresh_dir(name);
    let store = Store::open(dir.join("kanban.sqlite"));
    let who = Who { key: "tmux:local:terminum:%7".into(), agent: "claude".into(), title: "Testing".into(), project: None, dir: None, supervisor: false };
    let task = store.with(|tx| ops::create(tx, HUMAN, NewTask { title: "Choose API version".into(),
        session: Some(who.key.clone()), ..Default::default() }, 1)).unwrap().id;
    (store, dir, who, task)
}

fn args(task: &str) -> Value {
    json!({"id": task, "questions": ASKED, "kind": "operational",
           "options": [{"id": "retry", "label": "Retry"}, {"id": "wait", "label": "<b>Wait</b>"}], "recommended": "wait"})
}

#[test]
fn ac230_ask_human_refuses_invalid_fields_before_moving_or_adding_a_comment() {
    let (store, dir, who, task) = fixture("question-refusals");
    for (field, value, error) in [
        ("kind", json!("routine"), "kind must be operational or strategic."),
        ("options", json!("Retry"), "options must be an array"),
        ("options", json!([{"id": "wait"}]), "Each option must have a string id and label."),
        ("options", json!([{"id": "wait", "label": "One"}, {"id": "wait", "label": "Two"}]), "Option IDs and labels must be distinct."),
        ("options", json!([{"id": "wait", "label": " " }]), "Option labels must be"),
        ("options", json!([{"id": "wait", "label": "Two\nlines"}]), "Option labels must be plain, single-line text."),
        ("options", json!([{"id": "wait", "label": "Wait"}, {"id": "/start", "label": "Start"}]), "Option IDs must be"),
        ("options", json!([{"id": "wait", "label": "W".repeat(LABEL_MOST + 1)}]), "Option labels must be"),
        ("recommended", json!("absent"), "recommended must be an ID of a listed option."),
        ("recommended", Value::Null, "Questions with options require kind and recommended."),
        ("kind", Value::Null, "Questions with options require kind and recommended."),
    ] {
        let mut a = args(&task);
        a[field] = value;
        let refused = tools::run(&store, &dir, &who, "ask_human", &a).unwrap_err();
        assert!(refused.starts_with(error), "{field}: {refused}");
    }
    store.with(|tx| {
        assert_eq!(ops::get(tx, &task)?.status, "TODO");
        assert!(ops::history(tx, &task)?.as_array().unwrap().iter().all(|e| e["kind"] != "comment"));
        Ok(())
    }).unwrap();
}

#[test]
fn ac230_ac157_picks_once_and_free_text_uses_the_same_resume_path() {
    let (store, dir, who, task) = fixture("question-choices");
    let a = args(&task);
    tools::run(&store, &dir, &who, "ask_human", &a).unwrap();
    let event = store.with(|tx| {
        let t = ops::get(tx, &task)?;
        let question = current(tx, &t)?.unwrap();
        let e = ops::history(tx, &task)?.as_array().unwrap().iter().find(|e| e["id"] == question).unwrap().clone();
        assert_eq!(e["question"], json!({"kind": a["kind"], "options": a["options"], "recommended": a["recommended"]}));
        assert_eq!(e["text"], a["questions"]);
        Ok(question)
    }).unwrap();
    let choose = |event: &str, option: &str| store.with(|tx| answer(tx, &task, event, option, 10));
    assert_eq!(choose(&event, "missing").unwrap_err(), "That option is not on the question.");
    let (picked, kept) = choose(&event, "wait").unwrap();
    assert!(kept);
    assert_eq!(choose(&event, "wait").unwrap(), (picked.clone(), false));
    assert_eq!(choose(&event, "retry").unwrap_err(), "An option was already chosen for this question.");
    store.with(|tx| {
        assert!(ops::change_comment(tx, HUMAN, &task, &picked, Some("changed"), 10).is_err());
        let human = ops::comment(tx, HUMAN, &task, "Give it an hour.", 11)?;
        let history = ops::history(tx, &task)?;
        let e = history.as_array().unwrap().iter().find(|e| e["id"] == picked).unwrap();
        assert_eq!(e["author_kind"], "human");
        assert_eq!(e["choice"], json!({"question": event, "option": "wait"}));
        assert!(history.as_array().unwrap().iter().any(|e| e["id"] == human && e.get("choice").is_none()));
        assert_eq!(ops::answers(tx, &task)?.unwrap(), ["Selected option: wait.", "Give it an hour."]);
        assert_eq!(ops::get(tx, &task)?.status, "WAIT FOR HUMAN");
        ops::change_comment(tx, HUMAN, &task, &picked, None, 12)?;
        let scrubbed: bool = tx.query_row("SELECT json_extract(data, '$.choice') IS NULL FROM events WHERE id = ?1", [&picked], |r| r.get(0)).map_err(sql)?;
        assert!(scrubbed, "deletion removes the choice as well as the comment text");
        Ok(())
    }).unwrap();
    assert!(choose(&event, "retry").unwrap().1, "removing a choice lets the human choose again");

    tools::run(&store, &dir, &who, "ask_human", &a).unwrap();
    assert_eq!(choose(&event, "wait").unwrap_err(), STALE, "a later question supersedes it");
    let newest = store.with(|tx| {
        let t = ops::get(tx, &task)?;
        let next = current(tx, &t)?.unwrap();
        assert_eq!(ops::answers(tx, &task)?.unwrap(), Vec::<String>::new());
        ops::move_to(tx, HUMAN, &task, "IN PROGRESS", 14)?;
        ops::move_to(tx, HUMAN, &task, "WAIT FOR HUMAN", 15)?;
        Ok(next)
    }).unwrap();
    assert_eq!(choose(&newest, "wait").unwrap_err(), STALE, "moving back to waiting does not revive an old question");
}

#[test]
fn ac230_kind_can_be_recorded_without_making_a_free_text_question_selectable() {
    let (store, dir, who, task) = fixture("question-kind");
    tools::run(&store, &dir, &who, "ask_human", &json!({"id": task, "questions": ASKED, "kind": "strategic"})).unwrap();
    store.with(|tx| {
        let t = ops::get(tx, &task)?;
        assert_eq!(current(tx, &t)?, None);
        assert!(ops::history(tx, &task)?.as_array().unwrap().iter().any(|e| e["question"] == json!({"kind": "strategic"})));
        Ok(())
    }).unwrap();
}
