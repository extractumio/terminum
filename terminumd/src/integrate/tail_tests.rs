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

const DIRTY: &str = "api's folder stays where it is: 2 uncommitted files in it; it moves only when git status --porcelain is empty";

/// Batch 1 published, its folder holding the maintainer's work: a change beside the move, which git alone would let
/// through, and a new file; an ignored file, which does not count.
fn dirty(name: &str, r: &Sh) -> (World, Batch) {
    let w = world(name, settings());
    queued(&w, 1, "b.txt", "b\n");
    std::fs::write(w.api.join("a.txt"), "the maintainer's work\n").unwrap();
    std::fs::write(w.api.join("notes.txt"), "mine\n").unwrap();
    std::fs::write(w.api.join(".git/info/exclude"), "build/\n").unwrap();
    std::fs::create_dir(w.api.join("build")).unwrap();
    std::fs::write(w.api.join("build/out"), "x\n").unwrap();
    let first = until(&w, r, 1, bs::AFTER_WAITS, false);
    (w, first)
}

#[test]
fn ac191_a_folder_with_any_uncommitted_change_waits_until_it_is_clean() {
    let r = Sh::default();
    let (w, first) = dirty("it-dirty", &r);
    assert_eq!(first.why.as_deref(), Some(DIRTY));
    assert!(said(&w, &first).contains(&format!("After waits for batch 1: {DIRTY}. It runs again by itself; nothing waits for you.")), "{}", said(&w, &first));
    assert_eq!(status(&w, &i_of(&w, &first)), "DONE", "nothing waits for a human");
    let since = first.waits_since.unwrap();
    at(&w, &r, since + 300);
    assert_eq!((nth(&w, 1).state.as_str(), nth(&w, 1).waits_since), (bs::AFTER_WAITS, Some(since)), "still dirty: it waits on");
    assert_eq!((files(&w).as_str(), std::fs::read_to_string(w.api.join("a.txt")).unwrap().as_str()), ("a.txt", "the maintainer's work\n"));
    assert!(afters(&r).is_empty(), "After does not run on a folder that did not move");
    sh(&w.api, &["checkout", "-q", "--", "a.txt"]);
    std::fs::remove_file(w.api.join("notes.txt")).unwrap();
    at(&w, &r, since + 600);
    assert_eq!(files(&w), "a.txt\nb.txt", "clean, its ignored file kept: it follows on its next try");
    assert_eq!((nth(&w, 1).state.as_str(), afters(&r)), ("ended", vec!["batch-1.1.after".to_string()]));
}

#[test]
fn ac191_a_folder_still_dirty_after_6_hours_fails_for_a_human() {
    let r = Sh::default();
    let (w, first) = dirty("it-dirty-cap", &r);
    at(&w, &r, first.waits_since.unwrap() + 21_900);
    let failed = nth(&w, 1);
    assert_eq!((failed.state.as_str(), failed.why.as_deref()), ("after failed", Some(format!("waited 6 hours: {DIRTY}").as_str())));
    let words = said(&w, &failed);
    assert!(words.contains(&format!("After waited 6 hours for batch 1: {DIRTY}. The batch stays published")) && !words.contains("Its log"), "{words}");
    assert!(afters(&r).is_empty());
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
    assert!(said(&w, &first).contains("After's outcome is not known for batch 1: its window was lost. After is not run again. Check its work: API-"), "{}", said(&w, &first));
    // a human sees it on the board: a P1 Bug task of the project, which holds nothing
    let bug = said(&w, &first).rsplit("Check its work: ").next().unwrap().split('.').next().unwrap().to_string();
    let (t, text) = w.s.with(|tx| Ok((ops::get(tx, &bug)?, ops::text_of(tx, &bug)?))).unwrap();
    assert_eq!((t.title.as_str(), t.severity.as_str(), t.project.as_str(), t.owner_kind.as_str()), ("Bug: After's outcome is not known for batch 1 of api", "P1", "api", "human"));
    assert!(text.contains("After's line:\n\n```\ntrue\n```") && text.contains(&format!("({a})")) && text.contains("closed, or terminum's tmux server ended"), "{text}");
    until(&w, &fresh, 2, "after", true);
    let lines = fresh.lines.lock().unwrap().clone();
    assert!(lines.iter().any(|l| l.starts_with("batch-2.1.check")) && !lines.iter().any(|l| l.starts_with("batch-1.1.after")), "{lines:?}");
    let second = nth(&w, 2);
    assert!(said(&w, &second).contains("Batch 2 is checked again: its window was lost.") && second.red_rounds == 0, "said, never red (AC-190): {}", said(&w, &second));
    assert_eq!((status(&w, &a), status(&w, &b)), ("DONE".into(), "DONE".into()));
}

