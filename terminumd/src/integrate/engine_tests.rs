// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The integration queue's steps (terminum-integration.md AC-188…AC-191) on real repositories in a temporary
//! folder: a bare remote, the project's clone, tasks' commits; the check's window stood in for by `sh`.
use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;

use super::*;
use crate::integrate::runner::with_vars;
use crate::tasks::ops::{sql, NewTask, INTEGRATION};
use crate::tasks::store::HUMAN;

/// Runs a line at once with `sh`, or, `hang`, never ends it.
#[derive(Default)]
struct Sh {
    ran: Mutex<HashMap<String, (Option<i32>, String)>>,
    lines: Mutex<Vec<String>>,
    hang: bool,
}

impl Runner for Sh {
    fn start(&self, label: &str, folder: &Path, line: &str, vars: &[(&str, String)]) -> Result<String, String> {
        self.lines.lock().unwrap().push(format!("{label}: {line}"));
        let o = Command::new("/bin/sh").arg("-c").arg(with_vars(line, vars)).current_dir(folder).output().map_err(|e| e.to_string())?;
        let status = if self.hang { None } else { o.status.code() };
        let handle = format!("%{}", self.ran.lock().unwrap().len() + 1);
        self.ran.lock().unwrap().insert(handle.clone(), (status, String::from_utf8_lossy(&o.stdout).to_string()));
        Ok(handle)
    }
    fn status(&self, h: &str) -> Result<Option<i32>, String> {
        self.ran.lock().unwrap().get(h).map(|r| r.0).ok_or_else(|| "gone".into())
    }
    fn output(&self, h: &str) -> String {
        self.ran.lock().unwrap().get(h).map(|r| r.1.clone()).unwrap_or_default()
    }
    fn stop(&self, h: &str) {
        if let Some(r) = self.ran.lock().unwrap().get_mut(h) {
            r.0 = Some(-15);
        }
    }
    fn close(&self, _: &str) {}
}

fn sh(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@example.com"]).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

struct World {
    root: PathBuf,
    origin: PathBuf,
    api: PathBuf,
    s: Store,
}

fn world(name: &str, settings: Settings) -> World {
    let root = crate::testutil::fresh_dir(name);
    let (origin, api) = (root.join("origin.git"), root.join("api"));
    sh(&root, &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
    sh(&root, &["clone", "-q", origin.to_str().unwrap(), api.to_str().unwrap()]);
    sh(&api, &["config", "user.name", "t"]);
    sh(&api, &["config", "user.email", "t@example.com"]);
    std::fs::write(api.join("a.txt"), "one\ntwo\n").unwrap();
    sh(&api, &["add", "a.txt"]);
    sh(&api, &["commit", "-qm", "base"]);
    sh(&api, &["push", "-q", "origin", "HEAD:main"]);
    sh(&api, &["fetch", "-q", "origin"]);
    let s = Store::open(root.join("kanban.sqlite"));
    let dir = api.to_str().unwrap().to_string();
    s.with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('api', 'api', 1, ?1, 0)", [&dir]).map_err(sql)?;
        crate::tasks::slugs::fill(tx)?;
        integration::keep(tx, "api", &Settings { on: true, ..settings })
    }).unwrap();
    World { root, origin, api, s }
}

/// A task queued with a commit on origin/main that writes `file`.
fn queued(w: &World, n: u32, file: &str, text: &str) -> String {
    let id = w.s.with(|tx| ops::create(tx, HUMAN, NewTask { title: format!("t{n}"), text: "b".into(), project: Some("api".into()), ..Default::default() }, 1).map(|t| t.id)).unwrap();
    sh(&w.api, &["checkout", "-q", "--detach", "origin/main"]);
    std::fs::write(w.api.join(file), text).unwrap();
    sh(&w.api, &["add", file]);
    sh(&w.api, &["commit", "-qm", &format!("t{n}")]);
    let commit = sh(&w.api, &["rev-parse", "HEAD"]);
    sh(&w.api, &["checkout", "-q", "main"]);
    w.s.with(|tx| {
        let t = ops::get(tx, &id)?;
        ops::place(tx, HUMAN, &t, INTEGRATION, 2)?;
        tx.execute("INSERT INTO integration_entries (task, project, commit_sha, result, author, queued_at) VALUES (?1, 'api', ?2, ?3, 'human', ?4)",
                   rusqlite::params![t.key, commit, format!("Outcome: Done. t{n}"), n]).map_err(sql).map(drop)
    }).unwrap();
    id
}

fn go(w: &World, r: &Sh, steps: usize) {
    for i in 0..steps {
        step(&w.s, r, &w.root.join("home"), 100 + i as i64).unwrap();
    }
}

