// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Who is a manager and what each role may run (terminum-agents.md AC-174 to AC-181).

use serde_json::json;

use super::*;
use crate::dispatcher::runs::{Phase, Run};
use crate::tasks::ops::{self, sql, NewTask};
use crate::tasks::store::HUMAN;

const BOSS: &str = "tmux:local:t:%1";
const WORKER: &str = "tmux:local:t:%2";
const PLAIN: &str = "tmux:local:t:%3";

/// A run of the pool's agent `a` in pane `key`, working: + Session's with no `task`.
fn run_in(tx: &Transaction, key: &str, project: &str, task: Option<&str>) -> String {
    let like = Run { agent_id: "a1".into(), agent_name: "Opus".into(), kind: "claude".into(), command: "claude".into(), folder: "/w".into(), project: project.into(), ..Default::default() };
    let keys: Vec<String> = task.map(|t| ops::key_of(tx, t).unwrap()).into_iter().collect();
    let id = runs::add(tx, &like, &keys, Default::default(), 2).unwrap();
    runs::change(tx, &id, |r| { r.state = Phase::Working; r.pane_key = Some(key.into()); }).unwrap();
    id
}

fn task_in(tx: &Transaction, project: &str, title: &str) -> String {
    ops::create(tx, HUMAN, NewTask { title: title.into(), text: "body".into(), project: Some(project.into()), ..Default::default() }, 1).unwrap().id
}

/// An app with projects api and web, a pool agent, a manager in BOSS (api), a worker's run in WORKER.
fn app(tag: &str) -> (crate::app::Shared, String) {
    let (_, app) = crate::testutil::app_with_tasks(tag);
    let task = app.tasks.with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('api', 'api', 1, '', 0), ('web', 'web', 2, '', 1)", []).map_err(sql)?;
        crate::tasks::slugs::fill(tx)?;
        tx.execute("INSERT INTO agents (id, name, kind, command, effort, color, at_once) VALUES ('a1', 'Opus', 'claude', 'claude', '', 0, 2)", []).map_err(sql)?;
        run_in(tx, BOSS, "api", None);
        let task = task_in(tx, "api", "worked on");
        run_in(tx, WORKER, "api", Some(&task));
        Ok(task)
    }).unwrap();
    (app, task)
}

async fn call(app: &crate::app::Shared, key: &str, tool: &str, args: Value) -> Result<String, String> {
    in_pane(app, key, true, false, tool, &args).await
}

#[tokio::test]
async fn ac174_only_a_plus_session_s_session_is_its_project_s_manager() {
    let (app, _) = app("roles-who");
    let role = |key: &'static str| app.tasks.with(move |tx| Ok(boss(tx, key)?.map(|b| (b.project, b.name)))).unwrap();
    assert_eq!(role(BOSS), Some(("api".into(), "Opus · manager".into())), "+ Session's run: its project's manager");
    assert_eq!(role(WORKER), None, "a run with a task is a worker, whoever started it");
    assert_eq!(role(PLAIN), None, "a pane with no run is a worker: a new window gives no role");
    // let go, its session is no manager any more
    app.tasks.with(|tx| { let r = runs::in_pane(tx, BOSS)?.unwrap(); runs::let_go(tx, &r.id).map(drop) }).unwrap();
    assert_eq!(role(BOSS), None);
}

#[tokio::test]
async fn ac174_a_worker_neither_finds_nor_runs_a_manager_s_tool() {
    let (app, _) = app("roles-refuse");
    for key in [WORKER, PLAIN] {
        for tool in MANAGER_TOOLS {
            assert_eq!(call(&app, key, tool, json!({})).await.unwrap_err(), not_yours(tool), "{key} {tool}");
        }
    }
    assert!(call(&app, BOSS, "list_agents", json!({})).await.unwrap().contains("\"name\":\"Opus\""));
}

#[tokio::test]
async fn ac178_a_manager_s_call_comes_from_its_pane_and_its_main_agent() {
    let (app, _) = app("roles-pane");
    let not_from = in_pane(&app, BOSS, false, false, "list_agents", &json!({})).await.unwrap_err();
    assert!(not_from.starts_with("Not done: this call does not come from the manager's own pane"), "{not_from}");
    // a subagent reads, never writes
    assert!(in_pane(&app, BOSS, true, true, "list_agents", &json!({})).await.is_ok());
    assert_eq!(in_pane(&app, BOSS, true, true, "delete_agent", &json!({"agent": "Opus"})).await.unwrap_err(), SUBAGENT);
    assert!(app.tasks.with(|tx| crate::tasks::agents::by_id(tx, "a1")).unwrap().is_some(), "nothing was done");
}