/// An After that ends 75 with a reason while `../wait` stands in the project's folder's parent: a release, say.
const WAITS: &str = "if [ -f ../wait ]; then echo 'after: a release is running: waits for its end'; exit 75; fi";
const WAIT_WORDS: &str = "after: a release is running: waits for its end";

fn waiting(name: &str) -> World {
    let w = world(name, Settings { after: WAITS.into(), ..settings() });
    std::fs::write(w.root.join("wait"), "").unwrap();
    w
}

/// A try's three steps at `now`: its turn taken, its window started, its status read.
fn at(w: &World, r: &Sh, now: i64) {
    for _ in 0..3 {
        step(&w.s, r, &w.root.join("home"), now).unwrap();
    }
}

fn afters(r: &Sh) -> Vec<String> {
    r.lines.lock().unwrap().iter().filter(|l| l.contains(".after:")).map(|l| l.split(':').next().unwrap().to_string()).collect()
}

#[test]
fn ac191_an_after_that_cannot_run_now_waits_without_a_human_and_runs_again_by_itself() {
    let (w, r) = (waiting("it-wait"), Sh::default());
    let a = queued(&w, 1, "b.txt", "b\n");
    let first = until(&w, &r, 1, bs::AFTER_WAITS, false);
    let i = i_of(&w, &first);
    assert_eq!(first.why.as_deref(), Some(WAIT_WORDS));
    assert_eq!((status(&w, &a), status(&w, &i)), ("DONE".into(), "DONE".into()), "nothing waits for a human");
    let told = format!("After waits for batch 1: {WAIT_WORDS}. It runs again by itself; nothing waits for you.");
    assert!(said(&w, &first).contains(&told), "{}", said(&w, &first));
    let since = first.waits_since.expect("its first wait");
    // a push from outside meanwhile: a try again does not move the folder under what made it wait
    let other = w.root.join("other");
    sh(&w.root, &["clone", "-q", w.origin.to_str().unwrap(), other.to_str().unwrap()]);
    std::fs::write(other.join("z.txt"), "z\n").unwrap();
    sh(&other, &["add", "z.txt"]);
    sh(&other, &["commit", "-qm", "z"]);
    sh(&other, &["push", "-q", "origin", "HEAD:main"]);
    sh(&w.api, &["fetch", "-q", "origin"]);
    at(&w, &r, since + 299);
    assert_eq!(afters(&r).len(), 1, "not within its pause");
    at(&w, &r, since + 300);
    let again = nth(&w, 1);
    assert_eq!((again.state.as_str(), again.waits_since), (bs::AFTER_WAITS, Some(since)), "its first wait kept");
    assert_eq!(said(&w, &again).matches("After waits for batch 1").count(), 1, "told once");
    assert_eq!(files(&w), "a.txt\nb.txt", "the folder stays where its first try put it");
    std::fs::remove_file(w.root.join("wait")).unwrap();
    at(&w, &r, since + 600);
    assert_eq!(nth(&w, 1).state, "ended", "green: the wait ends");
    assert_eq!(status(&w, &i), "DONE");
    assert_eq!(afters(&r), ["batch-1.1.after"; 3]);
}

