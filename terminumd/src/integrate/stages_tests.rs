// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Prepare and Verify before a batch, and Cleanup after one (terminum-integration.md AC-198), on real repositories;
//! the windows stood in for by `sh`.
use super::*;
use crate::integrate::engine::tests::{batch, go, on_origin, queued_with, status, world, Sh};
use crate::tasks::integration::Settings;

const RETRY: &str = "mkdir -p .checks; printf '%s\\n' 'PASSED ONLY ON RETRY, the first run red: critical: name each with its open task' 'critical first run: expected width 80, got 79' > .checks/retried";

#[test]
fn ac197_verify_keeps_retry_diagnostics_before_removing_its_tree() {
    for exit in [0, 1] {
        let w = world(&format!("is-retry-{exit}"), Settings { verify: format!("{RETRY}; exit {exit}"), ..Settings::default() });
        let a = queued_with(&w, 1, "b.txt", "b\n", true);
        go(&w, &Sh::default(), 2);
        let said = w.s.with(|tx| ops::history(tx, &a)).unwrap().to_string();
        assert!(said.contains("PASSED ONLY ON RETRY") && said.contains("expected width 80, got 79"), "{said}");
        let notices = w.s.with(|tx| super::super::notices::since(tx, 0)).unwrap().0;
        assert!(notices.iter().any(|n| n.id == a && n.text.contains("PASSED ONLY ON RETRY")), "the agent's next hook hears it");
        assert!(!w.root.join("home/integration/api/verify-1").exists(), "diagnostics survive cleanup");
        if exit == 1 {
            assert_eq!(status(&w, &a), "IN PROGRESS", "a retry pass never hides another red step");
        }
    }
}

#[test]
fn ac197_check_keeps_retry_diagnostics_on_the_batch_and_clears_old_notes() {
    let w = world("is-check-retry", Settings { check: RETRY.into(), ..Settings::default() });
    std::fs::write(w.api.join(".git/info/exclude"), ".checks/\n").unwrap();
    let a = queued_with(&w, 1, "b.txt", "b\n", false);
    go(&w, &Sh::default(), 4);
    assert_eq!(status(&w, &a), "DONE");
    let b = batch(&w);
    let id = w.s.with(|tx| bs::slug(tx, b.task.as_deref().unwrap())).unwrap();
    let said = w.s.with(|tx| ops::history(tx, &id)).unwrap().to_string();
    assert!(said.contains("PASSED ONLY ON RETRY") && said.contains("expected width 80, got 79"), "{said}");

    w.s.with(|tx| {
        let mut s = crate::tasks::integration::read(tx, "api")?;
        s.check = "true".into();
        crate::tasks::integration::keep(tx, "api", &s)
    }).unwrap();
    let c = queued_with(&w, 2, "c.txt", "c\n", false);
    go(&w, &Sh::default(), 4);
    assert_eq!(status(&w, &c), "DONE");
    let next = batch(&w);
    let next_id = w.s.with(|tx| bs::slug(tx, next.task.as_deref().unwrap())).unwrap();
    let said = w.s.with(|tx| ops::history(tx, &next_id)).unwrap().to_string();
    assert!(!said.contains("PASSED ONLY ON RETRY"), "old diagnostics do not become this batch's: {said}");
    let kept = w.s.with(|tx| ops::history(tx, &id)).unwrap().to_string();
    assert!(kept.contains("expected width 80, got 79"), "earlier batch history survives tree reuse");
}

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

/// The window `label` ("verify-2") of `r` ended with `code`: a stand-in's handles count its starts, as its lines do.
fn end(r: &Sh, label: &str, code: i32) {
    let at = r.lines.lock().unwrap().iter().position(|l| l.starts_with(&format!("{label}: "))).unwrap_or_else(|| panic!("{label} never started"));
    r.ran.lock().unwrap().get_mut(&format!("%{}", at + 1)).unwrap().0 = Some(code);
}

fn started(r: &Sh) -> Vec<String> {
    r.lines.lock().unwrap().iter().filter_map(|l| l.split(':').next().filter(|s| s.starts_with("verify-")).map(String::from)).collect()
}