#[tokio::test]
async fn ac179_a_manager_runs_and_stops_only_its_project_s_tasks() {
    let (app, task) = app("roles-run");
    let far = app.tasks.with(|tx| Ok(task_in(tx, "web", "another project's"))).unwrap();
    assert_eq!(call(&app, BOSS, "run_task", json!({"id": far, "agent": "Opus"})).await.unwrap_err(), format!("{far} is outside your project."));
    assert_eq!(call(&app, BOSS, "stop_task", json!({"id": far})).await.unwrap_err(), format!("{far} is outside your project."));
    let no_agent = call(&app, BOSS, "run_task", json!({"id": task, "agent": "Nobody"})).await.unwrap_err();
    assert_eq!(no_agent, "The pool has no agent \"Nobody\": Opus.");
    // what runs in its project, a worker's run and its own session
    let runs = call(&app, BOSS, "list_runs", json!({})).await.unwrap();
    assert!(runs.starts_with(DATA) && runs.contains(&format!("\"tasks\":[\"{task}\"]")) && runs.contains("\"kind\":\"session\""), "{runs}");
    // stop: the task fails, its run is let go, and the history names the manager
    let stopped = call(&app, BOSS, "stop_task", json!({"id": task})).await.unwrap();
    assert_eq!(stopped, format!("Stopped: {task}. Its window stays; its slot is free."));
    let (status, history) = app.tasks.with(|tx| Ok((ops::get(tx, &task)?.status, ops::history(tx, &task)?.to_string()))).unwrap();
    assert_eq!(status, "FAILED");
    assert!(history.contains("Opus · manager stopped it."), "{history}");
}

#[tokio::test]
async fn ac180_a_manager_keeps_the_pool_by_the_board_s_rules() {
    let (app, _) = app("roles-agents");
    let made = call(&app, BOSS, "create_agent", json!({"name": "Sonnet", "kind": "claude", "command": "claude --model sonnet", "at_once": "3", "week_limit": "5"})).await.unwrap();
    assert_eq!(made, "Sonnet is in the pool: run_task starts tasks on it.");
    for (bad, why) in [(json!({"name": "Sonnet", "kind": "claude", "command": "claude"}), "The pool already has an agent named Sonnet."),
                       (json!({"name": "X", "kind": "claude", "command": "a\u{1b}[0m"}), "An agent's command is one line, with no control or invisible characters."),
                       (json!({"name": "X", "kind": "claude", "command": "claude", "at_once": "0"}), "at_once: a whole number from 1 to 100")] {
        assert_eq!(call(&app, BOSS, "create_agent", bad).await.unwrap_err(), why);
    }
    // a change keeps what it does not name; an empty limit clears it
    call(&app, BOSS, "update_agent", json!({"agent": "Sonnet", "at_once": "4", "week_limit": ""})).await.unwrap();
    let a = app.tasks.with(crate::tasks::agents::list).unwrap().into_iter().find(|a| a.name == "Sonnet").unwrap();
    assert_eq!((a.command.as_str(), a.at_once, a.week_limit), ("claude --model sonnet", 4, None));
    assert_eq!(call(&app, BOSS, "delete_agent", json!({"agent": "Sonnet"})).await.unwrap(), "Sonnet is out of the pool.");
    // the agent its own session runs on is held
    assert!(call(&app, BOSS, "delete_agent", json!({"agent": "Opus"})).await.unwrap_err().contains("has a session open"));
}

