// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A red target run again by terminumd (terminum-integration.md AC-198), on real repositories; the windows stood in
//! for by `sh`.
use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;

use super::*;
use crate::integrate::engine::step;
use crate::integrate::engine::tests::{go, queued_with, said, sh, status, world, Sh, World};
use crate::tasks::integration::Settings;

/// A Verify line that writes `rerun|of|sha` of each run to `seen` and fails step "critical" until `green` exists.
fn verify(seen: &Path, green: &Path) -> String {
    format!("echo \"$TM_INTEGRATION_RERUN|$TM_INTEGRATION_RERUN_OF|$TM_INTEGRATION_SHA\" >> {}; test -f {} && exit 0; mkdir -p .checks; printf 'critical\\n' > .checks/failed; exit 1",
            seen.display(), green.display())
}

struct Red {
    w: World,
    task: String,
    bug: String,
    seen: PathBuf,
    green: PathBuf,
    r: Sh,
}

/// A queued task held behind a red target whose Bug task names "critical", found red at time 100.
fn held(name: &str) -> Red {
    let root = crate::testutil::fresh_dir(&format!("{name}-seen"));
    let (seen, green) = (root.join("seen.txt"), root.join("green"));
    let w = world(name, Settings { verify: verify(&seen, &green), ..Settings::default() });
    let task = queued_with(&w, 1, "b.txt", "b\n", true);
    let r = Sh::default();
    go(&w, &r, 3);
    let bug = w.s.with(|tx| ops::rows(tx, "SELECT slug FROM tasks WHERE title LIKE 'origin/main is red:%'", [], |r| r.get(0))).unwrap().pop().expect("a Bug task");
    assert_eq!(stage(&w, &task).as_deref(), Some(super::super::red_target::HELD));
    Red { w, task, bug, seen, green, r }
}

fn beat(red: &Red, now: i64) {
    step(&red.w.s, &red.r, &red.w.root.join("home"), now).unwrap();
}

fn runs(red: &Red) -> usize {
    std::fs::read_to_string(&red.seen).unwrap_or_default().lines().count()
}

fn stage(w: &World, id: &str) -> Option<String> {
    w.s.with(|tx| tx.query_row("SELECT e.stage FROM integration_entries e JOIN tasks t ON t.id = e.task WHERE t.slug = ?1", [id], |r| r.get(0)).optional().map_err(sql)).unwrap().flatten()
}

#[test]
fn ac198_a_red_that_does_not_hold_ends_its_bug_by_itself_and_frees_its_tasks() {
    let red = held("rr-green");
    let target = sh(&red.w.api, &["rev-parse", "origin/main"]);
    let commit = red.w.s.with(|tx| tx.query_row("SELECT of_sha FROM integration_red", [], |r| r.get::<_, String>(0)).map_err(sql)).unwrap();
    let before = runs(&red);
    beat(&red, 1000);
    assert_eq!(runs(&red), before, "not before 30 minutes");
    std::fs::write(&red.green, "").unwrap();   // the busy moment has passed
    beat(&red, 2000);
    let lines = std::fs::read_to_string(&red.seen).unwrap();
    assert_eq!(lines.lines().last(), Some(format!("critical|{commit}|{target}").as_str()), "the Bug's steps alone, of the held task's plan, on the target itself");
    beat(&red, 2001);
    assert_eq!(status(&red.w, &red.bug), "DONE", "green there: Dispatcher ends the Bug task");
    let words = said(&red.w, &red.bug);
    assert!(words.contains(&format!("critical pass on origin/main at {} on terminumd's run: the target is green for them, the red did not hold. {} is DONE", &target[..9], red.bug)), "{words}");
    beat(&red, 2002);
    assert!(said(&red.w, &red.task).contains(&format!("{} (the red target) ended: {}'s Prepare and Verify run again.", red.bug, red.task)));
    for now in 2003..2012 {
        beat(&red, now);
    }
    assert_eq!(status(&red.w, &red.task), "DONE", "freed, it went on to its batch and landed, no one asked");
}

#[test]
fn ac198_a_red_that_holds_runs_again_only_when_the_target_moves() {
    let red = held("rr-red");
    beat(&red, 2000);
    let after_first = runs(&red);
    beat(&red, 2001);
    assert_eq!(status(&red.w, &red.bug), "TODO", "red there too: it stays");
    assert!(said(&red.w, &red.bug).contains("critical fail there too on origin/main at"));
    for now in [2400, 4000, 9000] {
        beat(&red, now);
    }
    assert_eq!(runs(&red), after_first, "the same target red twice: nothing runs again");
    // the target moves: its steps run again there, 30 minutes after the last red
    sh(&red.w.api, &["checkout", "-q", "--detach", "origin/main"]);
    std::fs::write(red.w.api.join("c.txt"), "c\n").unwrap();
    sh(&red.w.api, &["add", "c.txt"]);
    sh(&red.w.api, &["commit", "-qm", "moved"]);
    sh(&red.w.api, &["push", "-q", "origin", "HEAD:main"]);
    sh(&red.w.api, &["checkout", "-q", "main"]);
    beat(&red, 9500);
    assert_eq!(runs(&red), after_first + 1, "a moved target runs them again");
    assert_eq!(status(&red.w, &red.task), "INTEGRATION", "held all along, never returned");
}

#[test]
fn ac198_the_same_steps_red_then_green_twice_in_a_week_get_a_bug_of_their_own() {
    let red = held("rr-flaky");
    std::fs::write(&red.green, "").unwrap();
    beat(&red, 2000);
    beat(&red, 2001);
    assert_eq!(status(&red.w, &red.bug), "DONE");
    let flaky = |w: &World| w.s.with(|tx| ops::rows(tx, "SELECT slug FROM tasks WHERE title LIKE 'critical fail now and then%'", [], |r| r.get::<_, String>(0))).unwrap();
    assert!(flaky(&red.w).is_empty(), "once is a busy moment");
    // red again: a new Bug task, green again on its re-run within the week
    std::fs::remove_file(&red.green).unwrap();
    let again = queued_with(&red.w, 2, "d.txt", "d\n", true);
    for now in 3000..3006 {
        beat(&red, now);
    }
    assert_eq!(stage(&red.w, &again).as_deref(), Some(super::super::red_target::HELD));
    std::fs::write(&red.green, "").unwrap();
    beat(&red, 5000);
    beat(&red, 5001);
    let made = flaky(&red.w);
    assert_eq!(made.len(), 1, "a Bug task names the step red now and then");
    let t = red.w.s.with(|tx| ops::get(tx, &made[0])).unwrap();
    assert_eq!((t.severity.as_str(), t.status.as_str()), ("P1", "TODO"));
    assert!(red.w.s.with(|tx| integration::read(tx, "api")).is_ok());
}

#[test]
fn ac198_a_bug_a_human_ended_is_not_run_again() {
    let red = held("rr-human");
    red.w.s.with(|tx| { let t = ops::get(tx, &red.bug)?; ops::place(tx, crate::tasks::store::HUMAN, &t, "FAILED", 500) }).unwrap();
    beat(&red, 2000);
    beat(&red, 2001);
    assert!(said(&red.w, &red.task).contains(&format!("{} (the red target) ended: {}'s Prepare and Verify run again.", red.bug, red.task)), "its held task's own Verify runs again");
    assert!(!said(&red.w, &red.bug).contains("terminumd runs critical again"), "no re-run of the target for an ended Bug");
    assert_eq!(status(&red.w, &red.bug), "FAILED");
}
