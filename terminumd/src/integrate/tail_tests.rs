// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A published batch's After beside the next batch's Check (terminum-integration.md AC-188, AC-191) on real
//! repositories in a temporary folder; the lines' windows stood in for by `sh`, an After or a Check held running.
use crate::integrate::engine::step;
use crate::integrate::engine::tests::{batch, go, on_origin, queued, sh, status, world, Sh, World};
use crate::integrate::store::{self as bs, Batch};
use crate::tasks::integration::{self, Settings};
use crate::tasks::ops::{self, sql};
use rusqlite::OptionalExtension;

/// Batch `n` of the test's project, once built.
fn built(w: &World, n: i64) -> Option<Batch> {
    w.s.with(|tx| tx.query_row("SELECT id FROM integration_batches WHERE number = ?1", [n], |r| r.get::<_, i64>(0)).optional().map_err(sql)?.map(|id| bs::by_id(tx, id)).transpose()).unwrap()
}

fn nth(w: &World, n: i64) -> Batch {
    built(w, n).expect("built")
}

/// Steps until batch `n` stands in `state` with its line's window open (`window`) or not; at most 20.
fn until(w: &World, r: &Sh, n: i64, state: &str, window: bool) -> Batch {
    for _ in 0..20 {
        if let Some(b) = built(w, n) && b.state == state && b.pane.is_some() == window {
            return b;
        }
        go(w, r, 1);
    }
    panic!("batch {n} never stood {state}: {:?}", built(w, n));
}

fn files(w: &World) -> String {
    sh(&w.api, &["ls-files"])
}

/// Its integration task's ID.
fn i_of(w: &World, b: &Batch) -> String {
    w.s.with(|tx| bs::task_of(tx, b)).unwrap().unwrap()
}

fn said(w: &World, b: &Batch) -> String {
    let i = i_of(w, b);   // read outside: the store's lock is not taken twice
    w.s.with(|tx| ops::history(tx, &i)).unwrap().to_string()
}

fn settings() -> Settings {
    Settings { check: "true".into(), after: "true".into(), ..Settings::default() }
}

/// Batch 1 of two tasks published, its After running in the project's folder.
fn after_runs(name: &str, r: &Sh) -> World {
    let w = world(name, settings());
    queued(&w, 1, "b.txt", "b\n");
    queued(&w, 4, "e.txt", "e\n");
    until(&w, r, 1, "after", true);
    w
}

#[test]
fn ac191_the_next_batch_is_checked_and_published_while_after_runs_its_folder_moved_after() {
    let r = Sh { hang_on: vec![".after"], ..Default::default() };
    let w = after_runs("it-overlap", &r);
    let b = queued(&w, 2, "c.txt", "c\n");
    until(&w, &r, 2, bs::PUBLISHED, false);
    assert_eq!(status(&w, &b), "DONE", "its tasks DONE while batch 1's After runs");
    assert_eq!(on_origin(&w), "a.txt\nb.txt\nc.txt\ne.txt");
    assert_eq!(files(&w), "a.txt\nb.txt\ne.txt", "the folder is not moved under batch 1's After");
    let first = nth(&w, 1);
    assert_eq!((first.state.as_str(), first.tasks.len()), ("after", 2), "its tasks' DONE leaves the published batch as it is");
    assert!(!said(&w, &first).contains("nothing published"), "{}", said(&w, &first));
    assert!(r.lines.lock().unwrap().iter().any(|l| l.starts_with("batch-2.1.check")), "batch 2 was checked meanwhile");
    r.end(nth(&w, 1).pane.as_deref().unwrap(), 0);
    go(&w, &r, 1);
    assert_eq!(nth(&w, 1).state, "ended");
    until(&w, &r, 2, "after", true);
    assert_eq!(files(&w), "a.txt\nb.txt\nc.txt\ne.txt", "the folder follows when batch 2's After's turn comes");
}

