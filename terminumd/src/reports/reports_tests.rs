// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! terminum-reports.md AC-265, AC-266, AC-269, AC-271, AC-272, AC-274.

use serde_json::{json, Value};

use super::inbox::{self, Filter};
use super::record::{self, Stamp};
use crate::dispatcher::runs::{self, Phase, Run};
use crate::tasks::closeout;
use crate::tasks::ops::{self, sql, NewTask};
use crate::tasks::store::{Store, DISPATCHER, HUMAN};
use crate::tasks::tools::{run, Who};

const KEY: &str = "tmux:local:terminum:%7";
const OTHER: &str = "tmux:local:terminum:%8";
const MANAGER: &str = "tmux:local:terminum:%9";

fn fresh(name: &str) -> (Store, std::path::PathBuf) {
    let dir = crate::testutil::fresh_dir(name);
    let s = Store::open(dir.join("kanban.sqlite"));
    s.with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place, prefix) VALUES ('api', 'API', 1, '', 0, 'API')", []).map_err(sql)?;
        // the human opened %9 in API: its manager (terminum-agents.md AC-174)
        tx.execute("INSERT INTO project_members (project, pane_key, place) VALUES ('api', ?1, 0)", [MANAGER]).map_err(sql)?;
        crate::tasks::slugs::fill(tx)
    }).unwrap();
    (s, dir)
}

fn who(key: &str) -> Who {
    Who { key: key.into(), agent: "claude".into(), title: "api".into(), project: Some("api".into()), dir: None, supervisor: false }
}

/// A task of API in a run of Dispatcher's in pane `key`, IN PROGRESS: its ID.
fn running(s: &Store, title: &str, key: &str) -> String {
    s.with(|tx| {
        let t = ops::create(tx, HUMAN, NewTask { title: title.into(), text: "body".into(), project: Some("api".into()), ..Default::default() }, 1)?;
        let r = runs::add(tx, &Run { agent_id: "a1".into(), agent_name: "Opus".into(), kind: "claude".into(), project: "api".into(), ..Default::default() }, &[t.key.clone()], Default::default(), 1)?;
        runs::change(tx, &r, |r| { r.state = Phase::Working; r.pane_key = Some(key.into()); })?;
        ops::link(tx, HUMAN, &t.id, Some(key), 2)?;
        ops::move_to(tx, DISPATCHER, &t.id, "IN PROGRESS", 2)?;
        Ok(t.id)
    }).unwrap()
}

fn call(s: &Store, dir: &std::path::Path, key: &str, tool: &str, args: Value) -> Result<String, String> {
    run(s, dir, &who(key), tool, &args).map(|(t, _)| t)
}

fn report(title: &str) -> Value {
    json!({"title": title, "summary": "2 outdated packages", "body": "## Findings\n- serde"})
}

/// A routine's result in its own form (terminum-schedules.md AC-162).
const ROUTINE: &str = "Outcome: ok, 2 outdated\nSummary: s\nDetails: d\nFor the human: nothing";

/// A result in every section of the close-out (terminum-dispatcher.md AC-123).
fn result() -> String {
    closeout::SECTIONS.iter().fold("Outcome: Done, all checked".to_string(), |r, (name, _)| if *name == "Outcome" { r } else { format!("{r}\n{name}: None") })
}

fn one(s: &Store, id: &str) -> Value {
    s.with(|tx| inbox::detail(tx, &record::key_of(tx, id)?)).unwrap()
}

