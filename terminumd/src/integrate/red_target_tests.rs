// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A red Verify told apart from a red target (terminum-integration.md AC-198), on real repositories; the windows
//! stood in for by `sh`.
use super::*;
use crate::integrate::engine::tests::{go, queued_with, said, status, world, Sh, World};
use crate::tasks::integration::Settings;
use crate::tasks::ops::INTEGRATION;

/// A Verify line that writes what each run saw (`rerun|of|sha|whose tree`) to `seen` and fails `task` on the task's
/// commit and `target` on the target ("" exits 0, "-" exits 1 naming none).
fn verify(seen: &Path, task: &str, target: &str) -> String {
    let fail = |steps: &str| match steps {
        "" => "exit 0".to_string(),
        "-" => "exit 1".to_string(),
        s => format!("mkdir -p .checks; printf '{}' > .checks/failed; exit 1", s.split(' ').map(|x| format!("{x}\\n")).collect::<String>()),
    };
    format!("echo \"$TM_INTEGRATION_RERUN|$TM_INTEGRATION_RERUN_OF|$TM_INTEGRATION_SHA|$(test -f b.txt && echo task || echo target)\" >> {}; \
             if [ -n \"$TM_INTEGRATION_RERUN\" ]; then {}; fi; {}", seen.display(), fail(target), fail(task))
}

fn setup(name: &str, task: &str, target: &str) -> (World, String, std::path::PathBuf) {
    let root = crate::testutil::fresh_dir(&format!("{name}-seen"));
    let seen = root.join("seen.txt");
    let w = world(name, Settings { prepare: format!("echo prepared >> {}", root.join("prepared.txt").display()), verify: verify(&seen, task, target), ..Settings::default() });
    let a = queued_with(&w, 1, "b.txt", "b\n", true);
    (w, a, seen)
}

fn bugs(w: &World) -> Vec<(String, String, String)> {
    w.s.with(|tx| ops::rows(tx, "SELECT slug, severity, status FROM tasks WHERE title LIKE 'origin/main is red:%' ORDER BY number", [],
                           |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))).unwrap()
}

fn stage(w: &World, id: &str) -> Option<String> {
    w.s.with(|tx| tx.query_row("SELECT e.stage FROM integration_entries e JOIN tasks t ON t.id = e.task WHERE t.slug = ?1", [id], |r| r.get(0)).map_err(sql)).unwrap()
}

fn left(w: &World, id: &str) -> Option<String> {
    w.s.with(|tx| ops::how_left(tx, &ops::key_of(tx, id)?)).unwrap()
}