#[test]
fn ac191_afters_waiting_run_one_at_a_time_in_the_order_published() {
    let r = Sh { hang_on: vec![".after"], ..Default::default() };
    let w = after_runs("it-order", &r);
    queued(&w, 2, "c.txt", "c\n");
    until(&w, &r, 2, bs::PUBLISHED, false);
    queued(&w, 3, "d.txt", "d\n");
    until(&w, &r, 3, bs::PUBLISHED, false);
    r.end(nth(&w, 1).pane.as_deref().unwrap(), 0);
    let second = until(&w, &r, 2, "after", true);
    assert_eq!(nth(&w, 3).state, bs::PUBLISHED, "one After at a time");
    assert_eq!(files(&w), "a.txt\nb.txt\nc.txt\nd.txt\ne.txt", "the folder follows the target as it is: batch 2's After installs batch 3's too");
    r.end(second.pane.as_deref().unwrap(), 0);
    until(&w, &r, 3, "after", true);
    let afters: Vec<String> = r.lines.lock().unwrap().iter().filter(|l| l.contains(".after:")).cloned().collect();
    assert_eq!(afters, ["batch-1.1.after: true", "batch-2.1.after: true", "batch-3.1.after: true"]);
}

#[test]
fn ac191_after_over_the_checks_time_fails_and_waits_for_a_human() {
    let w = world("it-late", Settings { check_minutes: 1, ..settings() });
    let r = Sh { hang_on: vec![".after"], ..Default::default() };
    queued(&w, 1, "b.txt", "b\n");
    let first = until(&w, &r, 1, "after", true);
    step(&w.s, &r, &w.root.join("home"), 400).unwrap();
    let first = nth(&w, first.number);
    assert_eq!((first.state.as_str(), first.why.as_deref()), ("after failed", Some("it ran over 1 minutes")));
    assert_eq!(status(&w, &i_of(&w, &first)), "WAIT FOR HUMAN");
}

#[test]
fn ac191_after_failing_while_the_next_batch_is_checked_waits_for_a_human_and_holds_the_later_after() {
    let r = Sh { hang_on: vec![".after", ".check"], ..Default::default() };
    let w = world("it-failed", settings());
    queued(&w, 1, "b.txt", "b\n");
    r.end(until(&w, &r, 1, "checking", true).pane.as_deref().unwrap(), 0);
    let first = until(&w, &r, 1, "after", true);
    let b = queued(&w, 2, "c.txt", "c\n");
    let checking = until(&w, &r, 2, "checking", true);
    r.end(first.pane.as_deref().unwrap(), 2);
    go(&w, &r, 1);
    let first = nth(&w, 1);
    assert_eq!(first.state, "after failed");
    assert_eq!(status(&w, &i_of(&w, &first)), "WAIT FOR HUMAN");
    assert_eq!(nth(&w, 2).state, "checking", "the next batch's check runs on");
    r.end(checking.pane.as_deref().unwrap(), 0);
    until(&w, &r, 2, bs::PUBLISHED, false);
    assert_eq!(status(&w, &b), "DONE", "checked green: published");
    assert!(said(&w, &first).contains("Batch 2 was published on top of batch 1; its After waits for this task's end."), "{}", said(&w, &first));
    queued(&w, 3, "d.txt", "d\n");
    go(&w, &r, 3);
    assert_eq!(nth(&w, 2).state, bs::PUBLISHED, "its After waits for the human's word on batch 1");
    assert_eq!(nth(&w, 1).state, "after failed", "never replaced by a newer batch's outcome");
    assert_eq!(batch(&w).number, 2, "the project's queue builds no batch meanwhile");
    w.s.with(|tx| crate::integrate::keys::close(tx, first.id, "installed by hand", 300)).unwrap();
    until(&w, &r, 2, "after", true);
    assert_eq!(nth(&w, 1).why.as_deref(), Some("installed by hand"));
}

