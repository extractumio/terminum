// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The integration queue (terminum-integration.md AC-187), on a real repository in a temporary folder.
use std::path::PathBuf;

use super::*;
use crate::tasks::ops::NewTask;
use crate::tasks::store::{HUMAN, DISPATCHER};
use crate::tasks::tools::run;

const KEY: &str = "tmux:local:terminum:%7";

fn sh(dir: &Path, args: &[&str]) -> String {
    let done = Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@example.com"]).args(args).output().unwrap();
    assert!(done.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&done.stderr));
    String::from_utf8_lossy(&done.stdout).trim().to_string()
}

/// A project "api" whose folder is a repository with origin/main fetched, its integration on, and a task of it IN
/// PROGRESS in this session; the session's folder is a worktree with one commit of its own.
struct Repo {
    s: Store,
    who: Who,
    id: String,
    tree: PathBuf,
}

fn repo(name: &str) -> Repo {
    let root = crate::testutil::fresh_dir(name);
    let main = root.join("api");
    std::fs::create_dir_all(&main).unwrap();
    sh(&main, &["init", "-q", "-b", "main"]);
    sh(&main, &["commit", "-q", "--allow-empty", "-m", "base"]);
    sh(&main, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    let tree = root.join("tree");
    sh(&main, &["worktree", "add", "-q", "-b", "feat/x", tree.to_str().unwrap()]);
    std::fs::write(tree.join("x.txt"), "x\n").unwrap();
    sh(&tree, &["add", "x.txt"]);
    sh(&tree, &["commit", "-q", "-m", "x"]);
    let s = Store::open(root.join("kanban.sqlite"));
    let dir = main.to_str().unwrap().to_string();
    let id = s.with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('api', 'api', 1, ?1, 0)", [&dir]).map_err(sql)?;
        crate::tasks::slugs::fill(tx)?;
        integration::keep(tx, "api", &Settings { on: true, ..Settings::default() })?;
        let t = ops::create(tx, HUMAN, NewTask { title: "retry".into(), text: "body".into(), project: Some("api".into()), session: Some(KEY.into()), ..Default::default() }, 1)?;
        ops::move_to(tx, DISPATCHER, &t.id, "IN PROGRESS", 2)?;
        Ok(t.id)
    }).unwrap();
    let who = Who { key: KEY.into(), agent: "claude".into(), title: "api".into(), project: Some("api".into()), dir: Some(tree.to_str().unwrap().into()) };
    Repo { s, who, id, tree }
}

fn result() -> String {
    closeout::SECTIONS.iter().fold("Outcome: Done.".to_string(), |r, (name, _)| if *name == "Outcome" { r } else { format!("{r}\n{name}: None") })
}

fn call(r: &Repo, tool: &str, args: Value) -> Result<String, String> {
    run(&r.s, Path::new("/nonexistent"), &r.who, tool, &args).map(|(t, _)| t)
}

fn status(r: &Repo) -> String {
    r.s.with(|tx| Ok(ops::get(tx, &r.id)?.status)).unwrap()
}

#[test]
fn ac187_queued_with_its_result_it_stands_in_integration() {
    let r = repo("iq-queued");
    let head = sh(&r.tree, &["rev-parse", "HEAD"]);
    let said = call(&r, "integrate", json!({"id": r.id, "result": result()})).unwrap();
    assert_eq!(said, format!("{} is queued 1st for integration into origin/main. Expected batch: alone.", r.id));
    assert_eq!(status(&r), INTEGRATION);
    let q = r.s.with(|tx| queue_of(tx, "api")).unwrap();
    assert_eq!((q.len(), q[0].commit.as_str()), (1, head.as_str()), "the session's folder's HEAD");
    let history = r.s.with(|tx| ops::history(tx, &r.id)).unwrap();
    assert!(history.to_string().contains(&format!("Queued {} for integration into origin/main.", &head[..9])), "{history}");
}

#[test]
fn ac187_queued_again_it_keeps_its_place_with_the_new_commit() {
    let r = repo("iq-again");
    call(&r, "integrate", json!({"id": r.id, "result": result()})).unwrap();
    std::fs::write(r.tree.join("x.txt"), "fixed\n").unwrap();
    sh(&r.tree, &["commit", "-qam", "fix"]);
    let said = call(&r, "integrate", json!({"id": r.id, "result": result()})).unwrap();
    assert!(said.contains("queued 1st"), "{said}");
    assert_eq!(r.s.with(|tx| queue_of(tx, "api")).unwrap()[0].commit, sh(&r.tree, &["rev-parse", "HEAD"]));
}