fn status(w: &World, id: &str) -> String {
    w.s.with(|tx| Ok(ops::get(tx, id)?.status)).unwrap()
}

fn batch(w: &World) -> Batch {
    w.s.with(|tx| tx.query_row("SELECT id FROM integration_batches ORDER BY id DESC LIMIT 1", [], |r| r.get::<_, i64>(0)).map_err(sql).and_then(|id| bs::by_id(tx, id))).unwrap()
}

fn on_origin(w: &World) -> String {
    sh(&w.origin, &["ls-tree", "--name-only", "main"])
}

#[test]
fn ac191_no_check_a_batch_is_published_its_tasks_done_and_the_folder_follows() {
    let w = world("ie-plain", Settings::default());
    let (a, b) = (queued(&w, 1, "b.txt", "b\n"), queued(&w, 2, "c.txt", "c\n"));
    let old = sh(&w.origin, &["rev-parse", "main"]);
    go(&w, &Sh::default(), 2);
    assert_eq!((status(&w, &a), status(&w, &b)), ("DONE".into(), "DONE".into()));
    assert_eq!(on_origin(&w), "a.txt\nb.txt\nc.txt");
    assert!(Command::new("git").arg("-C").arg(&w.origin).args(["merge-base", "--is-ancestor", &old, "main"]).status().unwrap().success(), "never forced: the old main stays under the new");
    let bt = batch(&w);
    assert_eq!(bt.state, "ended");
    let i = w.s.with(|tx| bs::slug(tx, bt.task.as_deref().unwrap())).unwrap();
    assert!(i.starts_with("I_"), "{i}");
    assert_eq!(status(&w, &i), "DONE");
    assert_eq!(sh(&w.api, &["ls-files"]), "a.txt\nb.txt\nc.txt", "the project's folder fast-forwarded");
    assert_eq!(w.s.with(|tx| integration::read(tx, "api")).unwrap().size, 8, "a green batch doubles the size");
    let comments = w.s.with(|tx| ops::history(tx, &a)).unwrap().to_string();
    assert!(comments.contains("Outcome: Done. t1") && comments.contains(&format!("Integrated into origin/main at {} in batch 1, with {b}.", &sh(&w.origin, &["rev-parse", "main"])[..9])), "{comments}");
}

#[test]
fn ac190_a_green_check_on_the_candidate_then_the_publish() {
    let w = world("ie-check", Settings { check: "test -f b.txt && test -f c.txt && test \"$TM_INTEGRATION_BATCH\" = 1".into(), ..Settings::default() });
    let (a, _) = (queued(&w, 1, "b.txt", "b\n"), queued(&w, 2, "c.txt", "c\n"));
    let r = Sh::default();
    go(&w, &r, 1);
    assert_eq!(batch(&w).state, "checking");
    go(&w, &r, 3);
    assert_eq!(status(&w, &a), "DONE");
    assert!(r.lines.lock().unwrap()[0].starts_with("batch-1.1.check: test -f b.txt"));
    assert!(w.root.join("home/integration/api/batch-1.1.check.log").exists(), "its output kept in the batch's log");
}

#[test]
fn ac190_a_red_check_publishes_nothing_halves_the_size_and_waits_for_a_human() {
    let w = world("ie-red", Settings { check: "exit 3".into(), ..Settings::default() });
    let a = queued(&w, 1, "b.txt", "b\n");
    go(&w, &Sh::default(), 4);
    let bt = batch(&w);
    assert_eq!((bt.state.as_str(), bt.red_rounds), ("red", 1));
    assert_eq!(status(&w, &a), INTEGRATION, "its tasks stay queued in the batch");
    assert_eq!(on_origin(&w), "a.txt", "nothing published");
    let i = w.s.with(|tx| bs::slug(tx, bt.task.as_deref().unwrap())).unwrap();
    assert_eq!(status(&w, &i), "WAIT FOR HUMAN");
    assert_eq!(w.s.with(|tx| integration::read(tx, "api")).unwrap().size, 2);
}

#[test]
fn ac190_a_check_over_its_time_is_ended_and_red() {
    let w = world("ie-late", Settings { check: "true".into(), check_minutes: 1, ..Settings::default() });
    queued(&w, 1, "b.txt", "b\n");
    let r = Sh { hang: true, ..Default::default() };
    step(&w.s, &r, &w.root.join("home"), 100).unwrap();
    step(&w.s, &r, &w.root.join("home"), 101).unwrap();
    step(&w.s, &r, &w.root.join("home"), 102).unwrap();
    assert_eq!(batch(&w).state, "checking", "within its minute");
    step(&w.s, &r, &w.root.join("home"), 200).unwrap();
    let bt = batch(&w);
    assert_eq!(bt.state, "red");
    assert_eq!(w.s.with(|tx| tx.query_row("SELECT why FROM integration_rounds WHERE batch = ?1", [bt.id], |r| r.get::<_, String>(0)).map_err(sql)).unwrap(), "The check ran over 1 minutes.");
}

