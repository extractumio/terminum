// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! terminum-dispatcher.md AC-264: a report waits for its human.

use serde_json::{json, Value};

use super::*;
use crate::dispatcher::runs::{self, Phase, Run};
use crate::tasks::closeout;
use crate::tasks::ops::NewTask;
use crate::tasks::store::Store;
use crate::tasks::tools::{run, Who};

const KEY: &str = "tmux:local:terminum:%7";

fn fresh(name: &str) -> (Store, std::path::PathBuf) {
    let dir = crate::testutil::fresh_dir(name);
    let s = Store::open(dir.join("kanban.sqlite"));
    s.with(|tx| { tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('api', 'api', 1, '', 0)", []).map_err(sql)?; crate::tasks::slugs::fill(tx) }).unwrap();
    (s, dir)
}

fn who() -> Who {
    Who { key: KEY.into(), agent: "claude".into(), title: "api".into(), project: Some("api".into()), dir: None }
}

/// `titles`' tasks in api, the first the main task of the others, linked to KEY and IN PROGRESS in one run of
/// Dispatcher's there: their IDs.
fn in_a_run(s: &Store, titles: &[&str]) -> Vec<String> {
    s.with(|tx| {
        let mut made: Vec<ops::Task> = Vec::new();
        for title in titles {
            let parent = made.first().map(|m| m.id.clone());
            made.push(ops::create(tx, HUMAN, NewTask { title: (*title).into(), text: "body".into(), project: Some("api".into()), parent, ..Default::default() }, 1)?);
        }
        let keys: Vec<String> = made.iter().map(|t| t.key.clone()).collect();
        let r = runs::add(tx, &Run { agent_id: "a1".into(), agent_name: "Opus".into(), kind: "claude".into(), project: "api".into(), ..Default::default() }, &keys, Default::default(), 1)?;
        runs::change(tx, &r, |r| { r.state = Phase::Working; r.pane_key = Some(KEY.into()); r.agent_session = Some("c0ffee".into()); })?;
        for t in &made {
            ops::link(tx, HUMAN, &t.id, Some(KEY), 2)?;
            ops::move_to(tx, DISPATCHER, &t.id, "IN PROGRESS", 2)?;
        }
        Ok(made.into_iter().map(|t| t.id).collect())
    }).unwrap()
}

/// finish_task with a result in the close-out's sections, after `head`.
fn finish(s: &Store, dir: &std::path::Path, id: &str, more: Value) -> Result<String, String> {
    let result = closeout::SECTIONS.iter().fold("Outcome: Done".to_string(), |r, (name, _)| if *name == "Outcome" { r } else { format!("{r}\n{name}: None") });
    let mut args = json!({"id": id, "result": result});
    for (k, v) in more.as_object().into_iter().flatten() {
        args[k] = v.clone();
    }
    run(s, dir, &who(), "finish_task", &args).map(|(t, _)| t)
}

fn status(s: &Store, id: &str) -> String {
    s.with(|tx| ops::get(tx, id)).unwrap().status
}

fn moves(s: &Store, id: &str) -> Vec<String> {
    let h = s.with(|tx| ops::history(tx, id)).unwrap();
    h.as_array().unwrap().iter().filter(|e| e["text"].as_str().is_some_and(|t| t.contains(" moved it from "))).map(|e| e["text"].as_str().unwrap().to_string()).collect()
}

#[test]
fn ac264_a_report_waits_for_its_human_finished_for_its_run_and_done_reads_it() {
    let (s, dir) = fresh("report-waits");
    let id = in_a_run(&s, &["Audit the logs"]).remove(0);
    let said = finish(&s, &dir, &id, json!({"report": "yes"})).unwrap();
    assert_eq!(said, format!("{id} waits for the human to read your report."));
    assert_eq!(status(&s, &id), "WAIT FOR HUMAN");
    assert!(moves(&s, &id).last().unwrap().ends_with("moved it from IN PROGRESS to WAIT FOR HUMAN: a report to read"), "{:?}", moves(&s, &id));
    // for its run it is finished work, as DONE: AC-125 closes its session, nothing is left open in it
    let r = s.with(|tx| runs::last_of(tx, &ops::key_of(tx, &id)?)).unwrap().unwrap();
    assert_eq!((r.status.as_str(), r.open.len(), r.result), ("DONE", 0, true), "{r:?}");
    assert!(!s.with(|tx| runs::others_open(tx, KEY, "")).unwrap(), "it holds the session open for no other task");
    // no agent moves it: only the human's Done or RUN
    let refused = format!("{id} waits for the human to read its report: only their Done or RUN moves it. You can comment.");
    for (tool, args) in [("move_task", json!({"id": id, "to": "DONE"})), ("ask_human", json!({"id": id, "questions": closeout::ASKED})), ("wait_for_task", json!({"id": id, "on": "API-1", "why": "x"}))] {
        assert_eq!(run(&s, &dir, &who(), tool, &args).unwrap_err(), refused, "{tool}");
    }
    assert_eq!(finish(&s, &dir, &id, json!({})).unwrap_err(), refused, "finish_task again");
    assert!(run(&s, &dir, &who(), "add_comment", &json!({"id": id, "text": "a note"})).is_ok(), "a comment stays open to it");
    assert_eq!(status(&s, &id), "WAIT FOR HUMAN");
    s.with(|tx| read(tx, &id, ops::now() + 1)).unwrap();
    assert_eq!(status(&s, &id), "DONE");
    assert!(moves(&s, &id).last().unwrap().ends_with("moved it from WAIT FOR HUMAN to DONE: the report was read"), "{:?}", moves(&s, &id));
    assert_eq!(s.with(|tx| read(tx, &id, ops::now() + 2)).unwrap_err(), format!("{id} has no report waiting: a task moves by running it."), "read once");
}

#[test]
fn ac264_done_on_a_wait_that_is_no_report_is_refused_and_no_report_is_done() {
    let (s, dir) = fresh("report-refused");
    let ids = in_a_run(&s, &["Pick the API"]);
    let id = &ids[0];
    run(&s, &dir, &who(), "ask_human", &json!({"id": id, "questions": closeout::ASKED})).unwrap();
    assert_eq!(s.with(|tx| read(tx, id, 9)).unwrap_err(), format!("{id} has questions for you, not a report: answer them under Comment and press RUN."));
    assert_eq!(status(&s, id), "WAIT FOR HUMAN", "nothing moves");
    // a report that left its wait and came back another way is none
    s.with(|tx| { ops::move_to(tx, DISPATCHER, id, "IN PROGRESS", 10)?; ops::move_to(tx, DISPATCHER, id, "WAIT FOR HUMAN", 11) }).unwrap();
    assert!(!s.with(|tx| waits_on(tx, &ops::get(tx, id)?)).unwrap());
    let (s, dir) = fresh("report-none");
    let id = in_a_run(&s, &["Fix the loop"]).remove(0);
    assert_eq!(finish(&s, &dir, &id, json!({"report": "no"})).unwrap(), format!("{id} is DONE, with your result."), "another value is no report");
    assert_eq!(s.with(|tx| read(tx, &id, ops::now() + 1)).unwrap_err(), format!("{id} has no report waiting: a task moves by running it."));
    // a result refused for its sections waits for nothing
    let id = in_a_run(&s, &["Another"]).remove(0);
    assert!(run(&s, &dir, &who(), "finish_task", &json!({"id": id, "result": "Fixed it.", "report": "yes"})).unwrap().0.starts_with("Not a result"));
    assert_eq!(status(&s, &id), "IN PROGRESS");
}

#[test]
fn ac264_in_a_run_of_several_only_the_reports_wait() {
    let (s, dir) = fresh("report-several");
    let ids = in_a_run(&s, &["Main", "A report", "A fix"]);
    let (main, report, fix) = (&ids[0], &ids[1], &ids[2]);
    assert_eq!(finish(&s, &dir, report, json!({"report": "yes"})).unwrap(), format!("{report} waits for the human to read your report."));
    assert_eq!((status(&s, main), status(&s, fix)), ("IN PROGRESS".to_string(), "IN PROGRESS".to_string()), "nothing else waits with it");
    finish(&s, &dir, fix, json!({})).unwrap();
    assert_eq!(finish(&s, &dir, main, json!({})).unwrap(), format!("{main} is DONE, with your result."), "a waiting report is finished work for its main task");
    let r = s.with(|tx| runs::last_of(tx, &ops::key_of(tx, main)?)).unwrap().unwrap();
    assert_eq!(r.status, "DONE", "its session closes as AC-125 says");
    assert_eq!(status(&s, report), "WAIT FOR HUMAN");
    // the main task's next RUN takes no unread report again: its own RUN goes on with it
    assert!(s.with(|tx| crate::tasks::family::to_do(tx, &ops::key_of(tx, main)?)).unwrap().is_empty());
}