#[test]
fn ac198_a_red_verify_whose_steps_fail_on_the_target_too_stays_queued_behind_one_bug() {
    let (w, a, seen) = setup("rt-held", "critical", "critical");
    let target = crate::integrate::engine::tests::sh(&w.api, &["rev-parse", "origin/main"]);
    let r = Sh::default();
    go(&w, &r, 3);
    let lines = std::fs::read_to_string(&seen).unwrap();
    let commit = w.s.with(|tx| tx.query_row("SELECT commit_sha FROM integration_entries", [], |r| r.get::<_, String>(0)).map_err(sql)).unwrap();
    assert_eq!(lines, format!("||{commit}|task\ncritical|{commit}|{target}|target\n"), "its Verify alone ran again, on the target, told what and of what");
    let prepared = std::fs::read_to_string(seen.with_file_name("prepared.txt")).unwrap();
    assert_eq!(prepared.lines().count(), 1, "Prepare is not run on the target");
    assert_eq!(status(&w, &a), "INTEGRATION", "not returned");
    assert_eq!(stage(&w, &a).as_deref(), Some(HELD));
    assert_eq!(left(&w, &a), None);
    let bug = bugs(&w);
    assert_eq!(bug.len(), 1);
    let (bug, severity, open) = &bug[0];
    assert_eq!((severity.as_str(), open.as_str()), ("P1", "TODO"));
    let short = &target[..9];
    let words = said(&w, &a);
    assert!(words.contains(&format!("{a}'s Verify failed critical: they run on origin/main at {short} to tell whether the target is red too.")), "{words}");
    assert!(words.contains(&format!("{a} stays queued: the steps its Verify failed (critical) fail on origin/main at {short} too, so the target is red, not {a}. It waits until {bug} (the red target) is DONE or FAILED, then its Verify runs again; its wait for integration goes on while it stays in INTEGRATION. Its log: ")), "{words}");
    assert!(w.s.with(|tx| ops::text_of(tx, bug)).unwrap().contains(&format!("origin/main is red at {short}: the Verify steps critical fail there. {a}'s Verify failed the same steps")));
    let marks = w.s.with(|tx| crate::integrate::column::marks(tx, &[ops::get(tx, &a)?], 200)).unwrap();
    assert_eq!(marks.values().next().unwrap()["mark"], format!("waits: the target is red ({bug})"));
    assert!(w.s.with(|tx| tx.query_row("SELECT count(*) FROM integration_batches", [], |r| r.get::<_, i64>(0)).map_err(sql)).unwrap() == 0, "a held task joins no batch");
    assert!(!w.root.join("home/integration/api/verify-1").exists(), "its tree removed");

    // a second task red on the same steps: told of the same Bug, which hears of it; no second Bug
    let b = queued_with(&w, 2, "b.txt", "c\n", true);
    go(&w, &r, 3);
    assert_eq!(stage(&w, &b).as_deref(), Some(HELD));
    assert_eq!(bugs(&w).len(), 1, "one open Bug for the red target");
    assert!(said(&w, bug).contains(&format!("{b} also waits on this red target: its Verify failed critical")));
    go(&w, &r, 3);
    assert_eq!(std::fs::read_to_string(&seen).unwrap().lines().count(), 4, "held, nothing runs again");

    // the Bug DONE: each held task's Prepare and Verify run again; red on the red target still, a new Bug holds them
    w.s.with(|tx| { let t = ops::get(tx, bug)?; ops::place(tx, crate::tasks::store::HUMAN, &t, "DONE", 300) }).unwrap();
    go(&w, &r, 1);
    assert!(said(&w, &a).contains(&format!("{bug} (the red target) ended: {a}'s Prepare and Verify run again.")));
    go(&w, &r, 3);
    assert_eq!(std::fs::read_to_string(&seen).unwrap().lines().count(), 8);
    assert_eq!((stage(&w, &a).as_deref(), stage(&w, &b).as_deref()), (Some(HELD), Some(HELD)));
    assert_eq!(bugs(&w).len(), 2, "the closed Bug is not reused");
}

#[test]
fn ac198_steps_that_pass_on_the_target_return_the_task_that_broke_them() {
    // also the flaky step: red on the task after its retry, green on the target's run
    for (name, task, target, passed) in [("rt-broke", "critical cargo", "", "critical and cargo passed, so {a} broke them"),
                                         ("rt-part", "critical cargo", "critical", "cargo passed, so {a} broke it")] {
        let (w, a, _) = setup(name, task, target);
        go(&w, &Sh::default(), 3);
        assert_eq!(status(&w, &a), "IN PROGRESS");
        assert_eq!(left(&w, &a).as_deref(), Some("red"));
        let target_sha = crate::integrate::engine::tests::sh(&w.api, &["rev-parse", "--short=9", "origin/main"]);
        let failed = task.replace(' ', " and ");
        let words = said(&w, &a);
        assert!(words.contains(&format!("{a} returned: red check: its Verify failed {failed}; on origin/main at {target_sha} {}. Its log: ", passed.replace("{a}", &a))), "{words}");
        assert!(words.contains("; the target's: "), "{words}");
        assert!(bugs(&w).is_empty());
        let red: i64 = w.s.with(|tx| tx.query_row("SELECT count(*) FROM integration_returns", [], |r| r.get(0)).map_err(sql)).unwrap();
        assert_eq!(red, 1, "its commit not queued again");
    }
}

