// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A published batch's After beside the next batch's Check (terminum-integration.md AC-188, AC-191) on real
//! repositories in a temporary folder; the lines' windows stood in for by `sh`, an After or a Check held running.
use crate::integrate::engine::step;
use crate::integrate::engine::tests::{batch, go, on_origin, queued, sh, status, world, Sh, World};
use crate::integrate::store::{self as bs, Batch};
use crate::tasks::integration::Settings;
use crate::tasks::ops::{self, sql};

/// Batch `n` of the test's project.
fn nth(w: &World, n: i64) -> Batch {
    w.s.with(|tx| tx.query_row("SELECT id FROM integration_batches WHERE number = ?1", [n], |r| r.get::<_, i64>(0)).map_err(sql).and_then(|id| bs::by_id(tx, id))).unwrap()
}

/// Steps until batch `n` stands in `state` with its line's window open (`window`) or not; at most 20.
fn until(w: &World, r: &Sh, n: i64, state: &str, window: bool) -> Batch {
    for _ in 0..20 {
        if w.s.with(|tx| tx.query_row("SELECT count(*) FROM integration_batches WHERE number = ?1", [n], |r| r.get::<_, i64>(0)).map_err(sql)).unwrap() == 1 {
            let b = nth(w, n);
            if b.state == state && b.pane.is_some() == window {
                return b;
            }
        }
        go(w, r, 1);
    }
    panic!("batch {n} never stood {state}: {:?}", w.s.with(|tx| bs::by_id(tx, batch(w).id)).unwrap());
}

fn files(w: &World) -> String {
    sh(&w.api, &["ls-files"])
}

/// Its integration task's ID.
fn i_of(w: &World, b: &Batch) -> String {
    w.s.with(|tx| bs::slug(tx, b.task.as_deref().unwrap())).unwrap()
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
    let second = until(&w, &r, 2, bs::PUBLISHED, false);
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
    assert_eq!(second.number, 2);
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
    assert_eq!(w.s.with(|tx| tx.query_row("SELECT count(*) FROM integration_batches", [], |r| r.get::<_, i64>(0)).map_err(sql)).unwrap(), 2, "the project's queue builds no batch meanwhile");
    w.s.with(|tx| crate::integrate::keys::close(tx, first.id, "installed by hand", 300)).unwrap();
    until(&w, &r, 2, "after", true);
    assert_eq!(nth(&w, 1).why.as_deref(), Some("installed by hand"));
}

#[test]
fn ac191_terminumd_restarting_finds_both_windows_and_follows_each() {
    let r = Sh { hang_on: vec![".after", ".check"], ..Default::default() };
    let w = world("it-restart", settings());
    queued(&w, 1, "b.txt", "b\n");
    r.end(until(&w, &r, 1, "checking", true).pane.as_deref().unwrap(), 0);
    let first = until(&w, &r, 1, "after", true);
    queued(&w, 2, "c.txt", "c\n");
    let second = until(&w, &r, 2, "checking", true);
    // terminumd stops and starts: it keeps nothing between steps, the windows run on in tmux
    go(&w, &r, 2);
    assert_eq!((nth(&w, 1).pane, nth(&w, 2).pane), (first.pane.clone(), second.pane.clone()), "both found by their windows, none started again");
    r.end(first.pane.as_deref().unwrap(), 0);
    r.end(second.pane.as_deref().unwrap(), 0);
    go(&w, &r, 1);
    assert_eq!((nth(&w, 1).state.as_str(), nth(&w, 2).state.as_str()), ("ended", "green"), "each read its status in one step");
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