#[test]
fn ac265_ac266_a_worker_reports_on_its_task_in_the_project_s_count() {
    let (s, dir) = fresh("reports-made");
    let task = running(&s, "Audit the deps", KEY);
    let said = call(&s, &dir, KEY, "create_report", report("Nightly deps check")).unwrap();
    let id = said.split_whitespace().next().unwrap().to_string();
    assert_eq!(said, format!("{id} is in the human's Inbox. Finish {task} with finish_task as usual: the report is not its result."));
    assert!(id.starts_with("R_API-"), "{id}");
    let r = one(&s, &id);
    assert_eq!((r["source"]["task"].as_str(), r["read"].as_bool(), r["critical"].as_bool(), r["role"].as_str()), (Some(task.as_str()), Some(false), Some(false), Some("worker")));
    assert_eq!((r["summary"].as_str(), r["body"].as_str()), (Some("2 outdated packages"), Some("## Findings\n- serde")));
    // the task links it, and the next task's number comes after the report's: one count, no ID shared
    let history = s.with(|tx| ops::history(tx, &task)).unwrap();
    assert!(history.as_array().unwrap().iter().any(|e| e["text"] == format!("Report {id}: Nightly deps check")), "{history}");
    let next = s.with(|tx| ops::create(tx, HUMAN, NewTask { title: "Next".into(), project: Some("api".into()), ..Default::default() }, 3)).unwrap();
    let n = |id: &str| id.rsplit('-').next().unwrap().parse::<i64>().unwrap();
    assert_eq!(n(&next.id), n(&id) + 1);
    // a report's ID is no task's
    assert_eq!(call(&s, &dir, KEY, "get_task", json!({"id": id})).unwrap_err(), format!("{id} is a report: use get_report."));
    // finish_task then moves the task to DONE as any
    assert!(call(&s, &dir, KEY, "finish_task", json!({"id": task, "result": result()})).unwrap().starts_with(&format!("{task} is DONE, with your result.")));
}

#[test]
fn ac265_limits_are_said_in_words() {
    let (s, dir) = fresh("reports-limits");
    running(&s, "Audit", KEY);
    let made = |v: Value| call(&s, &dir, KEY, "create_report", v);
    assert_eq!(made(json!({"title": " ", "summary": "s", "body": "b"})).unwrap_err(), "Title: 1–200 characters");
    assert_eq!(made(json!({"title": "t".repeat(201), "summary": "s", "body": "b"})).unwrap_err(), "Title: 1–200 characters");
    assert_eq!(made(json!({"title": "t", "summary": "s".repeat(501), "body": "b"})).unwrap_err(), "Summary: at most 500 characters");
    assert_eq!(made(json!({"title": "t", "summary": "s", "body": "b".repeat((1 << 20) + 1)})).unwrap_err(), "Body: at most 1 MiB");
    assert_eq!(s.with(|tx| inbox::list(tx, &Filter::default())).unwrap()["reports"], json!([]), "nothing kept");
}

#[test]
fn ac266_a_worker_writes_only_to_its_own_and_is_bounded() {
    let (s, dir) = fresh("reports-worker");
    let mine = running(&s, "Mine", KEY);
    let theirs = running(&s, "Theirs", OTHER);
    let id = call(&s, &dir, OTHER, "create_report", report("Theirs")).unwrap().split_whitespace().next().unwrap().to_string();
    assert_eq!(call(&s, &dir, KEY, "append_to_report", json!({"id": id, "text": "mine"})).unwrap_err(), format!("{id} is not of your task or session: you can read it with get_report."));
    assert!(call(&s, &dir, KEY, "get_report", json!({"id": id})).unwrap().contains("\"title\":\"Theirs\""), "any caller reads");
    assert_eq!(call(&s, &dir, KEY, "create_report", json!({"task": theirs, "title": "t", "summary": "s", "body": "b"})).unwrap_err(), format!("{theirs} is not a task of this session: report on your own task, or name none."));
    assert_eq!(call(&s, &dir, KEY, "create_report", json!({"task": mine, "follows": id, "title": "t", "summary": "s", "body": "b"})).unwrap_err(), format!("{id} is not of your task or session: follow only your own reports."));
    assert_eq!(call(&s, &dir, KEY, "update_report", json!({"id": id, "title": "x"})).unwrap_err(), crate::roles::not_yours("update_report"));
    assert_eq!(call(&s, &dir, KEY, "delete_report", json!({"id": id})).unwrap_err(), crate::roles::not_yours("delete_report"));
    // its own: appended below, read state kept
    let own = call(&s, &dir, KEY, "create_report", report("Mine")).unwrap().split_whitespace().next().unwrap().to_string();
    assert_eq!(call(&s, &dir, KEY, "append_to_report", json!({"id": own, "text": "One more thing"})).unwrap(), format!("Your comment stands below {own}."));
    assert_eq!(one(&s, &own)["notes"][0]["text"], "One more thing");
    // at most 10 an hour
    for n in 0..9 {
        call(&s, &dir, KEY, "create_report", report(&format!("R{n}"))).unwrap();
    }
    assert_eq!(call(&s, &dir, KEY, "create_report", report("Eleventh")).unwrap_err(), "This session made 10 reports in the last hour: put more in one report, or append to it.");
}