#[tokio::test]
async fn ac180_a_manager_keeps_its_project_s_schedules_and_no_other_s() {
    let (app, _) = app("roles-sched");
    let made = call(&app, BOSS, "create_schedule", json!({"title": "Nightly", "text": "check the deps", "agent": "Opus", "cron": "0 3 * * *", "budget_minutes": "30"})).await.unwrap();
    assert!(made.starts_with("SCH-1 is made: Every day at 03:00."), "{made}");
    let s = app.tasks.with(crate::tasks::schedules::all).unwrap().pop().unwrap();
    assert_eq!((s.project.as_str(), s.author_name.as_str(), s.paused), ("api", "Opus · manager", false), "in its project, under its name, active");
    call(&app, BOSS, "update_schedule", json!({"schedule": "SCH-1", "paused": "yes"})).await.unwrap();
    let s = app.tasks.with(crate::tasks::schedules::all).unwrap().pop().unwrap();
    assert_eq!((s.paused, s.title.as_str(), s.cron.as_deref(), s.budget_seconds), (true, "Nightly", Some("0 3 * * *"), 1800), "what it did not name stays");
    assert!(call(&app, BOSS, "create_schedule", json!({"title": "No budget", "text": "x", "agent": "Opus", "cron": "0 3 * * *"})).await.unwrap_err().starts_with("A scheduled run needs a time budget"));
    // another project's schedule is not its own
    app.tasks.with(|tx| {
        let given = crate::tasks::schedules::Given::of(tx, &json!({"title": "Web's", "body": "x", "project": "web", "agent": "a1", "cron": "0 4 * * *", "budget_minutes": 10}), 1)?;
        crate::tasks::schedules::add(tx, HUMAN, &given, 1).map(drop)
    }).unwrap();
    for tool in ["update_schedule", "delete_schedule", "run_schedule"] {
        assert_eq!(call(&app, BOSS, tool, json!({"schedule": "SCH-2"})).await.unwrap_err(), "Your project has no schedule \"SCH-2\": list_schedules shows them.", "{tool}");
    }
    let listed = call(&app, BOSS, "list_schedules", json!({})).await.unwrap();
    assert!(listed.contains("Nightly") && !listed.contains("Web's"), "{listed}");
    assert_eq!(call(&app, BOSS, "delete_schedule", json!({"schedule": "sch-1"})).await.unwrap(), "SCH-1 is deleted; its tasks and firings stay.");
}

#[tokio::test]
async fn ac181_a_token_is_kept_as_its_hash_and_names_its_role_and_project() {
    let (app, task) = app("roles-token");
    let (worker, manager) = app.tasks.with(|tx| Ok((tokens::add(tx, "ci", "worker", "api", None, 100)?, tokens::add(tx, "ops", "manager", "api", None, 100)?))).unwrap();
    assert!(worker.starts_with(tokens::PREFIX) && worker.len() == tokens::PREFIX.len() + 48);
    let kept: Vec<String> = app.tasks.with(|tx| ops::rows(tx, "SELECT hash FROM tokens", [], |r| r.get(0))).unwrap();
    assert!(kept.iter().all(|h| !worker.contains(h.as_str()) && h.len() == 64), "only hashes are kept");
    let listed = app.tasks.with(tokens::list).unwrap();
    assert!(!json!(listed).to_string().contains(&worker[tokens::PREFIX.len()..]), "the list never shows one");
    for (name, role, project, days, why) in [("ci", "worker", "api", None, "A token named ci exists: remove it first."), ("x", "admin", "api", None, "A token's role is manager or worker."),
                                             ("", "worker", "api", None, "A token's name is 1 to 40 characters on one line."), ("x", "worker", "api", Some(0), "A token lasts 1 to 3650 days, or until removed.")] {
        assert_eq!(app.tasks.with(|tx| tokens::add(tx, name, role, project, days, 100)).unwrap_err(), why);
    }
    assert!(app.tasks.with(|tx| tokens::add(tx, "x", "worker", "nowhere", None, 100)).is_err(), "a project that is there");
    let by = |token: &str, tool: &str, args: Value| { let body = json!({"token": token, "tool": tool, "args": args}); let app = app.clone(); async move { by_token(&app, &body).await } };
    assert_eq!(by(&worker, "probe", json!({})).await.unwrap(), "worker");
    assert_eq!(by(&manager, "probe", json!({})).await.unwrap(), "manager");
    // a worker's token: its project's tasks, no manager's tool, nothing of a session's own
    assert!(by(&worker, "get_task", json!({"id": task})).await.unwrap().contains("worked on"));
    assert_eq!(by(&worker, "list_agents", json!({})).await.unwrap_err(), not_yours("list_agents"));
    for own in ["take_task", "ask_human", "dispatch_task", "update_session_plan"] {
        assert_eq!(by(&worker, own, json!({"id": task})).await.unwrap_err(), format!("{own} is a session's own: a token has no session."));
    }
    by(&worker, "add_comment", json!({"id": task, "text": "from ci"})).await.unwrap();
    assert!(app.tasks.with(|tx| ops::history(tx, &task)).unwrap().to_string().contains("\"author_name\":\"token · ci\""), "what it writes carries its name");
    // a manager's token runs the manager's tools, in its project
    assert!(by(&manager, "list_agents", json!({})).await.unwrap().contains("Opus"));
    // unknown, ended and removed tokens are refused alike
    let not = by(&format!("{}{}", tokens::PREFIX, "0".repeat(48)), "probe", json!({})).await.unwrap_err();
    assert!(not.starts_with("terminum does not know this token"), "{not}");
    let short = app.tasks.with(|tx| tokens::add(tx, "a day", "manager", "api", Some(1), 100)).unwrap();
    assert!(app.tasks.with(|tx| tokens::of(tx, &short, 100 + 86399)).is_ok());
    assert_eq!(app.tasks.with(|tx| tokens::of(tx, &short, 100 + 86400)).unwrap_err(), not, "its day is over");
    app.tasks.with(|tx| tokens::remove(tx, "ci")).unwrap();
    assert_eq!(by(&worker, "probe", json!({})).await.unwrap_err(), not);
}

