// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Prepare and Verify before a batch, and Cleanup after one (terminum-integration.md AC-198), on real repositories;
//! the windows stood in for by `sh`.
use super::*;
use crate::integrate::engine::tests::{batch, go, on_origin, queued_with, status, world, Sh};
use crate::tasks::integration::Settings;

#[test]
fn ac198_a_queued_task_is_prepared_and_verified_in_its_own_tree_before_its_batch() {
    let w = world("is-pass", Settings { prepare: "echo prepared > prepared.txt".into(), verify: "test -f prepared.txt".into(), ..Settings::default() });
    let a = queued_with(&w, 1, "b.txt", "b\n", true);
    let r = Sh::default();
    go(&w, &r, 1);
    assert!(r.lines.lock().unwrap().iter().any(|l| l.starts_with("verify-1: (\necho prepared > prepared.txt\n)") && l.contains("test -f prepared.txt")), "{:?}", r.lines.lock().unwrap());
    assert!(w.s.with(|tx| ops::history(tx, &a)).unwrap().to_string().contains("Verifying"));
    go(&w, &r, 4);
    assert_eq!(status(&w, &a), "DONE", "verified, then batched and published");
    assert_eq!(on_origin(&w), "a.txt\nb.txt", "Prepare's file stays in its own tree, never published");
    assert!(!w.root.join("home/integration/api/verify-1").exists(), "its tree removed");
}

#[test]
fn ac198_a_red_verify_returns_the_task_and_no_batch_holds_it() {
    let w = world("is-red", Settings { verify: "exit 1".into(), ..Settings::default() });
    let a = queued_with(&w, 1, "b.txt", "b\n", true);
    go(&w, &Sh::default(), 2);
    assert_eq!(status(&w, &a), "IN PROGRESS");
    let said = w.s.with(|tx| ops::history(tx, &a)).unwrap().to_string();
    assert!(said.contains("returned: red check: its Prepare or Verify ended with status 1"), "{said}");
    assert!(w.s.with(|tx| tx.query_row("SELECT count(*) FROM integration_batches", [], |r| r.get::<_, i64>(0)).map_err(sql)).unwrap() == 0, "no batch");
    assert_eq!(w.s.with(|tx| ops::how_left(tx, &ops::key_of(tx, &a)?)).unwrap().as_deref(), Some("red"));
}

#[test]
fn ac198_prepares_status_2_is_a_conflict() {
    let w = world("is-conflict", Settings { prepare: "exit 2".into(), ..Settings::default() });
    let a = queued_with(&w, 1, "b.txt", "b\n", true);
    go(&w, &Sh::default(), 2);
    assert_eq!(w.s.with(|tx| ops::how_left(tx, &ops::key_of(tx, &a)?)).unwrap().as_deref(), Some("conflict"));
}

#[test]
fn ac198_a_task_taken_out_while_verified_has_its_window_ended() {
    let w = world("is-out", Settings { verify: "true".into(), ..Settings::default() });
    let a = queued_with(&w, 1, "b.txt", "b\n", true);
    let r = Sh { hang: true, ..Default::default() };
    go(&w, &r, 1);
    w.s.with(|tx| { let t = ops::get(tx, &a)?; ops::place(tx, crate::tasks::store::HUMAN, &t, "IN PROGRESS", 150) }).unwrap();
    go(&w, &r, 1);
    assert!(r.ran.lock().unwrap().values().all(|(st, _)| st.is_some()), "its window was ended");
    assert_eq!(w.s.with(|tx| tx.query_row("SELECT count(*) FROM integration_verifies", [], |r| r.get::<_, i64>(0)).map_err(sql)).unwrap(), 0);
    assert!(!w.root.join("home/integration/api/verify-1").exists());
}

#[test]
fn ac198_without_prepare_or_verify_a_task_goes_straight_into_a_batch() {
    let w = world("is-none", Settings::default());
    let a = queued_with(&w, 1, "b.txt", "b\n", false);
    go(&w, &Sh::default(), 2);
    assert_eq!(status(&w, &a), "DONE");
}

#[test]
fn ac198_cleanup_runs_after_after_once_and_its_failure_undoes_nothing() {
    let w = world("is-cleanup", Settings { after: "true".into(), cleanup: "exit 4".into(), ..Settings::default() });
    let a = queued_with(&w, 1, "b.txt", "b\n", false);
    let r = Sh::default();
    go(&w, &r, 6);
    assert_eq!(status(&w, &a), "DONE");
    let b = batch(&w);
    assert_eq!(b.state, "ended");
    assert!(r.lines.lock().unwrap().iter().any(|l| l.ends_with(".cleanup: exit 4")), "{:?}", r.lines.lock().unwrap());
    let said = w.s.with(|tx| ops::history(tx, &bs::slug(tx, b.task.as_deref().unwrap())?)).unwrap().to_string();
    assert!(said.contains("Cleanup ended with status 4; nothing is undone"), "{said}");
}

#[test]
fn ac198_a_verify_that_fails_with_2_through_make_is_red_and_its_commit_not_queued_again() {
    let w = world("is-make", Settings { verify: "exit 2".into(), ..Settings::default() });
    let a = queued_with(&w, 1, "b.txt", "b\n", true);
    go(&w, &Sh::default(), 2);
    assert_eq!(w.s.with(|tx| ops::how_left(tx, &ops::key_of(tx, &a)?)).unwrap().as_deref(), Some("red"), "make's 2 for any failure is no conflict");
    let returned: i64 = w.s.with(|tx| tx.query_row("SELECT count(*) FROM integration_returns", [], |r| r.get(0)).map_err(sql)).unwrap();
    assert_eq!(returned, 1);
}

#[test]
fn ac198_a_lost_verify_window_runs_again() {
    let w = world("is-lost", Settings { verify: "true".into(), ..Settings::default() });
    let a = queued_with(&w, 1, "b.txt", "b\n", true);
    go(&w, &Sh { hang: true, ..Default::default() }, 1);   // started in a runner that then forgets it: lost
    let r = Sh::default();
    go(&w, &r, 4);
    assert_eq!(status(&w, &a), "DONE", "started again, verified, published: {:?}", r.lines.lock().unwrap());
}

#[test]
fn ac198_the_line_keeps_each_stages_text_apart() {
    assert_eq!(line_of("", ""), "");
    assert_eq!(line_of("", "make v # all"), "(\nmake v # all\n) || exit 1");
    let both = line_of("make p", "make v");
    assert!(both.starts_with("(\nmake p\n) || { s=$?; [ \"$s\" -eq 2 ] && exit 2; exit 1; }\n(\nmake v\n) || exit 1"), "{both}");
    for (prepare, verify, code) in [("exit 2", "true", 2), ("exit 3", "true", 1), ("true", "exit 2", 1), ("true", "true # c", 0)] {
        let got = std::process::Command::new("/bin/sh").arg("-c").arg(line_of(prepare, verify)).status().unwrap().code();
        assert_eq!(got, Some(code), "{prepare} / {verify}");
    }
}