#[test]
fn ac191_the_target_moved_builds_it_anew_and_checks_again() {
    let w = world("ie-moved", Settings { check: "true".into(), ..Settings::default() });
    let a = queued(&w, 1, "b.txt", "b\n");
    let r = Sh::default();
    go(&w, &r, 3);
    assert_eq!(batch(&w).state, "green");
    // a human pushes meanwhile
    let other = w.root.join("other");
    sh(&w.root, &["clone", "-q", w.origin.to_str().unwrap(), other.to_str().unwrap()]);
    std::fs::write(other.join("z.txt"), "z\n").unwrap();
    sh(&other, &["add", "z.txt"]);
    sh(&other, &["commit", "-qm", "human"]);
    sh(&other, &["push", "-q", "origin", "HEAD:main"]);
    go(&w, &r, 1);
    let bt = batch(&w);
    assert_eq!((bt.state.as_str(), bt.round), ("checking", 2), "built anew on the moved target, checked again");
    go(&w, &r, 3);
    assert_eq!(status(&w, &a), "DONE");
    assert_eq!(on_origin(&w), "a.txt\nb.txt\nz.txt");
}

#[test]
fn ac189_a_conflict_with_the_target_returns_at_once_in_its_words() {
    let w = world("ie-conflict", Settings::default());
    let a = queued(&w, 1, "a.txt", "one\nzwei\n");
    sh(&w.api, &["checkout", "-q", "main"]);
    std::fs::write(w.api.join("a.txt"), "one\nTWO\n").unwrap();
    sh(&w.api, &["commit", "-qam", "moved"]);
    sh(&w.api, &["push", "-q", "origin", "HEAD:main"]);
    go(&w, &Sh::default(), 1);
    assert_eq!(status(&w, &a), "IN PROGRESS");
    let h = w.s.with(|tx| ops::history(tx, &a)).unwrap().to_string();
    assert!(h.contains(&format!("{a} returned: conflict with origin/main at")) && h.contains("in a.txt. Merge"), "{h}");
    assert!(w.s.with(|tx| bs::waiting(tx, "api")).unwrap().is_empty());
}

#[test]
fn ac191_a_publish_terminumd_did_not_see_end_is_found_published_never_built_again() {
    let w = world("ie-restart", Settings::default());
    let a = queued(&w, 1, "b.txt", "b\n");
    go(&w, &Sh::default(), 1);
    let bt = batch(&w);
    assert_eq!(bt.state, "green");
    // the push went through, terminumd stopped before it wrote so
    sh(&w.api, &["push", "-q", "origin", &format!("{}:refs/heads/main", bt.candidate.as_deref().unwrap())]);
    go(&w, &Sh::default(), 1);
    assert_eq!(status(&w, &a), "DONE");
    assert_eq!(sh(&w.origin, &["rev-parse", "main"]), bt.candidate.unwrap(), "the same commit, no second merge on top");
}

/// A red batch of two tasks, its check `exit 3`.
fn red_batch(name: &str) -> (World, String, String, Batch) {
    let w = world(name, Settings { check: "exit 3".into(), ..Settings::default() });
    let (a, b) = (queued(&w, 1, "b.txt", "b\n"), queued(&w, 2, "c.txt", "c\n"));
    go(&w, &Sh::default(), 4);
    let bt = batch(&w);
    assert_eq!(bt.state, "red");
    (w, a, b, bt)
}

#[test]
fn ac194_the_human_returns_one_task_and_the_rest_is_built_anew_and_checked() {
    let (w, a, b, bt) = red_batch("ik-return");
    assert_eq!(w.s.with(|tx| super::super::keys::return_one(tx, bt.id, &a, "", 300)).unwrap_err(), "Say why, for its agent.");
    w.s.with(|tx| super::super::keys::return_one(tx, bt.id, &a, "its test fails", 300)).unwrap();
    assert_eq!(status(&w, &a), "IN PROGRESS");
    assert!(w.s.with(|tx| ops::history(tx, &a)).unwrap().to_string().contains("returned by"), "the human's words as its analysis");
    go(&w, &Sh::default(), 1);
    let again = batch(&w);
    assert_eq!((again.state.as_str(), again.round, again.tasks.len()), ("checking", 2, 1), "built anew without it");
    assert_eq!(status(&w, &b), INTEGRATION);
}