#[test]
fn ac266_with_several_tasks_the_report_names_one() {
    let (s, dir) = fresh("reports-several");
    let a = running(&s, "A", KEY);
    let b = running(&s, "B", KEY);
    assert_eq!(call(&s, &dir, KEY, "create_report", report("Which")).unwrap_err(), format!("Name the task this report is for: {a} or {b}."));
    assert!(call(&s, &dir, OTHER, "create_report", report("No task")).unwrap().ends_with("is in the human's Inbox."), "a session with no task: no source task");
}

#[test]
fn ac266_a_manager_changes_and_deletes_reports_of_its_project() {
    let (s, dir) = fresh("reports-manager");
    running(&s, "Audit", KEY);
    let id = call(&s, &dir, KEY, "create_report", report("Draft")).unwrap().split_whitespace().next().unwrap().to_string();
    assert_eq!(call(&s, &dir, MANAGER, "update_report", json!({"id": id, "title": "Final", "critical": "yes"})).unwrap(), format!("{id} is changed."));
    let r = one(&s, &id);
    assert_eq!((r["title"].as_str(), r["critical"].as_bool()), (Some("Final"), Some(true)));
    assert_eq!(s.with(record::lines).unwrap().len(), 1, "made critical while unread: its line comes");
    assert_eq!(call(&s, &dir, MANAGER, "update_report", json!({"id": id})).unwrap_err(), "Give title, summary, body or critical to change.");
    assert!(call(&s, &dir, MANAGER, "delete_report", json!({"id": id})).unwrap().starts_with(&format!("{id} is deleted")));
    assert_eq!(s.with(|tx| inbox::list(tx, &Filter::default())).unwrap()["reports"], json!([]));
    assert_eq!(s.with(|tx| inbox::list(tx, &Filter { deleted: true, ..Filter::default() })).unwrap()["reports"][0]["id"], json!(id), "restorable");
}

#[test]
fn ac266_finish_task_takes_no_report() {
    let (s, dir) = fresh("reports-finish");
    let task = running(&s, "Audit", KEY);
    assert_eq!(call(&s, &dir, KEY, "finish_task", json!({"id": task, "result": result(), "report": "yes"})).unwrap_err(), crate::tasks::finish::GONE_REPORT);
    assert_eq!(s.with(|tx| ops::get(tx, &task)).unwrap().status, "IN PROGRESS", "nothing moved");
}