#[test]
fn ac198_a_target_run_that_cannot_tell_returns_the_task() {
    let (w, a, _) = setup("rt-unknown", "critical", "-");
    go(&w, &Sh::default(), 3);
    assert_eq!(left(&w, &a).as_deref(), Some("red"));
    let words = said(&w, &a);
    assert!(words.contains("its Verify failed critical; its run on origin/main at ") && words.contains(" ended with status 1 naming no failed step, so whether the target is red too is not known."), "{words}");
}

#[test]
fn ac198_without_a_valid_note_a_red_verify_returns_as_before() {
    for (name, note) in [("rt-none", None), ("rt-bad", Some("critical\\nrm -rf ~\\n"))] {
        let root = crate::testutil::fresh_dir(&format!("{name}-seen"));
        let seen = root.join("seen.txt");
        let write = note.map_or(String::new(), |n| format!("mkdir -p .checks; printf '{n}' > .checks/failed; "));
        let w = world(name, Settings { verify: format!("echo run >> {}; {write}exit 1", seen.display()), ..Settings::default() });
        let a = queued_with(&w, 1, "b.txt", "b\n", true);
        go(&w, &Sh::default(), 3);
        assert_eq!(left(&w, &a).as_deref(), Some("red"));
        assert!(said(&w, &a).contains("returned: red check: its Prepare or Verify ended with status 1. Its log: "));
        assert_eq!(std::fs::read_to_string(&seen).unwrap(), "run\n", "nothing ran on the target");
    }
}

#[test]
fn ac198_a_lost_window_on_the_target_runs_there_again() {
    let (w, a, seen) = setup("rt-lost", "critical", "critical");
    let r = Sh::default();
    go(&w, &r, 2);
    assert_eq!(std::fs::read_to_string(&seen).unwrap().lines().count(), 2, "the target's run started");
    r.ran.lock().unwrap().clear();   // its window gone with a restart
    go(&w, &r, 1);
    let lines = std::fs::read_to_string(&seen).unwrap();
    assert_eq!(lines.lines().count(), 3);
    assert!(lines.lines().last().unwrap().starts_with("critical|"), "started again on the target, not from the task's commit: {lines}");
    go(&w, &r, 1);
    assert_eq!(stage(&w, &a).as_deref(), Some(HELD));
}

/// The existing task `id` queued with a new commit of `file`, as `integrate` does.
fn queue_again(w: &World, id: &str, n: u32, file: &str, text: &str) {
    let sh = |args: &[&str]| crate::integrate::engine::tests::sh(&w.api, args);
    sh(&["checkout", "-q", "--detach", "origin/main"]);
    std::fs::write(w.api.join(file), text).unwrap();
    sh(&["add", file]);
    sh(&["commit", "-qm", &format!("t{n}")]);
    let commit = sh(&["rev-parse", "HEAD"]);
    sh(&["checkout", "-q", "main"]);
    w.s.with(|tx| {
        let t = ops::get(tx, id)?;
        ops::place(tx, crate::tasks::store::HUMAN, &t, "INTEGRATION", 2)?;
        tx.execute("INSERT INTO integration_entries (task, project, commit_sha, result, author, queued_at, stage) VALUES (?1, 'api', ?2, 'Outcome: Done.', 'human', ?3, 'verifying')",
                   rusqlite::params![t.key, commit, n]).map_err(sql).map(drop)
    }).unwrap();
}

#[test]
fn ac198_the_red_targets_own_task_red_still_returns_and_never_waits_behind_itself() {
    let (w, a, _) = setup("rt-itself", "critical", "critical");
    let r = Sh::default();
    go(&w, &r, 3);
    let bug = bugs(&w)[0].0.clone();
    queue_again(&w, &bug, 2, "fix.txt", "not yet\n");
    go(&w, &r, 3);
    assert_eq!(left(&w, &bug).as_deref(), Some("red"), "returned, never held behind itself");
    assert!(said(&w, &bug).contains(&format!("{bug} returned: red check: its Verify failed critical, which fail on origin/main at ")) && said(&w, &bug).contains(&format!("too, and {bug} is the red target's own task: its commit does not mend them yet.")));
    assert_eq!(stage(&w, &a).as_deref(), Some(HELD), "the others stay held");
}