#[test]
fn ac194_return_all_gives_every_task_back_and_ends_the_batch() {
    let (w, a, b, bt) = red_batch("ik-all");
    w.s.with(|tx| super::super::keys::return_all(tx, bt.id, 300)).unwrap();
    assert_eq!((status(&w, &a), status(&w, &b)), ("IN PROGRESS".into(), "IN PROGRESS".into()));
    let i = w.s.with(|tx| bs::slug(tx, bt.task.as_deref().unwrap())).unwrap();
    assert_eq!((batch(&w).state.as_str(), status(&w, &i)), ("given up", "DONE".to_string()));
    assert!(w.s.with(|tx| super::super::keys::return_all(tx, bt.id, 301)).unwrap_err().contains("nothing to decide now"));
}

#[test]
fn ac194_check_again_runs_the_same_candidate_a_new_round() {
    let (w, _, _, bt) = red_batch("ik-again");
    w.s.with(|tx| super::super::keys::check_again(tx, bt.id, 300)).unwrap();
    let again = batch(&w);
    assert_eq!((again.state.as_str(), again.round, again.candidate), ("checking", 2, bt.candidate));
    let i = w.s.with(|tx| bs::slug(tx, bt.task.as_deref().unwrap())).unwrap();
    assert_eq!(status(&w, &i), INTEGRATION, "terminumd works on it again");
}

#[test]
fn ac191_a_refused_push_is_not_published_and_publish_again_tries_once_more() {
    let w = world("ik-refused", Settings::default());
    let a = queued(&w, 1, "b.txt", "b\n");
    let hook = w.origin.join("hooks/pre-receive");
    std::fs::write(&hook, "#!/bin/sh\necho nobody may push >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    go(&w, &Sh::default(), 2);
    let bt = batch(&w);
    assert_eq!(bt.state, "not published");
    assert_eq!(status(&w, &a), INTEGRATION);
    std::fs::remove_file(&hook).unwrap();
    w.s.with(|tx| super::super::keys::publish_again(tx, bt.id, 300)).unwrap();
    go(&w, &Sh::default(), 1);
    assert_eq!(status(&w, &a), "DONE");
}

#[test]
fn ac187_a_task_taken_out_of_a_batch_under_check_is_left_out_and_the_rest_checked_again() {
    let w = world("ie-out", Settings { check: "true".into(), ..Settings::default() });
    let (a, b) = (queued(&w, 1, "b.txt", "b\n"), queued(&w, 2, "c.txt", "c\n"));
    let r = Sh { hang: true, ..Default::default() };
    go(&w, &r, 2);
    assert!(batch(&w).pane.is_some(), "its check runs");
    w.s.with(|tx| { let t = ops::get(tx, &a)?; ops::place(tx, HUMAN, &t, "IN PROGRESS", 150) }).unwrap();   // a human's drag, withdraw_integration, a hook
    assert_eq!(batch(&w).state, "rebuild");
    go(&w, &Sh::default(), 4);
    assert_eq!((status(&w, &a), status(&w, &b)), ("IN PROGRESS".into(), "DONE".into()));
    assert_eq!(on_origin(&w), "a.txt\nc.txt", "what was taken out is never published");
}

#[test]
fn ac186_integration_turned_off_mid_batch_publishes_nothing_and_gives_the_tasks_back() {
    for state_steps in [1, 3] {
        let w = world(&format!("ie-off-{state_steps}"), Settings { check: "true".into(), ..Settings::default() });
        let a = queued(&w, 1, "b.txt", "b\n");
        go(&w, &Sh::default(), state_steps);
        w.s.with(|tx| { let s = integration::read(tx, "api")?; integration::keep(tx, "api", &Settings { on: false, ..s }) }).unwrap();
        go(&w, &Sh::default(), 2);
        assert_eq!((batch(&w).state.as_str(), status(&w, &a).as_str()), ("given up", "IN PROGRESS"), "after {state_steps} steps");
        assert_eq!(on_origin(&w), "a.txt");
    }
}

#[test]
fn ac194_a_batch_s_integration_task_moves_with_its_batch_only() {
    let w = world("ie-itask", Settings { check: "true".into(), ..Settings::default() });
    queued(&w, 1, "b.txt", "b\n");
    go(&w, &Sh { hang: true, ..Default::default() }, 2);
    let bt = batch(&w);   // read outside: the store's lock is not taken twice
    let i = w.s.with(|tx| bs::slug(tx, bt.task.as_deref().unwrap())).unwrap();
    assert_eq!(w.s.with(|tx| ops::move_to(tx, HUMAN, &i, "DONE", 200)).unwrap_err(), format!("{i} is batch 1's integration task: it ends with its batch."));
}
