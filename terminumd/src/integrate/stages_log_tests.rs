// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-198: red Prepare/Verify evidence survives a requeue and restart, including through the real tmux runner.
use super::*;
use crate::integrate::engine::tests::{status, world, queued_with, Sh, World};
use crate::tasks::integration::Settings;
use std::time::{Duration, Instant};

fn wait_for_end(w: &World, r: &dyn Runner) {
    let began = Instant::now();
    loop {
        verify_step(&w.s, r, &w.root.join("home"), 101).unwrap();
        let count = w.s.with(|tx| tx.query_row("SELECT count(*) FROM integration_verifies", [], |r| r.get::<_, i64>(0)).map_err(sql)).unwrap();
        if count == 0 { break }
        assert!(began.elapsed() < Duration::from_secs(30), "Verify did not end");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn attempt(w: &World) -> (String, String) {
    w.s.with(|tx| tx.query_row("SELECT attempt, commit_sha FROM integration_verifies", [], |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)).unwrap()
}

fn red_then_green(r: &dyn Runner, prepare_fails: bool, name: &str) {
    let mut w = world(name, Settings {
        prepare: if prepare_fails { "echo first-prepare-red; exit 2" } else { "echo prepared" }.into(),
        verify: "echo first-verify-red; exit 1".into(), ..Settings::default()
    });
    let home = w.root.join("home");
    let a = queued_with(&w, 1, "b.txt", "b\n", true);
    let legacy = home.join("integration/api/verify-1.log");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, "legacy evidence\n").unwrap();
    verify_step(&w.s, r, &home, 100).unwrap();
    let first = attempt(&w);
    w.s = Store::open(w.root.join("kanban.sqlite"));
    assert_eq!(attempt(&w), first, "a daemon restart keeps the attempt's identity");
    wait_for_end(&w, r);
    assert_eq!(status(&w, &a), "IN PROGRESS");
    let old = integration::verify_log_in(&home, "api", 1, &first.0);
    let red = std::fs::read_to_string(&old).unwrap();
    assert!(red.contains(if prepare_fails { "first-prepare-red" } else { "first-verify-red" }), "{red}");

    // Exercise a second attempt at the same commit and clock tick: neither is a unique run identity.
    w.s.with(|tx| {
        let mut settings = integration::read(tx, "api")?;
        settings.prepare = "echo second-prepare".into();
        settings.verify = "echo second-verify-green".into();
        integration::keep(tx, "api", &settings)?;
        let t = ops::get(tx, &a)?;
        ops::place(tx, crate::tasks::store::HUMAN, &t, "INTEGRATION", 102)?;
        tx.execute("INSERT INTO integration_entries (task, project, commit_sha, result, author, queued_at, stage)
            VALUES (?1, 'api', ?2, 'Outcome: Done.', 'human', 102, 'verifying')", (&t.key, &first.1)).map(drop).map_err(sql)
    }).unwrap();
    verify_step(&w.s, r, &home, 100).unwrap();
    let second = attempt(&w);
    assert_ne!(second.0, first.0);
    wait_for_end(&w, r);
    let new = integration::verify_log_in(&home, "api", 1, &second.0);
    let green = std::fs::read_to_string(&new).unwrap();
    assert!(green.contains("second-prepare") && green.contains("second-verify-green"), "{green}");
    assert_eq!(std::fs::read_to_string(&old).unwrap(), red, "the red run's evidence survives the green run");
    assert_eq!(std::fs::read_to_string(legacy).unwrap(), "legacy evidence\n");
    let history = w.s.with(|tx| ops::history(tx, &a)).unwrap().to_string();
    for path in [&old, &new] {
        assert!(history.matches(&path.display().to_string()).count() >= 2, "start and completion name their own log: {history}");
    }
    let notices = w.s.with(|tx| super::super::notices::since(tx, 0)).unwrap().0;
    assert!(notices.iter().any(|n| n.id == a && n.left.is_some() && n.text.contains(&old.display().to_string())), "the return notice names the red run's log");
}

#[test]
fn ac198_each_prepare_and_verify_attempt_keeps_its_log_after_requeue_and_restart() {
    for prepare_fails in [false, true] {
        red_then_green(&Sh::default(), prepare_fails, if prepare_fails { "verify-logs-prepare" } else { "verify-logs-fake" });
    }
}

#[test]
fn ac198_real_tmux_red_run_log_survives_a_green_requeue() {
    let tmux = crate::testutil::Tmux::start("verify-logs", (120, 30), "sleep 120").expect("tmux is required for AC-198's log regression");
    let runner = super::super::runner::Tmux { tmux: "tmux".into(), socket: tmux.socket.clone(), conf: "/dev/null".into() };
    red_then_green(&runner, false, "verify-logs-live");
}