#[test]
fn ac198_a_bug_failed_archived_or_gone_releases_its_held_tasks() {
    for (name, end) in [("rt-failed", "failed"), ("rt-archived", "archived"), ("rt-gone", "gone")] {
        let (w, a, _) = setup(name, "critical", "critical");
        let r = Sh::default();
        go(&w, &r, 3);
        let bug = bugs(&w)[0].0.clone();
        w.s.with(|tx| match end {
            "failed" => { let t = ops::get(tx, &bug)?; ops::place(tx, crate::tasks::store::HUMAN, &t, "FAILED", 300) }
            "archived" => ops::archive(tx, crate::tasks::store::HUMAN, &bug, true, 300),
            _ => tx.execute("UPDATE project_integration SET red_bug = 'no-such-task'", []).map(drop).map_err(sql),
        }).unwrap();
        go(&w, &r, 1);
        assert_eq!(stage(&w, &a).as_deref(), Some("verifying"), "{end}");
        let words = if end == "gone" { format!("The red target's task is gone: {a}'s Prepare and Verify run again.") } else { format!("{bug} (the red target) ended: {a}'s Prepare and Verify run again.") };
        assert!(said(&w, &a).contains(&words), "{end}");
    }
}

#[test]
fn ac198_a_task_taken_out_while_its_steps_run_on_the_target_has_that_window_ended() {
    let (w, a, seen) = setup("rt-out", "critical", "critical");
    let r = Sh::default();
    go(&w, &r, 2);
    assert!(std::fs::read_to_string(&seen).unwrap().lines().last().unwrap().starts_with("critical|"), "the target's run started");
    w.s.with(|tx| { let t = ops::get(tx, &a)?; ops::place(tx, crate::tasks::store::HUMAN, &t, "IN PROGRESS", 150) }).unwrap();
    go(&w, &r, 1);
    assert_eq!(w.s.with(|tx| tx.query_row("SELECT count(*) FROM integration_verifies", [], |r| r.get::<_, i64>(0)).map_err(sql)).unwrap(), 0);
    assert!(!w.root.join("home/integration/api/verify-1").exists());
    assert!(bugs(&w).is_empty(), "nothing judged");
}

#[test]
fn ac198_a_run_on_the_target_that_cannot_start_returns_the_task() {
    let (w, a, seen) = setup("rt-nostart", "critical", "critical");
    let r = Sh::default();
    go(&w, &r, 1);
    w.s.with(|tx| {
        let mut s = crate::tasks::integration::read(tx, "api")?;
        s.target = "origin/none".into();
        crate::tasks::integration::keep(tx, "api", &s)
    }).unwrap();
    go(&w, &r, 1);
    assert_eq!(left(&w, &a).as_deref(), Some("red"));
    let words = said(&w, &a);
    assert!(words.contains(&format!("{a} returned: red check: its Verify failed critical; its run on the target did not start: ")) && words.contains("so whether the target is red too is not known."), "{words}");
    assert_eq!(std::fs::read_to_string(&seen).unwrap().lines().count(), 1, "nothing ran on the target");
    assert!(!w.root.join("home/integration/api/verify-1").exists(), "its tree removed");
}

fn again(bug: &str, t: &str) -> String {
    format!("{bug} (the red target) ended: {t} left the queue while it waited behind it, so nothing runs for it. Integrate again to land it.")
}

