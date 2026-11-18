// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A red Verify told apart from a red target (terminum-integration.md AC-198), on real repositories; the windows
//! stood in for by `sh`.
use super::*;
use crate::integrate::engine::tests::{go, queued_with, status, world, Sh, World};
use crate::tasks::integration::Settings;

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

fn said(w: &World, id: &str) -> String {
    w.s.with(|tx| ops::history(tx, id)).unwrap().to_string()
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
    assert!(words.contains(&format!("{a} stays queued: the steps its Verify failed (critical) fail on origin/main at {short} too, so the target is red, not {a}. It waits until {bug} (the red target) is DONE or FAILED, then its Verify runs again; its wait for integration goes on. Its log: ")), "{words}");
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
