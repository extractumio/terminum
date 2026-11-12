// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;
use crate::dispatcher::subscriptions::{FIVE_HOURS, WEEK};

#[test]
fn ac168_when_a_kind_is_asked() {
    assert!(due(None, 1000, false, false), "at start: at once");
    assert!(!due(Some(1000), 1000 + IDLE_EVERY - 1, false, false), "idle: within the hour, not again");
    assert!(due(Some(1000), 1000 + IDLE_EVERY, false, false), "idle: hourly");
    assert!(!due(Some(1000), 1000 + RUNNING_EVERY - 1, true, false));
    assert!(due(Some(1000), 1000 + RUNNING_EVERY, true, false), "a run working: every ten minutes");
    assert!(due(Some(1000), 1000 + LEAST_GAP, false, true), "a task ended: soon after");
    assert!(!due(Some(1000), 1000 + LEAST_GAP - 1, true, true), "tasks ending one after another: one ask");
}

#[test]
fn ac168_a_task_ended_when_a_run_has_fewer_open_or_is_gone() {
    let m = |v: &[(&str, usize)]| v.iter().map(|(k, n)| (k.to_string(), *n)).collect::<HashMap<_, _>>();
    assert!(!ended(&m(&[]), &m(&[("r1", 1)])), "a run started");
    assert!(!ended(&m(&[("r1", 1)]), &m(&[("r1", 1)])));
    assert!(!ended(&m(&[("r1", 1)]), &m(&[("r1", 2)])), "a task added");
    assert!(ended(&m(&[("r1", 2)]), &m(&[("r1", 1)])), "one of two finished");
    assert!(ended(&m(&[("r1", 1)]), &m(&[])), "its run closed");
}

/// A folder of its own for one test's command.
fn dir(name: &str) -> std::path::PathBuf {
    crate::testutil::temp_root().join(format!("usage-{name}"))
}

/// A shell word: `s` in single quotes.
fn quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[tokio::test]
async fn ac168_claude_s_command_s_output_is_read() {
    let out = json!([{"type": "system", "subtype": "init"}, {"type": "assistant", "usage_report": {"rate_limits": {"limits": [
        {"kind": "session", "percent": 34, "resets_at": "2026-11-12T14:00:00Z", "scope": null},
        {"kind": "weekly_all", "percent": 52, "resets_at": null, "scope": null}]}}}, {"type": "result", "total_cost_usd": 0}]).to_string();
    let got = ask("claude", &format!("printf '%s' {}", quoted(&out)), &dir("claude"), Duration::from_secs(10)).await.unwrap();
    assert_eq!(got, vec![Window { minutes: FIVE_HOURS, used: 34.0, resets_at: Some(1794492000) }, Window { minutes: WEEK, used: 52.0, resets_at: None }]);
}

#[tokio::test]
async fn ac168_codex_s_app_server_is_asked_and_its_input_closed() {
    // a stand-in app-server: the three lines asked for, its answers, then it waits for its input's end
    let answer = json!({"id": 2, "result": {"rateLimits": {"primary": {"usedPercent": 21, "windowDurationMins": 10080, "resetsAt": 1795078866}, "secondary": null}}}).to_string();
    let line = format!("read a; read b; read c; case \"$c\" in *account/rateLimits/read*) ;; *) exit 9;; esac; echo '{{\"id\":1,\"result\":{{}}}}'; echo {}; cat >/dev/null", quoted(&answer));
    let started = std::time::Instant::now();
    let got = ask("codex", &line, &dir("codex"), Duration::from_secs(10)).await.unwrap();
    assert_eq!(got, vec![Window { minutes: WEEK, used: 21.0, resets_at: Some(1795078866) }]);
    assert!(started.elapsed() < Duration::from_secs(5), "its input closed once it answered: {:?}", started.elapsed());
}

#[tokio::test]
async fn ac168_a_failing_command_says_why() {
    let d = dir("fails");
    let why = |line: &'static str| { let d = d.clone(); async move { ask("claude", line, &d, Duration::from_secs(10)).await.unwrap_err() } };
    assert!(why("no-such-program-tm168 -p /usage").await.contains("not found on terminumd's PATH"));
    assert_eq!(why("echo 'Error: not logged in' >&2; exit 1").await, "it ended with exit 1: Error: not logged in");
    assert_eq!(why("echo '[{\"type\":\"result\",\"result\":\"API key\"}]'").await, "no subscription limits in its answer: API key");
    let codex = ask("codex", "exit 0", &d, Duration::from_secs(10)).await.unwrap_err();
    assert_eq!(codex, "it ended without an answer");
}

#[tokio::test]
async fn ac168_a_command_that_hangs_is_ended_with_what_it_started() {
    let d = dir("hangs");
    let pidfile = d.join("child.pid");
    let line = format!("sleep 30 & echo $! > {}; wait", quoted(&pidfile.display().to_string()));
    let started = std::time::Instant::now();
    assert_eq!(ask("claude", &line, &d, Duration::from_millis(500)).await.unwrap_err(), "no answer in 0.5 s");
    assert!(started.elapsed() < Duration::from_secs(4));
    let pid: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    // SAFETY: signal 0 only asks whether the pid is there
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "its child ended with its group");
}

#[test]
fn ac168_the_path_finds_the_agents_installers_folders() {
    let p = path();
    assert!(p.split(':').any(|d| d.ends_with("/.local/bin")) && p.split(':').any(|d| d == "/opt/homebrew/bin"), "{p}");
}