#[test]
fn ac269_read_unread_deleted_restored_and_purged_after_30_days() {
    let (s, dir) = fresh("reports-state");
    running(&s, "Audit", KEY);
    let id = call(&s, &dir, KEY, "create_report", report("Read me")).unwrap().split_whitespace().next().unwrap().to_string();
    assert_eq!(call(&s, &dir, KEY, "attach_report_file", json!({"id": id, "name": "table.csv", "text": "a,b"})).unwrap(), format!("table.csv is on {id}."));
    let file = one(&s, &id)["files"][0]["id"].as_str().unwrap().to_string();
    let key = s.with(|tx| record::key_of(tx, &id)).unwrap();
    s.with(|tx| {
        let rev = record::revision(tx)?;
        assert!(record::stamp(tx, &key, Stamp::Read, Some(10))? && !record::stamp(tx, &key, Stamp::Read, Some(11))?, "once");
        assert_ne!(record::revision(tx)?, rev, "every page hears it by the Inbox's revision");
        assert_eq!(inbox::unread(tx)?["projects"], json!({}));
        assert!(record::stamp(tx, &key, Stamp::Read, None)?);
        assert_eq!(inbox::unread(tx)?["projects"], json!({"api": 1}));
        assert!(record::stamp(tx, &key, Stamp::Deleted, Some(13))?);
        assert_eq!(inbox::unread(tx)?["projects"], json!({}), "a deleted report counts nowhere");
        assert!(record::stamp(tx, &key, Stamp::Deleted, None)?);
        record::stamp(tx, &key, Stamp::Deleted, Some(100))?;
        assert!(record::purge(tx, 100 + record::KEPT_DELETED - 1)?.is_empty(), "kept 30 days");
        assert_eq!(record::purge(tx, 101 + record::KEPT_DELETED)?, vec![file], "then it goes with its files");
        assert_eq!(record::key_of(tx, &id).unwrap_err(), format!("No report {id}."));
        Ok(())
    }).unwrap();
    assert_eq!(crate::tasks::files::sweep(&s, &dir, 5).unwrap(), 0);
}

#[test]
fn ac271_only_a_critical_report_raises_a_line_once_an_hour_per_worker_session() {
    let (s, dir) = fresh("reports-lines");
    running(&s, "Audit", KEY);
    call(&s, &dir, KEY, "create_report", report("Quiet")).unwrap();
    assert!(s.with(record::lines).unwrap().is_empty(), "no alert for a report not critical");
    let mut loud = report("Build broken");
    loud["critical"] = json!("yes");
    let first = call(&s, &dir, KEY, "create_report", loud.clone()).unwrap().split_whitespace().next().unwrap().to_string();
    call(&s, &dir, KEY, "create_report", loud).unwrap();
    let lines = s.with(record::lines).unwrap();
    assert_eq!(lines, vec![(first.clone(), "api".to_string(), format!("{first} critical: Build broken"))], "the second stands red without a line");
    // critical unread first in the Inbox
    assert_eq!(s.with(|tx| inbox::list(tx, &Filter::default())).unwrap()["reports"][0]["critical"], json!(true));
    let key = s.with(|tx| record::key_of(tx, &first)).unwrap();
    s.with(|tx| record::stamp(tx, &key, Stamp::Read, Some(ops::now()))).unwrap();
    assert!(s.with(record::lines).unwrap().is_empty(), "read: its line goes");
    // an incident's report alerts through the incident alone
    s.with(|tx| super::sources::incident(tx, "api", 12, "task API-1", "cost over its bound", 20).map(drop)).unwrap();
    assert!(s.with(record::lines).unwrap().is_empty());
}