fn verifying(w: &crate::integrate::engine::tests::World) -> i64 {
    w.s.with(|tx| tx.query_row("SELECT count(*) FROM integration_verifies WHERE pane IS NOT NULL", [], |r| r.get(0)).map_err(sql)).unwrap()
}

#[test]
fn ac198_three_tasks_verify_at_once_one_started_a_beat_and_a_short_one_never_waits_for_long_ones() {
    let w = world("is-three", Settings { verify: "true".into(), ..Settings::default() });
    let ids: Vec<String> = (1..=4).map(|n| queued_with(&w, n, &format!("f{n}.txt"), "x\n", true)).collect();
    let r = Sh { hang: true, ..Default::default() };
    go(&w, &r, 1);
    assert_eq!(started(&r), ["verify-1"], "one a beat, the oldest queued first");
    go(&w, &r, 4);
    assert_eq!(started(&r), ["verify-1", "verify-2", "verify-3"], "three at once, the fourth waits");
    assert_eq!(verifying(&w), 3);
    end(&r, "verify-3", 0);   // a pass reused: it ends while the two before it still run
    go(&w, &r, 1);
    assert_eq!(started(&r), ["verify-1", "verify-2", "verify-3", "verify-4"], "the fourth starts in the beat the third ends");
    let stage: Option<String> = w.s.with(|tx| tx.query_row("SELECT stage FROM integration_entries e JOIN tasks t ON t.id = e.task WHERE t.slug = ?1", [&ids[2]], |r| r.get(0)).map_err(sql)).unwrap_or(None);
    assert_eq!(stage, None, "{} is ready for a batch at once", ids[2]);
    assert!(r.ran.lock().unwrap().iter().filter(|(h, _)| ["%1", "%2"].contains(&h.as_str())).all(|(_, (st, _))| st.is_none()), "the long ones run on");
    assert_eq!(verifying(&w), 3);
}

#[test]
fn ac198_a_red_verify_beside_another_of_the_same_file_returns_only_its_own_task() {
    let w = world("is-beside", Settings { verify: "true".into(), ..Settings::default() });
    let (a, b) = (queued_with(&w, 1, "b.txt", "ours\n", true), queued_with(&w, 2, "b.txt", "theirs\n", true));
    let r = Sh { hang: true, ..Default::default() };
    go(&w, &r, 2);
    assert_eq!(verifying(&w), 2, "two Verifies of overlapping tasks run at once");
    end(&r, "verify-2", 1);
    go(&w, &r, 1);
    assert_eq!(status(&w, &b), "IN PROGRESS");
    assert_eq!(w.s.with(|tx| ops::how_left(tx, &ops::key_of(tx, &b)?)).unwrap().as_deref(), Some("red"));
    assert_eq!(status(&w, &a), ops::INTEGRATION);
    assert_eq!(verifying(&w), 1, "{a}'s runs on");
    end(&r, "verify-1", 0);
    go(&w, &r, 4);
    assert_eq!(status(&w, &a), "DONE", "passing, it joins a batch and is published alone");
    assert_eq!(on_origin(&w), "a.txt\nb.txt");
}

#[test]
fn ac198_a_restart_follows_two_running_verifies_and_starts_lost_ones_again_once() {
    let w = world("is-restart", Settings { verify: "true".into(), ..Settings::default() });
    let (a, b) = (queued_with(&w, 1, "b.txt", "b\n", true), queued_with(&w, 2, "c.txt", "c\n", true));
    let r = Sh { hang: true, ..Default::default() };
    go(&w, &r, 2);
    // terminumd started again, terminum's tmux kept: the same windows, followed where they are
    go(&w, &r, 3);
    assert_eq!(started(&r), ["verify-1", "verify-2"], "neither started twice");
    assert_eq!(verifying(&w), 2);
    // terminum's tmux gone with the stop: each lost window is started again from its commit, one a beat
    let fresh = Sh { hang: true, ..Default::default() };
    go(&w, &fresh, 1);
    go(&w, &fresh, 3);
    assert_eq!(started(&fresh), ["verify-1", "verify-2"], "each started again once");
    end(&fresh, "verify-1", 0);
    end(&fresh, "verify-2", 0);
    go(&w, &fresh, 4);
    assert_eq!((status(&w, &a), status(&w, &b)), ("DONE".to_string(), "DONE".to_string()), "both passed close together: one batch");
    let said = w.s.with(|tx| ops::history(tx, &a)).unwrap().to_string();
    assert!(said.contains(&format!("{a} is in batch 1 with {b}")), "{said}");
}

