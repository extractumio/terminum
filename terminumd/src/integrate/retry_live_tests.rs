// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-197 through the real integration tmux runner and check_changed.py, with a private repo/server/store.
use std::path::Path;
use std::time::{Duration, Instant};

use super::engine::{self, tests::{batch, queued_with, status, world}};
use super::store as bs;
use crate::tasks::{integration::Settings, ops};

#[test]
fn ac197_real_retry_in_verify_and_check_reaches_durable_history() {
    let tmux = crate::testutil::Tmux::start("retry-history", (120, 30), "sleep 120").expect("tmux is required for AC-197's real-runner check");
    let runner = super::runner::Tmux { tmux: "tmux".into(), socket: tmux.socket.clone(), conf: "/dev/null".into() };
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("scripts");
    let command = format!("ln -s {} scripts && python3 retry.py", crate::cli::commands::quote(&scripts.to_string_lossy()));
    let w = world("retry-live", Settings { verify: command.clone(), check: command, ..Settings::default() });
    let script = r#"import sys
from pathlib import Path
sys.path.insert(0, str(Path.cwd() / 'scripts'))
import check_changed
step = 'if test -f .checks/attempted; then exit 0; fi; touch .checks/attempted; echo first-run-example; exit 1'
raise SystemExit(bool(check_changed.run([('critical', step)], root=Path.cwd(), gate=False)))
"#;
    let a = queued_with(&w, 1, "retry.py", script, true);
    let started = Instant::now();
    loop {
        engine::step(&w.s, &runner, &w.root.join("home"), 100 + started.elapsed().as_secs() as i64).unwrap();
        if status(&w, &a) == "DONE" { break }
        assert!(started.elapsed() < Duration::from_secs(120), "integration did not finish: {}", w.s.with(|tx| ops::history(tx, &a)).unwrap());
        std::thread::sleep(Duration::from_millis(100));
    }
    let verify = w.s.with(|tx| ops::history(tx, &a)).unwrap().to_string();
    assert!(verify.contains("PASSED ONLY ON RETRY") && verify.contains("first-run-example"), "{verify}");
    assert!(!w.root.join("home/integration/api/verify-1").exists());
    let b = batch(&w);
    let id = w.s.with(|tx| bs::slug(tx, b.task.as_deref().unwrap())).unwrap();
    let check = w.s.with(|tx| ops::history(tx, &id)).unwrap().to_string();
    assert!(check.contains("PASSED ONLY ON RETRY") && check.contains("first-run-example"), "{check}");
    assert!(check.contains("> critical: the first run's last lines:"), "tails are check data");
    // tmux's guard ends only this test's server on success or panic; temp_root removes the repo/store.
}