#[test]
fn ac191_an_after_still_waiting_after_6_hours_fails_for_a_human() {
    let (w, r) = (waiting("it-cap"), Sh::default());
    queued(&w, 1, "b.txt", "b\n");
    let since = until(&w, &r, 1, bs::AFTER_WAITS, false).waits_since.unwrap();
    at(&w, &r, since + 21_599);
    assert_eq!(nth(&w, 1).state, bs::AFTER_WAITS, "under its cap it waits");
    at(&w, &r, since + 21_900);
    let first = nth(&w, 1);
    assert_eq!((first.state.as_str(), first.why.as_deref()), ("after failed", Some(format!("waited 6 hours: {WAIT_WORDS}").as_str())));
    assert_eq!(status(&w, &i_of(&w, &first)), "WAIT FOR HUMAN");
    assert!(said(&w, &first).contains(&format!("After waited 6 hours for batch 1: {WAIT_WORDS}. The batch stays published")), "{}", said(&w, &first));
}

#[test]
fn ac191_the_next_batch_is_published_while_an_after_waits_and_its_after_runs_behind_it() {
    let (w, r) = (waiting("it-wait-next"), Sh::default());
    queued(&w, 1, "b.txt", "b\n");
    let since = until(&w, &r, 1, bs::AFTER_WAITS, false).waits_since.unwrap();
    let b = queued(&w, 2, "c.txt", "c\n");
    until(&w, &r, 2, bs::PUBLISHED, false);
    assert_eq!(status(&w, &b), "DONE", "built, checked and published while batch 1 waits");
    go(&w, &r, 3);
    assert_eq!(nth(&w, 2).state, bs::PUBLISHED, "its After waits behind batch 1's");
    std::fs::remove_file(w.root.join("wait")).unwrap();
    at(&w, &r, since + 300);
    go(&w, &r, 3);
    assert_eq!((nth(&w, 1).state.as_str(), nth(&w, 2).state.as_str()), ("ended", "ended"));
    assert_eq!(afters(&r), ["batch-1.1.after", "batch-1.1.after", "batch-2.1.after"]);
}

#[test]
fn ac191_a_waiting_after_holds_its_project_s_later_afters_and_no_other_project_s() {
    let w = world("it-wait-other", settings());
    w.s.with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('web', 'web', 2, '/nowhere', 1)", []).map_err(sql)?;
        for (project, number, state) in [("api", 7, bs::AFTER_WAITS), ("api", 8, bs::PUBLISHED), ("web", 3, bs::PUBLISHED)] {
            tx.execute("INSERT INTO integration_batches (project, number, tasks, base_sha, state, created_at, step_started) VALUES (?1, ?2, '[]', 'x', ?3, 0, 1000)",
                       rusqlite::params![project, number, state]).map_err(sql)?;
        }
        Ok(())
    }).unwrap();
    let next = |now| w.s.with(|tx| bs::next_after(tx, now, 300)).unwrap().map(|b| (b.project, b.number));
    assert_eq!(next(1100), Some(("web".into(), 3)), "within api's pause: its batch 8 waits behind 7, web's runs");
    assert_eq!(next(1300), Some(("api".into(), 7)), "its pause passed: the oldest is tried again");
}

#[test]
fn ac191_a_failed_after_asks_with_its_last_rows_what_is_held_and_what_close_does() {
    let w = world("it-ask", Settings { after: "echo 'make: *** [install] Error 1'; exit 2".into(), ..settings() });
    queued(&w, 1, "b.txt", "b\n");
    let first = until(&w, &Sh::default(), 1, "after failed", false);
    assert_eq!(first.why.as_deref(), Some("status 2: make: *** [install] Error 1"));
    let asked = said(&w, &first);
    assert!(asked.contains("After failed for batch 1 with status 2: make: *** [install] Error 1. The batch stays published and its tasks DONE. \
                            Held until this task ends: api's next batches are not built, and the After of every batch published after it waits. \
                            Mend the cause, then press After again to run After once more, or do its work by hand and press Close…: api's batches and later Afters go on. Its log: "), "{asked}");
}

/// Batch 1 published with an After that fails while `../broken` stands, ends 75 while `../wait` stands.
fn after_failed(name: &str) -> (World, Sh, Batch) {
    let line = format!("if [ -f ../broken ]; then echo 'disk full'; exit 2; fi; {WAITS}");
    let w = world(name, Settings { after: line, ..settings() });
    std::fs::write(w.root.join("broken"), "").unwrap();
    let r = Sh::default();
    queued(&w, 1, "b.txt", "b\n");
    let first = until(&w, &r, 1, "after failed", false);
    std::fs::remove_file(w.root.join("broken")).unwrap();
    (w, r, first)
}

