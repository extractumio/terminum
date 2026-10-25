// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! `terminum folders` and the install's ask (AC-08) against a fake terminumd's /folders: what
//! is said for each answer, and when terminum's tmux must restart. No folder is opened here.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{json, Value};

use super::super::folders;
use super::{fixed, said, serve, Home, Route};

fn answers(list: Vec<Value>) -> Route {
    let n = AtomicUsize::new(0);
    Box::new(move |_| (200, list[n.fetch_add(1, Ordering::Relaxed).min(list.len() - 1)].clone()))
}

fn row(name: &str, state: &str) -> Value {
    json!({"name": name, "path": format!("/Users/alex/{name}"), "state": state})
}

#[test]
fn the_restart_line_names_the_program_that_started_tmux_unless_it_is_the_daemon() {
    let home = Home::new();
    let ctx = home.ctx(&[]);
    let ours = ctx.program().display().to_string();
    assert_eq!(folders::restart_line(&ctx, &json!(ours)), None, "started by the daemon launchd runs");
    assert_eq!(folders::restart_line(&ctx, &Value::Null), None, "no tmux runs: nothing to say");
    let before = folders::restart_line(&ctx, &json!("")).unwrap();
    assert!(before.starts_with("terminum's tmux was started by a version before this one, or by hand: ") && before.contains("tmux -L terminum kill-server"), "{before}");
    let dev = home.path("src/target/release/terminumd").display().to_string();
    let other = folders::restart_line(&ctx, &json!(dev)).unwrap();
    assert!(other.starts_with("terminum's tmux was started by ~/src/target/release/terminumd: macOS judges"), "{other}");
}

#[cfg(target_os = "macos")]
#[test]
fn decided_folders_are_said_at_once_allowed_on_one_line_refused_with_where_to_allow() {
    let home = Home::new();
    let ctx = home.ctx(&[]);
    let mut err = row("iCloud Drive", "error");
    err["error"] = json!("Input/output error");
    let answer = json!({"launchd": true, "done": true, "tmux_started_by": ctx.program(),
        "folders": [row("Desktop", "allowed"), row("Documents", "allowed"), row("Downloads", "refused"), err]});
    let _s = serve(&ctx.sock(), HashMap::from([("POST /folders".to_string(), fixed(200, answer))]));
    folders::command(&ctx).unwrap();
    assert_eq!(said(&ctx), [
        "Folders: Desktop, Documents allowed",
        "Downloads: refused by macOS — allow terminumd in System Settings › Privacy & Security › Files and Folders",
        "iCloud Drive: Input/output error",
    ]);
}

#[cfg(target_os = "macos")]
#[test]
fn while_macos_asks_it_says_so_once_then_the_answers_and_that_the_old_tmux_must_restart() {
    let home = Home::new();
    let ctx = home.ctx(&[]);
    let asking = json!({"launchd": true, "done": false, "tmux_started_by": "",
        "folders": [row("Desktop", "allowed"), row("Dropbox", "asking"), row("OneDrive", "pending")]});
    let done = json!({"launchd": true, "done": true, "tmux_started_by": "",
        "folders": [row("Desktop", "allowed"), row("Dropbox", "allowed"), row("OneDrive", "allowed")]});
    let s = serve(&ctx.sock(), HashMap::from([("POST /folders".to_string(), answers(vec![asking.clone(), asking, done]))]));
    folders::command(&ctx).unwrap();
    let lines = said(&ctx);
    assert_eq!(lines[0], "macOS asks on this Mac's screen whether programs in terminum's terminals may read Desktop, Dropbox, OneDrive (Allow: asked once)");
    assert_eq!(lines[1], "Folders: Desktop, Dropbox, OneDrive allowed for terminum; the terminals of the running tmux get them once it restarts");
    assert!(lines[2].starts_with("terminum's tmux was started by a version before this one"), "{lines:?}");
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(s.requests.lock().len(), 3, "it waited until the answers came");
}

#[cfg(target_os = "macos")]
#[test]
fn past_its_wait_it_says_what_macos_still_asks_and_where_to_answer() {
    let home = Home::new();
    let ctx = home.ctx(&[]);
    let asking = json!({"launchd": true, "done": false, "tmux_started_by": ctx.program(),
        "folders": [row("Desktop", "allowed"), row("Dropbox", "asking"), row("OneDrive", "pending")]});
    let _s = serve(&ctx.sock(), HashMap::from([("POST /folders".to_string(), fixed(200, asking))]));
    folders::ask_and_report(&ctx, Duration::from_millis(200)).unwrap();
    assert_eq!(said(&ctx)[1..], [
        "Folders: Desktop allowed".to_string(),
        "macOS is still asking about Dropbox (then OneDrive): answer on the Mac's screen; terminum folders shows the answers".to_string(),
    ]);
}

#[cfg(target_os = "macos")]
#[test]
fn nothing_is_asked_without_launchd_or_on_an_upgrade_to_an_ad_hoc_build() {
    let home = Home::new();
    let ctx = home.ctx(&[]);
    let answer = json!({"launchd": false, "done": true, "folders": [], "tmux_started_by": null});
    let s = serve(&ctx.sock(), HashMap::from([("POST /folders".to_string(), fixed(200, answer))]));
    folders::command(&ctx).unwrap();
    assert_eq!(said(&ctx), ["terminumd was not started by launchd: macOS would ask for the app it runs in, not for terminum"]);
    folders::at_install(&ctx, false);           // daemon/terminumd is no Developer ID build (there is none)
    assert_eq!(said(&ctx)[1], "This build is signed ad hoc: macOS asks for its folders again on first use; terminum folders asks now");
    assert_eq!(s.requests.lock().len(), 1, "the upgrade did not ask terminumd");
    let direct = home.ctx(&[("TM_NO_LAUNCHD", "1")]);
    folders::at_install(&direct, true);
    assert!(said(&direct).is_empty(), "a terminumd run by this command asks nothing");
    let test_agent = home.ctx(&[("TM_LAUNCHD_LABEL", "io.terminum.test-x")]);
    folders::at_install(&test_agent, true);
    assert!(said(&test_agent).is_empty() && s.requests.lock().len() == 1, "nor the launchd test's agent, which sees the real home");
}