#[test]
fn ac187_taken_out_it_is_in_progress_and_out_of_the_queue() {
    let r = repo("iq-withdraw");
    call(&r, "integrate", json!({"id": r.id, "result": result()})).unwrap();
    assert_eq!(call(&r, "withdraw_integration", json!({"id": r.id})).unwrap(), format!("{} was taken out of the queue: IN PROGRESS, its budget runs on.", r.id));
    assert_eq!(status(&r), "IN PROGRESS");
    assert!(r.s.with(|tx| queue_of(tx, "api")).unwrap().is_empty());
    assert_eq!(call(&r, "withdraw_integration", json!({"id": r.id})).unwrap_err(), format!("{} is not queued for integration.", r.id));
}

#[test]
fn ac187_any_move_out_of_integration_leaves_the_queue_and_none_puts_a_task_in() {
    let r = repo("iq-moves");
    call(&r, "integrate", json!({"id": r.id, "result": result()})).unwrap();
    r.s.with(|tx| ops::move_to(tx, HUMAN, &r.id, "TODO", 3)).unwrap();   // a human's drag, a hook's move
    assert!(r.s.with(|tx| queue_of(tx, "api")).unwrap().is_empty(), "no entry left behind its task");
    assert_eq!(r.s.with(|tx| ops::move_to(tx, HUMAN, &r.id, INTEGRATION, 4)).unwrap_err(), format!("{} goes to INTEGRATION only by integrate: it queues its commit.", r.id));
    assert_eq!(call(&r, "move_task", json!({"id": r.id, "to": "INTEGRATION"})).unwrap_err(), format!("{} goes to INTEGRATION only by integrate: it queues its commit.", r.id));
}

#[test]
fn ac187_refused_in_its_words() {
    let r = repo("iq-refused");
    std::fs::write(r.tree.join("y.txt"), "uncommitted\n").unwrap();
    assert_eq!(call(&r, "integrate", json!({"id": r.id, "result": result()})).unwrap_err(), format!("Commit your work first: 1 file is changed in {}.", r.tree.display()));
    std::fs::remove_file(r.tree.join("y.txt")).unwrap();
    assert_eq!(call(&r, "integrate", json!({"id": r.id, "result": result(), "commit": "9c1d0aa"})).unwrap_err(), "9c1d0aa is not a commit of this project's repository.");
    let base = sh(&r.tree, &["rev-parse", "HEAD~1"]);
    assert_eq!(call(&r, "integrate", json!({"id": r.id, "result": result(), "commit": base})).unwrap_err(), format!("{} is already in origin/main: finish the task with finish_task.", &base[..9]));
    assert!(call(&r, "integrate", json!({"id": r.id, "result": "Fixed."})).unwrap_err().contains("lacks"), "a result in its sections");
    r.s.with(|tx| integration::keep(tx, "api", &Settings::default())).unwrap();
    assert_eq!(call(&r, "integrate", json!({"id": r.id, "result": result()})).unwrap_err(), "Integration is off for api: publish your work as its rules say.");
    assert_eq!(status(&r), "IN PROGRESS", "nothing queued");
}

#[test]
fn ac187_the_same_commit_after_a_red_check_is_not_queued_again() {
    let r = repo("iq-red");
    let head = sh(&r.tree, &["rev-parse", "HEAD"]);
    r.s.with(|tx| tx.execute("INSERT INTO integration_returns (task, commit_sha, why, at) VALUES ((SELECT id FROM tasks WHERE slug = ?1), ?2, 'red', 1)", (&r.id, &head)).map_err(sql)).unwrap();
    assert_eq!(call(&r, "integrate", json!({"id": r.id, "result": result()})).unwrap_err(), format!("{} was returned for a red check: commit a fix first, or ask a human to queue it.", &head[..9]));
}

#[test]
fn ac187_status_says_the_settings_and_the_queue() {
    let r = repo("iq-status");
    call(&r, "integrate", json!({"id": r.id, "result": result()})).unwrap();
    let v: Value = serde_json::from_str(&call(&r, "integration_status", json!({})).unwrap()).unwrap();
    assert_eq!((v["on"].as_bool(), v["target"].as_str(), v["batch_size"].as_i64()), (Some(true), Some("origin/main"), Some(4)));
    assert_eq!(v["queue"][0]["task"].as_str(), Some(r.id.as_str()));
    assert_eq!(v["queue"][0]["place"].as_str(), Some("1st"));
}

#[test]
fn ordinals_and_lists_read_as_words() {
    assert_eq!([1, 2, 3, 4, 11, 12, 13, 21, 22].map(ordinal), ["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd"]);
    assert_eq!((and_list(&["A"]), and_list(&["A", "B"]), and_list(&["A", "B", "C"])), ("A".into(), "A and B".into(), "A, B and C".into()));
}
