// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! An agent's rights on tasks (AC-115).

use super::*;
use crate::tasks::store::HUMAN;

const KEY: &str = "tmux:local:terminum:%7";
const OTHER: &str = "tmux:local:terminum:%8";

fn who(project: Option<&str>) -> Who {
    Who { key: KEY.into(), agent: "claude".into(), title: "api".into(), project: project.map(str::to_string), dir: None }
}

fn fresh(name: &str) -> (Store, std::path::PathBuf) {
    let dir = crate::testutil::fresh_dir(name);
    let s = Store::open(dir.join("kanban.sqlite"));
    s.with(|tx| { tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('api', 'api', 1, '', 0), ('web', 'web', 2, '', 1)", []).map_err(ops::sql)?; crate::tasks::slugs::fill(tx) }).unwrap();
    (s, dir)
}

fn task(s: &Store, project: &str, session: Option<&str>) -> String {
    s.with(|tx| ops::create(tx, HUMAN, NewTask { title: format!("t in {project} {session:?}"), text: "body".into(), project: Some(project.into()), session: session.map(str::to_string), ..Default::default() }, 1)).unwrap().id
}

/// A text in the default template's sections (AC-145).
const SECTIONS: &str = "## Problem or goal\nx\n## Context\ny\n## Expected result\nz\n## Prerequisites\nNone";

/// The close-out's sections after `head`, each "None" (AC-166).
fn closed_out(head: &str) -> String {
    closeout::SECTIONS.iter().fold(head.to_string(), |r, (name, _)| format!("{r}\n{name}: None"))
}

/// A create_task with no text gets one in the template's sections, a finish_task's result the close-out's: what these
/// tests check is elsewhere.
fn call(s: &Store, dir: &Path, w: &Who, tool: &str, mut args: Value) -> Result<String, String> {
    if tool == "create_task" && args.get("text").is_none() && args.get("title").is_some() {
        args["text"] = json!(SECTIONS);
    }
    if tool == "finish_task" && let Some(head) = args["result"].as_str() {
        args["result"] = json!(closed_out(head));
    }
    run(s, dir, w, tool, &args).map(|(t, _)| t)
}

#[test]
fn ac115_an_agent_does_all_on_its_own() {
    let (s, dir) = fresh("tt-own");
    let w = who(Some("api"));
    let id = task(&s, "api", Some(KEY));
    let got = call(&s, &dir, &w, "get_task", json!({"id": id})).unwrap();
    assert!(got.starts_with(DATA), "said to be data: {got}");
    assert!(got.contains("\"text\":\"body\""));
    assert_eq!(call(&s, &dir, &w, "move_task", json!({"id": id, "to": "IN PROGRESS"})).unwrap(), format!("{id} is in IN PROGRESS."));
    call(&s, &dir, &w, "add_comment", json!({"id": id, "text": "note"})).unwrap();
    call(&s, &dir, &w, "finish_task", json!({"id": id, "result": "**fixed**"})).unwrap();
    let (status, history) = s.with(|tx| Ok((ops::get(tx, &id)?.status, ops::history(tx, &id)?))).unwrap();
    assert_eq!(status, "DONE");
    let lines = history.as_array().unwrap();
    assert!(lines.iter().any(|e| e["kind"] == "comment" && e["text"] == closed_out("**fixed**") && e["author_name"] == "claude · api"), "its result is its comment, its author the agent");
}

/// A human's RUN of `id`, its agent working in `pane`.
pub(crate) fn running(s: &Store, id: &str, pane: &str) {
    use crate::dispatcher::runs::{self, Phase, Run};
    s.with(|tx| {
        let like = Run { agent_id: "a1".into(), agent_name: "Opus".into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: "api".into(), ..Default::default() };
        let run = runs::add(tx, &like, &[ops::key_of(tx, id)?], Default::default(), 1)?;
        runs::change(tx, &run, |r| { r.state = Phase::Working; r.pane_key = Some(pane.into()); }).map(drop)
    }).unwrap();
}

fn status(s: &Store, id: &str) -> String {
    s.with(|tx| ops::get(tx, id)).unwrap().status
}

#[test]
fn ac115_a_task_of_the_project_is_moved_and_finished_whatever_session_holds_it() {
    let (s, dir) = fresh("tt-others");
    let w = who(Some("api"));
    let (held, free) = (task(&s, "api", Some(OTHER)), task(&s, "api", None));
    assert!(call(&s, &dir, &w, "get_task", json!({"id": held})).is_ok());
    assert!(call(&s, &dir, &w, "add_comment", json!({"id": held, "text": "seen"})).is_ok());
    assert_eq!(call(&s, &dir, &w, "move_task", json!({"id": held, "to": "IN PROGRESS"})).unwrap(), format!("{held} is in IN PROGRESS."), "another session's: moved");
    assert_eq!(call(&s, &dir, &w, "move_task", json!({"id": held, "to": "WAIT FOR HUMAN"})).unwrap_err(), format!("{held} belongs to session %8: only its agent asks on it. You can comment."), "nor parked for a human with no question (AC-157)");
    assert_eq!(status(&s, &held), "IN PROGRESS");
    assert_eq!(call(&s, &dir, &w, "finish_task", json!({"id": held, "result": "did it"})).unwrap(), format!("{held} is DONE, with your result."));
    assert_eq!(call(&s, &dir, &w, "finish_task", json!({"id": free, "result": "and this"})).unwrap(), format!("{free} is DONE, with your result."), "a free one, with no take_task");
    assert_eq!((status(&s, &held), status(&s, &free)), ("DONE".into(), "DONE".into()));
    let history = s.with(|tx| ops::history(tx, &held)).unwrap();
    assert!(history.as_array().unwrap().iter().any(|e| e["kind"] == "comment" && e["text"] == closed_out("did it") && e["author_name"] == "claude · api"), "its history says who finished it: {history}");
    assert_eq!(s.with(|tx| ops::get(tx, &held)).unwrap().session.as_deref(), Some(OTHER), "its session stays its session");
}

#[test]
fn ac115_inside_a_humans_run_an_agent_finishes_its_projects_other_tasks_too() {
    let (s, dir) = fresh("tt-inrun");
    let w = who(Some("api"));
    let (mine, kid) = (task(&s, "api", Some(KEY)), task(&s, "api", None));
    running(&s, &mine, KEY);
    assert!(call(&s, &dir, &w, "take_task", json!({"id": kid})).unwrap_err().contains("finish that first"), "taking stays refused in a RUN");
    assert!(call(&s, &dir, &w, "finish_task", json!({"id": kid, "result": "the subtask"})).is_ok());
    assert!(call(&s, &dir, &w, "finish_task", json!({"id": mine, "result": "the main one"})).is_ok());
    assert_eq!((status(&s, &kid), status(&s, &mine)), ("DONE".into(), "DONE".into()));
}

#[test]
fn ac115_what_a_humans_run_runs_in_another_session_is_that_runs() {
    let (s, dir) = fresh("tt-held");
    let w = who(Some("api"));
    let id = task(&s, "api", Some(OTHER));
    running(&s, &id, OTHER);
    let refused = format!("{id} runs in session %8 at a human's RUN. You can read it and comment.");
    assert_eq!(call(&s, &dir, &w, "move_task", json!({"id": id, "to": "DONE"})).unwrap_err(), refused);
    assert_eq!(call(&s, &dir, &w, "finish_task", json!({"id": id, "result": "x"})).unwrap_err(), refused);
    assert!(call(&s, &dir, &w, "add_comment", json!({"id": id, "text": "seen"})).is_ok());
    assert_eq!(status(&s, &id), "TODO");
    // from its start, with no pane yet, until Dispatcher closes it: a turn's end between does not free it
    let starting = task(&s, "api", None);
    s.with(|tx| crate::dispatcher::runs::add(tx, &crate::dispatcher::runs::Run { agent_id: "a2".into(), agent_name: "Opus".into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: "api".into(), ..Default::default() }, &[ops::key_of(tx, &starting)?], Default::default(), 1).map(drop)).unwrap();
    assert_eq!(call(&s, &dir, &w, "finish_task", json!({"id": starting, "result": "x"})).unwrap_err(), format!("{starting} is starting at a human's RUN. You can read it and comment."));
    s.with(|tx| { let run = crate::dispatcher::runs::last_of(tx, &ops::key_of(tx, &id)?)?.unwrap(); crate::dispatcher::runs::change(tx, &run.id, |r| r.turn_ended = true).map(drop) }).unwrap();
    assert_eq!(call(&s, &dir, &w, "move_task", json!({"id": id, "to": "DONE"})).unwrap_err(), refused, "its turn ended, the run not closed");
    s.with(|tx| { let run = crate::dispatcher::runs::last_of(tx, &ops::key_of(tx, &id)?)?.unwrap(); crate::dispatcher::runs::change(tx, &run.id, |r| r.state = crate::dispatcher::runs::Phase::Closed).map(drop) }).unwrap();
    assert!(call(&s, &dir, &w, "move_task", json!({"id": id, "to": "FAILED"})).is_ok(), "the run over, it is moved as any other");
    let far = task(&s, "web", None);
    assert_eq!(call(&s, &dir, &w, "finish_task", json!({"id": far, "result": "x"})).unwrap_err(), format!("{far} is outside this session's project."), "another project stays out of reach");
    assert_eq!(call(&s, &dir, &who(None), "finish_task", json!({"id": id, "result": "x"})).unwrap_err(), format!("{id} is outside this session's project."), "a session in Default shares nothing");
}

#[test]
fn ac115_another_project_is_out_of_scope() {
    let (s, dir) = fresh("tt-scope");
    let w = who(Some("api"));
    let id = task(&s, "web", None);
    assert_eq!(call(&s, &dir, &w, "get_task", json!({"id": id})).unwrap_err(), format!("{id} is outside this session's project."));
    assert!(call(&s, &dir, &w, "add_comment", json!({"id": id, "text": "x"})).is_err());
    assert!(call(&s, &dir, &w, "take_task", json!({"id": id})).is_err());
    let listed = call(&s, &dir, &w, "list_tasks", json!({})).unwrap();
    assert!(!listed.contains(&format!("\"{id}\"")), "{listed}");
}

#[test]
fn ac115_a_session_in_default_reaches_only_its_own_and_what_it_made() {
    let (s, dir) = fresh("tt-default");
    let w = who(None);
    let mine = task(&s, "default", Some(KEY));
    let others = task(&s, "default", Some(OTHER));
    let free = task(&s, "default", None);
    let made = call(&s, &dir, &w, "create_task", json!({"title": "Bug: a gap", "severity": "P2"})).unwrap();
    let made = made.trim_start_matches("Kept as ").trim_end_matches(", in TODO.").to_string();
    let listed = call(&s, &dir, &w, "list_tasks", json!({})).unwrap();
    for (id, seen) in [(&mine, true), (&made, true), (&others, false), (&free, false)] {
        assert_eq!(listed.contains(&format!("\"{id}\"")), seen, "{id} in {listed}");
    }
    assert!(call(&s, &dir, &w, "get_task", json!({"id": others})).is_err());
    let (first, array) = listed.split_once('\n').unwrap();
    assert_eq!(first, format!("{DATA}{NO_PROJECT}"), "its first line says the list is its own, no board's");
    assert!(array.starts_with('['), "the array stays the second line: {array}");
    let shared = call(&s, &dir, &who(Some("api")), "list_tasks", json!({})).unwrap();
    assert_eq!(shared.lines().next(), Some(DATA), "a project's session is told nothing more");
}

#[test]
fn ac110_two_default_sessions_of_one_title_share_nothing() {
    let (s, dir) = fresh("tt-same-title");
    let a = who(None);
    let b = Who { key: OTHER.into(), ..who(None) };
    let made = call(&s, &dir, &a, "create_task", json!({"title": "Debt: a's gap", "severity": "P3"})).unwrap();
    let id = made.trim_start_matches("Kept as ").trim_end_matches(", in TODO.").to_string();
    assert!(call(&s, &dir, &a, "get_task", json!({"id": id})).is_ok());
    assert!(call(&s, &dir, &b, "get_task", json!({"id": id})).is_err(), "the same title gives no reach");
    assert!(!call(&s, &dir, &b, "list_tasks", json!({})).unwrap().contains(&format!("\"{id}\"")));
}

#[test]
fn ac115_reading_makes_no_user_row() {
    let (s, dir) = fresh("tt-read");
    call(&s, &dir, &who(Some("api")), "list_tasks", json!({})).unwrap();
    let agents: i64 = s.with(|tx| tx.query_row("SELECT count(*) FROM users WHERE kind = 'agent'", [], |r| r.get(0)).map_err(ops::sql)).unwrap();
    assert_eq!(agents, 0);
}

#[test]
fn ac115_a_list_is_cut_after_an_agent_s_reach_and_says_so() {
    let (s, dir) = fresh("tt-cut");
    let w = who(None);
    for n in 0..200 {
        s.with(|tx| ops::create(tx, HUMAN, NewTask { title: format!("busy {n}"), severity: Some("P0".into()), ..Default::default() }, 1)).unwrap();
    }
    let mine = task(&s, "default", Some(KEY));
    let listed = call(&s, &dir, &w, "list_tasks", json!({})).unwrap();
    assert!(listed.contains(&format!("\"{mine}\"")), "its own, past the board's first 200");
    for n in 0..200 {
        s.with(|tx| ops::create(tx, HUMAN, NewTask { title: format!("in api {n}"), project: Some("api".into()), ..Default::default() }, 1)).unwrap();
    }
    assert!(call(&s, &dir, &who(Some("api")), "list_tasks", json!({})).unwrap().ends_with("ask by status or severity for others."));
}

#[test]
fn ac115_a_free_task_can_be_taken_one_with_a_session_cannot() {
    let (s, dir) = fresh("tt-take");
    let w = who(Some("api"));
    let free = task(&s, "api", None);
    call(&s, &dir, &w, "take_task", json!({"id": free})).unwrap();
    let t = s.with(|tx| ops::get(tx, &free)).unwrap();
    assert_eq!((t.session.as_deref(), t.status.as_str(), t.owner_name.as_str()), (Some(KEY), "TODO", "claude · api"), "linked, owned and still in TODO");
    let held = task(&s, "api", Some(OTHER));
    assert_eq!(call(&s, &dir, &w, "take_task", json!({"id": held})).unwrap_err(), format!("{held} has a session already."));
}

#[test]
fn ac115_a_new_task_is_marked_cleaned_and_not_kept_twice() {
    let (s, dir) = fresh("tt-new");
    let w = who(Some("api"));
    let said = call(&s, &dir, &w, "create_task", json!({"title": "Bug: Gap\u{202e} in\u{7} retry", "text": format!("{SECTIONS}\nAPI_TOKEN=abc123 here"), "severity": "P1"})).unwrap();
    let id = said.trim_start_matches("Kept as ").trim_end_matches(", in TODO.").to_string();
    let (t, text) = s.with(|tx| Ok((ops::get(tx, &id)?, ops::text_of(tx, &id)?))).unwrap();
    assert_eq!((t.title.as_str(), t.by_agent, t.session.clone(), t.status.as_str(), t.project.as_str(), t.severity.as_str()), ("Bug: Gap in retry", true, None, "TODO", "api", "P1"));
    assert!(text.contains("[redacted]") && !text.contains("abc123"), "{text}");
    assert_eq!(call(&s, &dir, &w, "create_task", json!({"title": "Bug: gap in  RETRY.", "severity": "P1"})).unwrap_err(), format!("Not kept: {id} already says this. Read it with execute_tool(\"get_task\", {{id}}), and comment if you have more."));
    assert_eq!(call(&s, &dir, &w, "create_task", json!({})).unwrap_err(), "title is required");
}

#[test]
fn ac109_an_agent_s_file_stays_inside_its_folder_or_comes_as_text() {
    let (s, dir) = fresh("tt-file");
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("out.log"), "log").unwrap();
    std::fs::write(dir.join("secret.txt"), "key").unwrap();
    std::os::unix::fs::symlink(dir.join("secret.txt"), work.join("link.txt")).unwrap();
    let mut w = who(Some("api"));
    w.dir = Some(work.display().to_string());
    let id = task(&s, "api", Some(KEY));
    call(&s, &dir, &w, "attach_file", json!({"task": id, "path": work.join("out.log")})).unwrap();
    let list = s.with(|tx| files::of_task(tx, &id)).unwrap();
    assert_eq!(list[0].name, "out.log");
    assert!(call(&s, &dir, &w, "attach_file", json!({"task": id, "path": dir.join("secret.txt")})).unwrap_err().contains("outside this session's folder"));
    assert!(call(&s, &dir, &w, "attach_file", json!({"task": id, "path": work.join("link.txt")})).unwrap_err().contains("outside"), "judged on resolved paths");
    call(&s, &dir, &w, "attach_file", json!({"task": id, "path": "out.log"})).unwrap();
    assert_eq!(s.with(|tx| files::of_task(tx, &id)).unwrap().len(), 2, "a relative path is the session's");
    let got = call(&s, &dir, &w, "get_file", json!({"task": id, "name": "out.log"})).unwrap();
    assert!(got.contains(&list[0].id));
    w.dir = None;
    assert!(call(&s, &dir, &w, "attach_file", json!({"task": id, "path": "/etc/hosts"})).unwrap_err().starts_with("From a host"));
    call(&s, &dir, &w, "attach_file", json!({"task": id, "name": "note.txt", "text": "hello"})).unwrap();
    let big = "x".repeat(TEXT_FILE_MOST + 1);
    assert!(call(&s, &dir, &w, "attach_file", json!({"task": id, "name": "big.txt", "text": big})).unwrap_err().contains("at most"));
    assert!(call(&s, &dir, &w, "get_file", json!({"task": id, "name": "note.txt"})).unwrap_err().contains("host"));
}

/// terminum-kanban.md AC-145: only what is left to do, said as such
#[test]
fn ac145_what_is_left_to_do_says_its_kind_and_severity() {
    let ((s, dir), w) = (fresh("tools-left"), who(Some("api")));
    assert_eq!(call(&s, &dir, &w, "create_task", json!({"title": "a gap", "severity": "P2"})).unwrap_err(), "Start the title with Bug:, Feat: or Debt:.");
    assert_eq!(call(&s, &dir, &w, "create_task", json!({"title": "Feat: a gap"})).unwrap_err(), "Give it a severity: P0, P1, P2 or P3.");
    for title in ["Bug: drops a BOM", "Feat: paging", "Debt: two copies of a rule"] {
        assert!(call(&s, &dir, &w, "create_task", json!({"title": title, "severity": "P2"})).unwrap().starts_with("Kept as"), "{title}");
    }
}

#[test]
fn ac142_a_session_with_no_task_takes_one_and_writes_as_its_agent_of_the_pool() {
    let ((s, dir), w) = (fresh("tools-ad-hoc"), who(Some("api")));
    let free = task(&s, "api", None);
    s.with(|tx| {
        let like = crate::dispatcher::runs::Run { agent_id: "a1".into(), agent_name: "Opus reviewer".into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: "api".into(), ..Default::default() };
        let id = crate::dispatcher::runs::add(tx, &like, &[], Default::default(), 1)?;
        crate::dispatcher::runs::change(tx, &id, |r| { r.state = crate::dispatcher::runs::Phase::Working; r.pane_key = Some(KEY.into()); }).map(drop)
    }).unwrap();
    assert!(call(&s, &dir, &w, "take_task", json!({"id": free})).is_ok(), "+ Session's agent may take a task: it was given none");
    call(&s, &dir, &w, "add_comment", json!({"id": free, "text": "on it"})).unwrap();
    let author: String = s.with(|tx| tx.query_row("SELECT u.name FROM events e JOIN users u ON u.id = e.author WHERE e.kind = 'comment' ORDER BY e.rowid DESC LIMIT 1", [], |r| r.get(0)).map_err(|e| e.to_string())).unwrap();
    assert_eq!(author, "Opus reviewer", "one agent, one name: the pool's");
}

#[test]
fn ac120_a_long_get_task_comes_in_pages() {
    let ((s, dir), w) = (fresh("tools-pages"), who(Some("api")));
    let id = s.with(|tx| {
        let t = ops::create(tx, HUMAN, NewTask { title: "long".into(), project: Some("api".into()), ..Default::default() }, 1)?;
        for n in 0..105 {
            ops::comment(tx, HUMAN, &t.id, &format!("comment {n} {}", "x".repeat(900)), 2)?;
        }
        Ok(t.id)
    }).unwrap();
    let first = call(&s, &dir, &w, "get_task", json!({"id": id})).unwrap();
    assert!(first.len() < HISTORY_PAGE_BYTES + 2_000, "a page within the link's limit: {}", first.len());
    assert!(first.contains("page 1 of 3: execute_tool(\"get_task\", {id, page}) with page \"2\" for the next") && first.contains("comment 0 ") && !first.contains("comment 104 "), "the first page says there is more");
    let last = call(&s, &dir, &w, "get_task", json!({"id": id, "page": "3"})).unwrap();
    let task: Value = serde_json::from_str(last.lines().last().unwrap()).unwrap();
    assert!(last.contains("comment 104 ") && !last.contains("for the next") && task.get("text").is_none() && task["files"].is_i64(), "the last page: its text and files' list only on the first, here their number");
    assert_eq!(call(&s, &dir, &w, "get_task", json!({"id": id, "page": "4"})).unwrap_err(), "page is 1 to 3");
    assert!(call(&s, &dir, &w, "get_task", json!({"id": id, "page": "x"})).is_err());
}

#[test]
fn ac120_a_host_s_get_file_goes_through_its_link() {
    let ((s, dir), w) = (fresh("tools-hostfile"), who(Some("api")));
    let id = s.with(|tx| ops::create(tx, HUMAN, NewTask { title: "t".into(), project: Some("api".into()), ..Default::default() }, 1).map(|t| t.id)).unwrap();
    call(&s, &dir, &w, "attach_file", json!({"task": id, "name": "log.txt", "text": "hello"})).unwrap();
    assert!(call(&s, &dir, &w, "get_file", json!({"task": id, "name": "log.txt"})).unwrap_err().contains("through its link"), "the route sends it, not run");
    let (name, path) = s.with(|tx| task_file(tx, &dir, &w, &json!({"task": id, "name": "log.txt"}))).unwrap();
    assert_eq!((name.as_str(), std::fs::read_to_string(path).unwrap().as_str()), ("log.txt", "hello"));
    assert!(s.with(|tx| task_file(tx, &dir, &w, &json!({"task": id, "name": "none.txt"}))).unwrap_err().contains("has no file none.txt"));
    assert!(s.with(|tx| task_file(tx, &dir, &who(Some("web")), &json!({"task": id, "name": "log.txt"}))).unwrap_err().contains("outside"), "judged as get_task");
}

#[test]
fn ac145_an_agent_s_text_follows_its_project_s_template() {
    let (s, dir) = fresh("tt-template");
    let w = who(Some("api"));
    let bare = run(&s, &dir, &w, "create_task", &json!({"title": "Bug: no sections", "severity": "P2", "text": "it breaks"})).unwrap_err();
    assert!(bare.starts_with("Not kept: the text lacks the sections") && bare.contains("## Expected result"), "the default is quoted: {bare}");
    s.with(|tx| crate::tasks::template::set(tx, "api", "## Why\n## Done when")).unwrap();
    let own = run(&s, &dir, &w, "create_task", &json!({"title": "Bug: own", "severity": "P2", "text": SECTIONS})).unwrap_err();
    assert!(own.ends_with("## Why\n## Done when"), "the project's own is quoted: {own}");
    assert!(run(&s, &dir, &w, "create_task", &json!({"title": "Bug: own", "severity": "P2", "text": "## Why\na\n## Done when\nb"})).is_ok());
    // a title without its kind and a text without the sections: both said in one refusal
    let both = run(&s, &dir, &w, "create_task", &json!({"title": "a gap", "severity": "P2", "text": "it breaks"})).unwrap_err();
    assert!(both.starts_with("Start the title with Bug:, Feat: or Debt:.\nNot kept: the text lacks the sections") && both.ends_with("## Done when"), "{both}");
    let all = run(&s, &dir, &w, "create_task", &json!({"title": "a gap", "text": "it breaks"})).unwrap_err();
    assert!(all.starts_with("Give it a severity: P0, P1, P2 or P3.\nStart the title with Bug:, Feat: or Debt:.\nNot kept:"), "{all}");
    // update_task by the same rules, every fault at once
    let id = run(&s, &dir, &w, "create_task", &json!({"title": "Bug: mine", "severity": "P2", "text": "## Why\na\n## Done when\nb"})).unwrap().0;
    let id = id.split_whitespace().find(|x| x.starts_with("API-")).unwrap().trim_end_matches(['.', ':', ',']).to_string();
    let upd = run(&s, &dir, &w, "update_task", &json!({"id": id, "title": "mine", "text": "it breaks"})).unwrap_err();
    assert!(upd.starts_with("Start the title with Bug:, Feat: or Debt:.\nNot kept: the text lacks the sections"), "{upd}");
}

#[test]
fn ac157_an_agent_asks_on_its_own_task_and_a_dispatcher_session_is_told_to_end_its_turn() {
    let ((s, dir), w) = (fresh("tools-ask"), who(Some("api")));
    let id = task(&s, "api", Some(KEY));
    let other = task(&s, "api", Some(OTHER));
    let asked = "1. Which API? Options: v1, v2. Recommended: v2, v1 is going away.";
    assert_eq!(call(&s, &dir, &w, "ask_human", json!({"id": id, "questions": asked})).unwrap(), format!("{id} waits for the human, with your questions. Wait for the answer in this session."));
    let (status, history) = s.with(|tx| Ok((ops::get(tx, &id)?.status, ops::history(tx, &id)?))).unwrap();
    assert_eq!(status, "WAIT FOR HUMAN");
    assert!(history.as_array().unwrap().iter().any(|e| e["kind"] == "comment" && e["text"] == asked && e["questions"] == true), "kept as questions: {history}");
    assert_eq!(call(&s, &dir, &w, "ask_human", json!({"id": other, "questions": "x"})).unwrap_err(), format!("{other} belongs to session %8: only its agent asks on it. You can comment."), "only its own task");
    running(&s, &id, KEY);
    let said = call(&s, &dir, &w, "ask_human", json!({"id": id, "questions": asked})).unwrap();
    assert!(said.contains("End your turn now") && said.contains("resumes this conversation"), "{said}");
    call(&s, &dir, &w, "move_task", json!({"id": id, "to": "IN PROGRESS"})).unwrap();
    let refused = call(&s, &dir, &w, "move_task", json!({"id": id, "to": "WAIT FOR HUMAN"})).unwrap_err();
    assert!(refused.contains("ask_human") && refused.contains("stays where it was"), "its own move with no question is refused, pointing to ask_human: {refused}");
    assert_eq!(s.with(|tx| ops::get(tx, &id)).unwrap().status, "IN PROGRESS", "nothing moved");
    let asked_now = s.with(|tx| crate::dispatcher::runs::in_pane(tx, KEY)).unwrap().unwrap().asked;
    assert!(!asked_now, "its move back off WAIT FOR HUMAN cleared the run's ask");
}

#[test]
fn ac166_a_task_s_end_says_what_its_session_made_and_what_its_result_names_in_vain() {
    let (s, dir) = fresh("tt-made");
    let w = who(Some("api"));
    let made_id = |said: String| said.trim_start_matches("Kept as ").split(',').next().unwrap().to_string();
    let early = made_id(call(&s, &dir, &w, "create_task", json!({"title": "Debt: before this task", "severity": "P3"})).unwrap());
    s.with(|tx| tx.execute("UPDATE tasks SET created_at = 1 WHERE slug = ?1", [&early]).map(drop).map_err(ops::sql)).unwrap();
    let id = task(&s, "api", None);
    call(&s, &dir, &w, "take_task", json!({"id": id})).unwrap();
    call(&s, &dir, &w, "move_task", json!({"id": id, "to": "IN PROGRESS"})).unwrap();
    let left = made_id(call(&s, &dir, &w, "create_task", json!({"title": "Bug: left for later", "severity": "P2"})).unwrap());
    let dropped = made_id(call(&s, &dir, &w, "create_task", json!({"title": "Debt: no longer needed", "severity": "P3"})).unwrap());
    s.with(|tx| tx.execute("UPDATE tasks SET archived_at = 2 WHERE slug = ?1", [&dropped]).map(drop).map_err(ops::sql)).unwrap();
    let line = format!("Tasks made in its session: {left}.");
    let said = call(&s, &dir, &w, "finish_task", json!({"id": id, "result": format!("New tasks: {left}, API-99 and API-99 again; UTF-8 and AC-125 are no tasks")})).unwrap();
    assert_eq!(said, format!("{id} is DONE, with your result. {line} {}", closeout::unknown_words(std::slice::from_ref(&"API-99".to_string()))), "one made before the task was started, and one archived since, are not its follow-ups");
    let told = |id: &str| s.with(|tx| ops::history(tx, id)).unwrap().as_array().unwrap().iter().filter(|e| e["kind"] == "told").map(|e| e["text"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(told(&id), std::slice::from_ref(&line), "terminum's own line, under the result");
    // however it ends: a human's or Dispatcher's move says it too
    s.with(|tx| { ops::move_to(tx, HUMAN, &id, "IN PROGRESS", 1)?; ops::move_to(tx, crate::tasks::store::DISPATCHER, &id, "FAILED", 2) }).unwrap();
    assert_eq!(told(&id).len(), 2, "FAILED by Dispatcher: {:?}", told(&id));
    let plain = task(&s, "api", None);
    assert_eq!(call(&s, &dir, &w, "finish_task", json!({"id": plain, "result": "did it"})).unwrap(), format!("{plain} is DONE, with your result."), "no session, never IN PROGRESS: nothing is claimed");
    assert!(told(&plain).is_empty());
}

fn call_raw(s: &Store, dir: &Path, w: &Who, args: Value) -> String {
    run(s, dir, w, "finish_task", &args).map(|(t, _)| t).unwrap()
}

#[test]
fn ac123_a_result_is_kept_only_in_its_sections() {
    let (s, dir) = fresh("tt-sections");
    let w = who(Some("api"));
    let id = task(&s, "api", Some(KEY));
    let bare = call_raw(&s, &dir, &w, json!({"id": id, "result": "Outcome: done\nDone: the fix"}));
    assert_eq!(bare, closeout::missing_words(&id, "TODO", &["Checked", "Not done", "Needs a human", "New tasks", "Tasks changed", "Issues and discoveries", "Further recommendations"]));
    let (status, comments) = s.with(|tx| Ok((ops::get(tx, &id)?.status, ops::history(tx, &id)?.as_array().unwrap().iter().filter(|e| e["kind"] == "comment").count()))).unwrap();
    assert_eq!((status.as_str(), comments), ("TODO", 1), "not a result: its words a comment, the task where it was");
    assert!(s.with(|tx| Ok(crate::dispatcher::runs::card_marks(tx, &[ops::get(tx, &id)?.key.as_str()])?.is_empty())).unwrap());
    let markdown = closeout::SECTIONS.iter().map(|(name, _)| format!("## {name}\nNone")).collect::<Vec<_>>().join("\n");
    assert_eq!(call_raw(&s, &dir, &w, json!({"id": id, "result": markdown})), format!("{id} is DONE, with your result."), "Markdown headings count too");
}

#[test]
fn ac123_a_section_is_a_heading_line_and_a_schedule_has_its_own_form() {
    for (before, after) in [("", ": x"), ("**", ":** x"), ("### ", ""), ("__", "__\nx"), ("- **", ":** x"), ("1. ", ": x"), ("2) ", ""), ("> ", ":"), ("= ", ":")] {
        let r = closeout::SECTIONS.iter().map(|(n, _)| format!("{before}{}{after}", n.to_uppercase())).collect::<Vec<_>>().join("\n");
        let lacks = closeout::missing(&r, &closeout::SECTIONS);
        assert_eq!(lacks.is_empty(), before != "= ", "{r}: {lacks:?}");
    }
    assert_eq!(closeout::missing("Not done: x\nDone by hand\nThe Outcome: y", &closeout::SECTIONS[..3]), ["Outcome", "Done", "Checked"], "a heading starts its line and ends at its name");
    assert!(closeout::missing("Outcome: ok\nSummary: s\nDetails: d\nFor the human: nothing", &closeout::SCHEDULED).is_empty());
}

/// terminum-dispatcher.md AC-170: Auto-Dispatcher's round reads and finishes its own; it makes, takes and moves nothing.
#[test]
fn ac170_a_rounds_pane_makes_takes_and_moves_nothing() {
    use crate::dispatcher::runs::{self, Phase, Run};
    let (s, dir) = fresh("tools-round");
    let (other, round) = (task(&s, "api", None), task(&s, "api", None));
    s.with(|tx| {
        let key = ops::key_of(tx, &round)?;
        crate::tasks::auto::add_round(tx, &crate::tasks::auto::Round { task: key.clone(), project: "api".into(), slots: 1, candidates: vec![other.clone()] }, 1)?;
        let run = runs::add(tx, &Run { agent_id: "p".into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: "api".into(), ..Default::default() }, &[key], Default::default(), 1)?;
        runs::change(tx, &run, |r| { r.state = Phase::Working; r.pane_key = Some(KEY.into()); }).map(drop)
    }).unwrap();
    let w = who(Some("api"));
    for (tool, args) in [("create_task", json!({"title": "Bug: more work", "severity": "P1"})), ("take_task", json!({"id": other})), ("move_task", json!({"id": other, "to": "DONE"})),
                         ("update_task", json!({"id": other, "severity": "P0", "reason": "x"})), ("ask_human", json!({"id": round.clone(), "questions": "1?"}))] {
        assert_eq!(call(&s, &dir, &w, tool, args).unwrap_err(), format!("Auto-Dispatcher's round reads tasks, starts them with dispatch_task and finishes its own, {round}: {tool} is not its."));
    }
    assert!(call(&s, &dir, &w, "finish_task", json!({"id": other, "result": "done"})).unwrap_err().ends_with("finish_task is not its."), "another's task is not its to finish");
    assert!(call(&s, &dir, &w, "add_comment", json!({"id": other, "text": "x"})).unwrap_err().ends_with("add_comment is not its."), "what is not named is not its");
    assert!(call(&s, &dir, &w, "get_task", json!({"id": other})).is_ok());
    assert!(call(&s, &dir, &who(Some("api")), "list_tasks", json!({})).is_ok());
}

#[test]
fn ac166_follow_ups_that_name_no_task_are_reminded() {
    let (s, dir) = fresh("tt-untracked");
    let w = who(Some("api"));
    let finish = |id: &str, head: &str| call(&s, &dir, &w, "finish_task", json!({"id": id, "result": head})).unwrap();
    let started = || {
        let id = task(&s, "api", None);
        call(&s, &dir, &w, "take_task", json!({"id": id})).unwrap();
        call(&s, &dir, &w, "move_task", json!({"id": id, "to": "IN PROGRESS"})).unwrap();
        id
    };
    let id = started();
    let said = finish(&id, "Further recommendations: replay the steps on the live board");
    assert_eq!(said, format!("{id} is DONE, with your result. {}", closeout::untracked_words(&["Further recommendations"])), "DONE all the same: a reminder");
    let id = started();
    let said = finish(&id, "Not done:\n- the live replay\nFurther recommendations:\n- also docs");
    assert!(said.ends_with(&closeout::untracked_words(&["Not done", "Further recommendations"])), "{said}");
    let id = started();
    let other = task(&s, "api", None);
    assert_eq!(finish(&id, &format!("Further recommendations: replay it, {other}")), format!("{id} is DONE, with your result."), "one that names its task");
    let id = started();
    assert_eq!(finish(&id, "Outcome: done"), format!("{id} is DONE, with your result."), "every follow-up None");
}