#[test]
fn ac191_after_restarts_terminumd_both_windows_are_found_again_and_the_next_after_waits() {
    let r = Sh { hang_on: vec![".after", ".check"], ..Default::default() };
    let w = world("it-restart", settings());
    queued(&w, 1, "b.txt", "b\n");
    r.end(until(&w, &r, 1, "checking", true).pane.as_deref().unwrap(), 0);
    let first = until(&w, &r, 1, "after", true);
    queued(&w, 2, "c.txt", "c\n");
    let second = until(&w, &r, 2, "checking", true);
    // terminumd stops and starts: the engine keeps nothing between steps, the windows run on in tmux; this proves the
    // step's side of AC-191's restart, a real restart is scripts/e2e_integration.py's (spec §11, to come)
    go(&w, &r, 2);
    assert_eq!((nth(&w, 1).pane, nth(&w, 2).pane), (first.pane.clone(), second.pane.clone()), "both found by their windows, none started again");
    r.end(second.pane.as_deref().unwrap(), 0);
    until(&w, &r, 2, bs::PUBLISHED, false);
    go(&w, &r, 2);
    assert_eq!((nth(&w, 1).pane, nth(&w, 2).state.as_str()), (first.pane.clone(), bs::PUBLISHED), "batch 2's After waits for batch 1's end");
    r.end(first.pane.as_deref().unwrap(), 0);
    until(&w, &r, 2, "after", true);
    assert_eq!(nth(&w, 1).state, "ended");
}

#[test]
fn ac191_a_published_batch_whose_turn_comes_after_integration_was_turned_off_ends_without_its_after() {
    let r = Sh { hang_on: vec![".after"], ..Default::default() };
    let w = after_runs("it-off", &r);
    queued(&w, 2, "c.txt", "c\n");
    until(&w, &r, 2, bs::PUBLISHED, false);
    w.s.with(|tx| { let s = integration::read(tx, "api")?; integration::keep(tx, "api", &Settings { on: false, ..s }) }).unwrap();
    go(&w, &r, 2);
    assert_eq!(nth(&w, 1).state, "after", "a running After goes on");
    r.end(nth(&w, 1).pane.as_deref().unwrap(), 0);
    go(&w, &r, 2);
    let second = nth(&w, 2);
    assert_eq!((second.state.as_str(), second.why.as_deref()), ("ended", Some("After did not run for batch 2: integration was turned off for api")));
    assert_eq!(files(&w), "a.txt\nb.txt\ne.txt", "no git command in its folder once off");
    assert!(!r.lines.lock().unwrap().iter().any(|l| l.starts_with("batch-2.1.after")));
}

#[test]
fn ac191_a_folder_off_its_branch_stays_and_the_batch_s_row_says_after_did_not_run() {
    let w = world("it-folder", Settings { cleanup: "true".into(), ..settings() });
    sh(&w.api, &["checkout", "-q", "-b", "mine"]);
    queued(&w, 1, "b.txt", "b\n");
    sh(&w.api, &["checkout", "-q", "mine"]);
    let r = Sh::default();
    until(&w, &r, 1, "ended", false);
    let first = nth(&w, 1);
    assert_eq!(first.why.as_deref(), Some("After did not run for batch 1: api's folder stays where it is: it is on mine, not main"), "kept through its Cleanup");
    assert!(!r.lines.lock().unwrap().iter().any(|l| l.contains(".after:")), "{:?}", r.lines.lock().unwrap());
    assert!(r.lines.lock().unwrap().iter().any(|l| l.ends_with(".cleanup: true")));
}

#[test]
fn ac191_both_windows_lost_the_check_runs_again_and_after_does_not() {
    let r = Sh { hang_on: vec![".after", ".check"], ..Default::default() };
    let w = world("it-lost", settings());
    let a = queued(&w, 1, "b.txt", "b\n");
    r.end(until(&w, &r, 1, "checking", true).pane.as_deref().unwrap(), 0);
    until(&w, &r, 1, "after", true);
    let b = queued(&w, 2, "c.txt", "c\n");
    until(&w, &r, 2, "checking", true);
    // tmux's server went too: a runner that knows no window
    let fresh = Sh::default();
    go(&w, &fresh, 1);
    let first = nth(&w, 1);
    assert_eq!(first.state, "ended");
    assert!(said(&w, &first).contains("After's outcome is not known for batch 1: its window was lost. After is not run again."), "{}", said(&w, &first));
    until(&w, &fresh, 2, "after", true);
    let lines = fresh.lines.lock().unwrap().clone();
    assert!(lines.iter().any(|l| l.starts_with("batch-2.1.check")) && !lines.iter().any(|l| l.starts_with("batch-1.1.after")), "{lines:?}");
    assert_eq!((status(&w, &a), status(&w, &b)), ("DONE".into(), "DONE".into()));
}