#[tokio::test]
async fn ac182_a_schedule_runs_now_whatever_its_cadence() {
    let (app, _) = app("roles-now");
    call(&app, BOSS, "create_schedule", json!({"title": "Nightly", "text": "check the deps", "agent": "Opus", "cron": "0 3 * * *", "budget_minutes": "30", "paused": "yes"})).await.unwrap();
    let before = app.tasks.with(crate::tasks::schedules::all).unwrap().pop().unwrap();
    // no tmux here: its task is made as a firing's and its start refused, which the answer says
    let said = call(&app, BOSS, "run_schedule", json!({"schedule": "SCH-1"})).await.unwrap();
    assert!(said.starts_with("SCH-1 ran now: its task S_API-") && said.contains(" is FAILED."), "{said}");
    let (after, made) = app.tasks.with(|tx| { let s = crate::tasks::schedules::all(tx)?.pop().unwrap(); let t = crate::tasks::schedules::last_task(tx, &s)?.unwrap(); Ok((s, t)) }).unwrap();
    assert_eq!((after.last_due, after.paused, after.cron.as_deref()), (before.last_due, true, Some("0 3 * * *")), "its cadence and its pause are as they were");
    assert_eq!((made.title.as_str(), made.project.as_str()), ("Nightly", "api"));
    let history = app.tasks.with(|tx| ops::history(tx, &made.id)).unwrap().to_string();
    assert!(history.contains("Opus · manager ran schedule SCH-1 now."), "{history}");
    // while a task of it is in progress nothing is made
    app.tasks.with(|tx| {
        let t = ops::create(tx, crate::tasks::store::DISPATCHER, NewTask { title: "Nightly".into(), project: Some("api".into()), scheduled: true, ..Default::default() }, 5)?;
        crate::tasks::schedules::fired(tx, &after, (5, 0), Some(&t.key), "made", None, 5).map(drop)
    }).unwrap();
    let busy = call(&app, BOSS, "run_schedule", json!({"schedule": "SCH-1"})).await.unwrap_err();
    assert!(busy.starts_with("Not run: S_API-") && busy.ends_with(" is still in progress."), "{busy}");
    let fires: i64 = app.tasks.with(|tx| tx.query_row("SELECT count(*) FROM schedule_fires WHERE schedule = ?1 AND task IS NOT NULL", [&after.id], |r| r.get(0)).map_err(sql)).unwrap();
    assert_eq!(fires, 2, "the run now and the one in progress, each a firing on the Scheduled page; the refused one wrote none");
}

#[tokio::test]
async fn ac179_a_session_outside_its_project_is_not_a_manager_s() {
    let (app, _) = app("roles-sessions");
    for tool in ["get_session", "send_to_session", "close_session"] {
        let said = call(&app, BOSS, tool, json!({"session": WORKER, "text": "x"})).await.unwrap_err();
        assert_eq!(said, format!("Your project has no session {WORKER:?}: list_sessions shows them."), "{tool}: only what the session list holds for its project");
    }
    assert_eq!(call(&app, BOSS, "list_sessions", json!({})).await.unwrap(), format!("{DATA}\n[]"));
}

#[tokio::test]
async fn ac174_the_list_s_rows_say_which_sessions_are_managers() {
    let (app, _) = app("roles-rows");
    let rows = || {
        app.tasks.with(|tx| app.dispatcher.read_runs(tx).map(drop)).unwrap();
        let mut groups = vec![json!({"items": [{"id": BOSS}, {"id": WORKER}, {"id": PLAIN, "manager": true}]})];
        mark(&app, &mut groups);
        groups[0]["items"].clone()
    };
    assert_eq!(rows(), json!([{"id": BOSS, "manager": true}, {"id": WORKER}, {"id": PLAIN}]), "only a + Session's session, and a row that was one says so no more");
    assert!(app.dispatcher.managers_changed() && !app.dispatcher.managers_changed(), "the list is read again once");
    app.tasks.with(|tx| { let r = runs::in_pane(tx, BOSS)?.unwrap(); runs::let_go(tx, &r.id).map(drop) }).unwrap();
    assert_eq!(rows(), json!([{"id": BOSS}, {"id": WORKER}, {"id": PLAIN}]), "let go, its row has no mark");
    assert!(app.dispatcher.managers_changed());
}