#[test]
fn ac198_three_at_once_counts_every_project_of_the_machine() {
    let w = world("is-projects", Settings { verify: "true".into(), ..Settings::default() });
    let ids: Vec<String> = (1..=4).map(|n| queued_with(&w, n, &format!("f{n}.txt"), "x\n", true)).collect();
    w.s.with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place) SELECT 'web', 'web', 2, dir, 1 FROM projects WHERE id = 'api'", []).map_err(sql)?;
        crate::tasks::integration::keep(tx, "web", &Settings { on: true, verify: "true".into(), ..Settings::default() })?;
        for id in &ids[2..] {
            tx.execute("UPDATE integration_entries SET project = 'web' WHERE task = (SELECT id FROM tasks WHERE slug = ?1)", [id]).map_err(sql)?;
        }
        Ok(())
    }).unwrap();
    let r = Sh { hang: true, ..Default::default() };
    go(&w, &r, 6);
    assert_eq!(started(&r).len(), 3, "two of api's and one of web's: {:?}", started(&r));
}

#[test]
fn ac198_a_verify_over_its_time_with_its_waits_counted_is_red() {
    let w = world("is-over", Settings { verify: "true".into(), check_minutes: 5, ..Settings::default() });
    let a = queued_with(&w, 1, "b.txt", "b\n", true);
    let (r, home) = (Sh { hang: true, ..Default::default() }, w.root.join("home"));
    verify_step(&w.s, &r, &home, 100).unwrap();
    verify_step(&w.s, &r, &home, 100 + 5 * 60).unwrap();
    assert_eq!(status(&w, &a), ops::INTEGRATION, "five minutes, a slot's wait among them: still its time");
    verify_step(&w.s, &r, &home, 101 + 5 * 60).unwrap();
    assert_eq!(status(&w, &a), "IN PROGRESS");
    let said = w.s.with(|tx| ops::history(tx, &a)).unwrap().to_string();
    assert!(said.contains("returned: red check: its Prepare or Verify ran over 5 minutes"), "{said}");
}

#[test]
fn ac198_a_run_whose_window_is_gone_and_tree_left_is_removed_and_started_again() {
    let w = world("is-left", Settings { verify: "true".into(), ..Settings::default() });
    let a = queued_with(&w, 1, "b.txt", "b\n", true);
    // as a lost window leaves it when git could not remove its tree then: no window, the row kept
    w.s.with(|tx| tx.execute("INSERT INTO integration_verifies (task, project, commit_sha, pane, tree, started)
                              SELECT task, project, commit_sha, NULL, 'gone', 50 FROM integration_entries", []).map(drop).map_err(sql)).unwrap();
    let r = Sh::default();
    go(&w, &r, 4);
    assert_eq!(started(&r), ["verify-1"], "never stuck verifying");
    assert_eq!(status(&w, &a), "DONE");
}

#[test]
fn ac146_a_verify_that_skipped_timed_checks_passes_and_says_so_to_its_task() {
    let skip = "mkdir -p .checks; echo 'targeted: SKIPPED timed checks, the machine busy for 15 minutes: critical' > .checks/skipped";
    for exit in [0, 1] {
        let w = world(&format!("is-skip-{exit}"), Settings { verify: format!("{skip}; exit {exit}"), ..Settings::default() });
        let a = queued_with(&w, 1, "b.txt", "b\n", true);
        go(&w, &Sh::default(), 3);
        let said = w.s.with(|tx| ops::history(tx, &a)).unwrap().to_string();
        let told = said.contains("passed without the timed checks a busy machine skipped (AC-146)") && said.contains("SKIPPED timed checks, the machine busy");
        assert_eq!(told, exit == 0, "said on a pass, a red one says its own: {said}");
    }
}