#[test]
fn ac272_a_routine_s_result_becomes_a_report_unless_its_run_made_one() {
    let (s, dir) = fresh("reports-routine");
    let routine = |title: &str, key: &str| -> String {
        let id = running(&s, title, key);
        s.with(|tx| {
            tx.execute("INSERT INTO agents (id, project, name, kind, command, color, at_once) VALUES ('a', 'api', 'Opus', 'claude', 'claude', 0, 2) ON CONFLICT DO NOTHING", []).map_err(sql)?;
            let given = crate::tasks::schedules::Given::of(tx, &json!({"title": title, "body": "b", "agent": "a", "cron": "0 3 * * *", "budget_minutes": 30, "severity": "P2", "project": "api"}), 1)?;
            let (sched, _) = crate::tasks::schedules::add(tx, HUMAN, &given, 1)?;
            tx.execute("INSERT INTO schedule_fires (schedule, due_at, at, task, state) VALUES (?1, 1, 1, ?2, 'started')", (&sched, ops::key_of(tx, &id)?)).map_err(sql)?;
            Ok(id)
        }).unwrap()
    };
    let task = routine("Nightly deps check", KEY);
    let said = call(&s, &dir, KEY, "finish_task", json!({"id": task, "result": ROUTINE})).unwrap();
    let id = said.split("Inbox as ").nth(1).expect(&said).split('.').next().unwrap().to_string();
    assert!(said.starts_with(&format!("{task} is DONE; its result went to the human's Inbox as {id}.")), "{said}");
    let r = one(&s, &id);
    assert_eq!((r["summary"].as_str(), r["source"]["task"].as_str(), r["source"]["name"].as_str()), (Some("ok, 2 outdated"), Some(task.as_str()), Some("Nightly deps check")));
    assert_eq!(s.with(|tx| ops::get(tx, &task)).unwrap().status, "DONE");
    // its own report in the run: no second one
    let task = routine("Weekly audit", OTHER);
    call(&s, &dir, OTHER, "create_report", report("Weekly audit")).unwrap();
    assert!(call(&s, &dir, OTHER, "finish_task", json!({"id": task, "result": ROUTINE})).unwrap().starts_with(&format!("{task} is DONE, with your result.")));
    assert_eq!(s.with(|tx| inbox::list(tx, &Filter::default())).unwrap()["reports"].as_array().unwrap().len(), 2, "none superseded, none doubled");
}

#[test]
fn ac274_a_waiting_report_moves_to_the_inbox_once() {
    let (s, _) = fresh("reports-move");
    let task = running(&s, "Audit the logs", KEY);
    s.with(|tx| {
        ops::comment(tx, "dispatcher", &task, "Outcome: attention\nTwo errors a night", 5)?;
        ops::move_marked(tx, DISPATCHER, &task, "WAIT FOR HUMAN", Some(("report", "a report to read")), 5)?;
        let asks = running_in(tx)?;
        let moved = super::migrate::waiting_reports(tx)?;
        assert_eq!(moved.len(), 1);
        assert_eq!(ops::get(tx, &task)?.status, "DONE");
        assert_eq!(ops::get(tx, &asks)?.status, "WAIT FOR HUMAN", "a task with questions stays");
        let r = inbox::detail(tx, &record::key_of(tx, &moved[0])?)?;
        assert_eq!((r["summary"].as_str(), r["body"].as_str(), r["read"].as_bool(), r["created_at"].as_i64()), (Some("attention"), Some("Outcome: attention\nTwo errors a night"), Some(false), Some(5)));
        let history = ops::history(tx, &task)?;
        assert!(history.as_array().unwrap().iter().any(|e| e["text"] == format!("Dispatcher moved it from WAIT FOR HUMAN to DONE: its report went to the Inbox as {}", moved[0])), "{history}");
        assert!(super::migrate::waiting_reports(tx)?.is_empty(), "once");
        Ok(())
    }).unwrap();
}

/// A task in WAIT FOR HUMAN with an agent's question, as AC-157 leaves it: its ID.
fn running_in(tx: &rusqlite::Transaction) -> Result<String, String> {
    let t = ops::create(tx, HUMAN, NewTask { title: "Which version?".into(), project: Some("api".into()), ..Default::default() }, 6)?;
    ops::move_to(tx, DISPATCHER, &t.id, "WAIT FOR HUMAN", 6)?;
    Ok(t.id)
}

#[test]
fn ac265_a_deleted_project_s_reports_move_to_default_keeping_their_ids() {
    let (s, dir) = fresh("reports-project-gone");
    running(&s, "Audit", KEY);
    let id = call(&s, &dir, KEY, "create_report", report("Kept")).unwrap().split_whitespace().next().unwrap().to_string();
    s.with(|tx| crate::tasks::projects::move_tasks_to_default(tx, "api", "API", 9).map(drop)).unwrap();
    assert_eq!(one(&s, &id)["project"], json!(crate::tasks::store::DEFAULT_PROJECT), "{id} keeps its ID in Default");
}