fn again(w: &World, b: &Batch) -> Result<String, String> {
    w.s.with(|tx| crate::integrate::keys::after_again(tx, b.id, 200))
}

#[test]
fn ac191_after_again_green_ends_the_batch_s_task_without_a_close() {
    let (w, r, first) = after_failed("it-again");
    let i = i_of(&w, &first);
    assert_eq!(crate::integrate::keys::keys_of(&first), ["after-again", "close"]);
    assert_eq!(again(&w, &first).unwrap(), "Batch 1's After runs again.");
    assert_eq!(status(&w, &i), "INTEGRATION", "terminumd's work again");
    let b = queued(&w, 2, "c.txt", "c\n");
    go(&w, &r, 3);
    assert_eq!(nth(&w, 1).state, "ended", "run once more at the tail's next turn, no pause");
    assert_eq!(status(&w, &i), "DONE");
    until(&w, &r, 2, "ended", false);
    assert_eq!(status(&w, &b), "DONE", "the project's queue goes on");
    assert_eq!(afters(&r), ["batch-1.1.after", "batch-1.1.after", "batch-2.1.after"]);
    let words = said(&w, &first);
    assert!(words.contains("After runs again for batch 1, by ") && words.contains("After passed for batch 1 when run again."), "{words}");
}

#[test]
fn ac191_after_again_red_asks_again_and_waiting_stays_in_integration() {
    let (w, r, first) = after_failed("it-again-red");
    let i = i_of(&w, &first);
    std::fs::write(w.root.join("broken"), "").unwrap();
    again(&w, &first).unwrap();
    let red = until(&w, &r, 1, "after failed", false);
    assert_eq!((red.why.as_deref(), status(&w, &i)), (Some("status 2: disk full"), "WAIT FOR HUMAN".to_string()));
    assert_eq!(said(&w, &red).matches("After failed for batch 1 with status 2: disk full.").count(), 2, "each failure kept");
    std::fs::remove_file(w.root.join("broken")).unwrap();
    std::fs::write(w.root.join("wait"), "").unwrap();
    again(&w, &red).unwrap();
    go(&w, &r, 3);
    let waits = nth(&w, 1);
    assert_eq!((waits.why.as_deref(), status(&w, &i)), (Some(WAIT_WORDS), "INTEGRATION".to_string()));
    assert_eq!(again(&w, &waits).unwrap_err(), "Batch 1 is after waits: nothing to decide now.");
    std::fs::remove_file(w.root.join("wait")).unwrap();
    at(&w, &r, waits.step_started.unwrap() + 300);
    assert_eq!((nth(&w, 1).state.as_str(), status(&w, &i)), ("ended", "DONE".to_string()));
}

#[test]
fn ac191_after_again_is_refused_for_a_red_batch() {
    let w = world("it-again-refused", Settings { check: "exit 3".into(), ..settings() });
    queued(&w, 1, "b.txt", "b\n");
    let red = until(&w, &Sh::default(), 1, "red", false);
    assert_eq!(again(&w, &red).unwrap_err(), "Batch 1 is red: After again is for a batch whose After failed.");
}

#[test]
fn ac191_a_waiting_after_ends_without_after_once_integration_is_off_or_its_line_emptied() {
    for (name, off) in [("it-wait-off", true), ("it-wait-empty", false)] {
        let (w, r) = (waiting(name), Sh::default());
        queued(&w, 1, "b.txt", "b\n");
        let since = until(&w, &r, 1, bs::AFTER_WAITS, false).waits_since.unwrap();
        w.s.with(|tx| { let s = integration::read(tx, "api")?; integration::keep(tx, "api", &if off { Settings { on: false, ..s } } else { Settings { after: String::new(), ..s } }) }).unwrap();
        at(&w, &r, since + 300);
        let first = nth(&w, 1);
        let why = off.then_some("After did not run for batch 1: integration was turned off for api");
        assert_eq!((first.state.as_str(), first.why.as_deref()), ("ended", why), "an emptied line: nothing runs after, as for any batch");
        assert_eq!(afters(&r).len(), 1, "not run again");
    }
}
