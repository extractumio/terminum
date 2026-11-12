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

fn open(v: &[(&str, usize)]) -> HashMap<String, usize> {
    v.iter().map(|(k, n)| (k.to_string(), *n)).collect()
}

#[test]
fn ac168_a_task_ended_when_a_run_has_fewer_open_or_is_gone() {
    let m = open;
    assert!(!ended(&m(&[]), &m(&[("r1", 1)])), "a run started");
    assert!(!ended(&m(&[("r1", 1)]), &m(&[("r1", 1)])));
    assert!(!ended(&m(&[("r1", 1)]), &m(&[("r1", 2)])), "a task added");
    assert!(ended(&m(&[("r1", 2)]), &m(&[("r1", 1)])), "one of two finished");
    assert!(ended(&m(&[("r1", 1)]), &m(&[])), "its run closed");
    assert!(!ended(&m(&[("r0", 0)]), &m(&[])), "a session with no task closed: no task ended");
}

/// An ask that ended, as Busy's drop says it.
fn done(k: &mut Kind) {
    k.busy = false;
}

#[test]
fn ac168_a_task_ending_too_soon_after_an_ask_is_asked_for_later() {
    let (mut k, line) = (Kind::default(), Some("u"));
    assert!(k.step(open(&[("r1", 1)]), line, false, 0), "the first beat asks");
    assert!(!k.step(open(&[("r1", 1)]), line, false, 10), "busy");
    done(&mut k);
    assert!(!k.step(open(&[("r1", 0)]), line, false, 30), "its task ended 30 s after the ask: not yet");
    assert!(!k.step(open(&[("r1", 0)]), line, false, 45));
    assert!(k.step(open(&[("r1", 0)]), line, false, 60), "a minute after the ask: that end is asked for");
    done(&mut k);
    assert!(!k.step(open(&[("r1", 0)]), line, false, 200), "then idle: hourly");
    assert!(k.step(open(&[]), line, false, 60 + IDLE_EVERY));
}

#[test]
fn ac168_a_task_ending_during_an_ask_is_asked_for_after_it() {
    let (mut k, line) = (Kind::default(), Some("u"));
    assert!(k.step(open(&[("r1", 2)]), line, false, 0));
    assert!(!k.step(open(&[("r1", 1)]), line, false, 70), "a task ended while it is asked: busy");
    done(&mut k);
    assert!(k.step(open(&[("r1", 1)]), line, false, 100), "asked once that ask is over");
}

#[test]
fn ac168_a_failed_ask_is_tried_again_in_ten_minutes_and_a_changed_line_at_once() {
    let mut k = Kind::default();
    assert!(k.step(open(&[]), Some("u"), false, 0));
    done(&mut k);
    assert!(!k.step(open(&[]), Some("u"), true, RUNNING_EVERY - 1));
    assert!(k.step(open(&[]), Some("u"), true, RUNNING_EVERY), "failed: as often as while working");
    done(&mut k);
    assert!(k.step(open(&[]), Some("v"), false, RUNNING_EVERY + 5), "its line changed: at once");
    done(&mut k);
    assert!(!k.step(open(&[("r1", 1)]), None, false, IDLE_EVERY * 9), "no line: never");
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
async fn ac168_a_command_saying_much_on_its_error_output_still_answers() {
    let out = json!([{"usage_report": {"rate_limits": {"limits": [{"kind": "weekly_all", "percent": 7, "resets_at": null, "scope": null}]}}}]).to_string();
    let line = format!("head -c 300000 /dev/zero | tr '\\0' x >&2; printf '%s' {}", quoted(&out));
    let got = ask("claude", &line, &dir("loud"), Duration::from_secs(10)).await.unwrap();
    assert_eq!(got[0].used, 7.0);
}

#[tokio::test]
async fn ac168_what_a_command_leaves_running_ends_with_its_group() {
    let d = dir("leaves");
    let pidfile = d.join("left.pid");
    let out = json!([{"usage_report": {"rate_limits": {"limits": [{"kind": "weekly_all", "percent": 7, "resets_at": null, "scope": null}]}}}]).to_string();
    let line = format!("sh -c 'trap \"\" TERM; sleep 30' </dev/null >/dev/null 2>&1 & echo $! > {}; printf '%s' {}", quoted(&pidfile.display().to_string()), quoted(&out));
    assert!(ask("claude", &line, &d, Duration::from_secs(10)).await.is_ok());
    let pid: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    // SAFETY: signal 0 only asks whether the pid is there
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "one that ignores SIGTERM ends too");
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