/// Held tasks leave the queue: by new work in its session (Dispatcher's move), a human's Take out then DONE, a human's
/// move then queued again by Integrate, and DONE from the queue; only the one still out of the queue and open is told
/// to integrate again, once.
#[test]
fn ac198_a_held_task_that_leaves_the_queue_is_told_and_told_again_when_the_red_target_ends() {
    let (w, a, _) = setup("rt-left", "critical", "critical");
    let b = queued_with(&w, 2, "b.txt", "c\n", true);
    let c = queued_with(&w, 3, "b.txt", "d\n", true);
    let d = queued_with(&w, 4, "b.txt", "e\n", true);
    let r = Sh::default();
    go(&w, &r, 8);
    for t in [&a, &b, &c, &d] {
        assert_eq!(stage(&w, t).as_deref(), Some(HELD), "{t}");
    }
    let bug = bugs(&w)[0].0.clone();
    let human = crate::tasks::store::HUMAN;
    w.s.with(|tx| {
        ops::move_to(tx, DISPATCHER, &a, "IN PROGRESS", 150)?;
        crate::integrate::column::out(tx, &b, 150)?;
        ops::place(tx, human, &ops::get(tx, &c)?, "IN PROGRESS", 150)?;
        ops::place(tx, human, &ops::get(tx, &d)?, "DONE", 150)
    }).unwrap();
    for t in [&a, &b, &c] {
        assert_eq!(status(&w, t), "IN PROGRESS");
        assert!(said(&w, t).contains(&format!("{t} left the queue while it waited behind {bug} (the red target): it is not queued now, so {bug}'s end runs nothing for it. \
            Integrate again to land it; {t} is told when {bug} ends.")), "{t}: {}", said(&w, t));
        assert!(said(&w, &bug).contains(&format!("{t} left the queue while it waited on this red target: when this task ends, {t} is told to integrate again.")));
    }
    assert_eq!((left(&w, &a).as_deref(), left(&w, &c).as_deref()), (Some("out"), Some("out")));
    assert!(!said(&w, &d).contains("left the queue"), "DONE from the queue: nothing pending");
    // b ends; c is queued again by a human's Integrate from a folder at a new commit
    w.s.with(|tx| ops::place(tx, human, &ops::get(tx, &b)?, "DONE", 160)).unwrap();
    let sh = |args: &[&str]| crate::integrate::engine::tests::sh(&w.api, args);
    sh(&["checkout", "-q", "--detach", "origin/main"]);
    std::fs::write(w.api.join("b.txt"), "f\n").unwrap();
    sh(&["add", "b.txt"]);
    sh(&["commit", "-qm", "t5"]);
    crate::tasks::integration_queue::integrate_by_human(&w.s, &c, w.api.to_str()).unwrap();
    sh(&["checkout", "-q", "main"]);
    assert_eq!(status(&w, &c), INTEGRATION);
    w.s.with(|tx| { let t = ops::get(tx, &bug)?; ops::place(tx, human, &t, "DONE", 300) }).unwrap();
    go(&w, &r, 2);
    assert_eq!(said(&w, &a).matches(&again(&bug, &a)).count(), 1, "told once: {}", said(&w, &a));
    assert_eq!(status(&w, &a), "IN PROGRESS", "nothing queued for it");
    let notice = w.s.with(|tx| ops::noticed(tx, DISPATCHER, &ops::key_of(tx, &a)?, "requeue", &ops::key_of(tx, &bug)?)).unwrap();
    assert!(notice, "a notice: its session hears it (AC-192)");
    for t in [&b, &c, &d] {
        assert!(!said(&w, t).contains(&again(&bug, t)), "{t}: DONE or queued again since");
    }
    let rows: i64 = w.s.with(|tx| tx.query_row("SELECT count(*) FROM integration_held_left", [], |r| r.get(0)).map_err(sql)).unwrap();
    assert_eq!(rows, 0, "nothing kept once told");
}

#[test]
fn ac198_a_held_task_leaving_after_its_red_target_ended_is_told_at_once() {
    let (w, a, _) = setup("rt-left-late", "critical", "critical");
    go(&w, &Sh::default(), 3);
    let bug = bugs(&w)[0].0.clone();
    w.s.with(|tx| {
        ops::archive(tx, crate::tasks::store::HUMAN, &bug, true, 300)?;
        ops::move_to(tx, DISPATCHER, &a, "IN PROGRESS", 301)
    }).unwrap();
    assert!(said(&w, &a).contains(&again(&bug, &a)));
    assert_eq!(left(&w, &a).as_deref(), Some("out"));
}
