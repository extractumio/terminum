// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! macOS's protected folders asked for at install and by `terminum folders` (AC-08): terminumd
//! opens each one itself (POST /folders) so macOS asks for terminumd while the user is at the
//! Mac; and when terminum's tmux was started by another program, which macOS judges its
//! terminals by, that it must restart once.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::ctx::Ctx;
use super::{launch, sockhttp};

/// How long the install waits for the answers: nobody may be at the Mac (an upgrade from the
/// phone, over ssh); `terminum folders` waits longer.
const INSTALL_WAIT: Duration = Duration::from_secs(5);
const COMMAND_WAIT: Duration = Duration::from_secs(120);
const SETTINGS: &str = "System Settings › Privacy & Security › Files and Folders";

/// terminumd's answer within `wait` seconds; None when it is down or refuses.
fn ask(ctx: &Ctx, wait: f64) -> Option<Value> {
    let path = format!("/folders?wait={wait}");
    let (status, answer) = sockhttp::call(&ctx.sock(), "POST", &path, None, Duration::from_secs_f64(wait + 5.0)).ok()?;
    answer.filter(|_| status < 400)
}

fn names<'a>(v: &'a Value, states: &[&str]) -> Vec<&'a str> {
    v["folders"].as_array().into_iter().flatten()
        .filter(|r| r["state"].as_str().is_some_and(|s| states.contains(&s)))
        .filter_map(|r| r["name"].as_str()).collect()
}

/// When terminum's tmux was started by another program than the daemon launchd runs: the line
/// that says it must restart once (`started_by` as /health and /folders say it).
pub fn restart_line(ctx: &Ctx, started_by: &Value) -> Option<String> {
    let by = started_by.as_str()?;
    let ours = std::fs::canonicalize(ctx.program()).unwrap_or_else(|_| ctx.program());
    if Path::new(by) == ours {
        return None;
    }
    let who = if by.is_empty() { "a version before this one, or by hand".to_string() } else { ctx.shown(Path::new(by)) };
    Some(format!("terminum's tmux was started by {who}: macOS judges its terminals' folders by that, not by terminum; \
                  restart it once at a quiet moment: tmux -L terminum kill-server (sessions, windows, splits, folders \
                  and projects come back; running programs do not)"))
}

/// What the answers say, decided ones first, then what macOS still asks.
fn report(ctx: &Ctx, v: &Value) {
    let stale = cfg!(target_os = "macos").then(|| restart_line(ctx, &v["tmux_started_by"])).flatten();
    let allowed = names(v, &["allowed"]);
    if !allowed.is_empty() {
        let note = if stale.is_some() { " for terminum; the terminals of the running tmux get them once it restarts" } else { "" };
        ctx.done(&format!("Folders: {} allowed{note}", allowed.join(", ")));
    }
    for name in names(v, &["refused"]) {
        ctx.say(format!("{name}: refused by macOS — allow terminumd in {SETTINGS}"));
    }
    for r in v["folders"].as_array().into_iter().flatten().filter(|r| r["state"] == "error") {
        ctx.say(format!("{}: {}", r["name"].as_str().unwrap_or("?"), r["error"].as_str().unwrap_or("cannot be opened")));
    }
    if let [first, rest @ ..] = &names(v, &["asking", "pending"])[..] {
        let then = if rest.is_empty() { String::new() } else { format!(" (then {})", rest.join(", ")) };
        ctx.say(format!("macOS is still asking about {first}{then}: answer on the Mac's screen; terminum folders shows the answers"));
    }
    if let Some(line) = stale {
        ctx.say(line);
    }
}

/// Asks, says once what macOS is about to ask when it asks anything, and waits up to `wait`.
pub(super) fn ask_and_report(ctx: &Ctx, wait: Duration) -> Result<(), String> {
    let Some(mut v) = ask(ctx, 1.0) else {
        return Err(format!("terminumd did not answer: {}", launch::describe(ctx, None, None)));
    };
    if v["service"] != true {
        ctx.say("terminumd was not started by launchd: macOS would ask for the app it runs in, not for terminum");
        return Ok(());
    }
    if v["done"] != true {
        let all = names(&v, &["allowed", "refused", "asking", "pending", "error"]);
        ctx.say(format!("macOS asks on this Mac's screen whether programs in terminum's terminals may read {} (Allow: asked once)", all.join(", ")));
        let until = Instant::now() + wait;
        while v["done"] != true && Instant::now() < until {
            let left = until.saturating_duration_since(Instant::now()).as_secs_f64().min(10.0);
            match ask(ctx, left) {
                Some(next) => v = next,
                None => break,
            }
        }
    }
    report(ctx, &v);
    Ok(())
}

/// `terminum folders`.
pub fn command(ctx: &Ctx) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        ctx.say("Nothing to ask: only macOS asks for folders");
        return Ok(());
    }
    ask_and_report(ctx, COMMAND_WAIT)
}

/// A build signed with a Developer ID: macOS keeps its answers across upgrades (AC-08).
fn developer_id(binary: &Path) -> bool {
    let mut cmd = std::process::Command::new("/usr/bin/codesign");
    cmd.args(["-dv", "--verbose=2"]).arg(binary);
    launch::run_timeout(cmd, Duration::from_secs(10)).is_ok_and(|o| {
        let info = String::from_utf8_lossy(&o.stderr);
        info.lines().any(|l| l.strip_prefix("TeamIdentifier=").is_some_and(|t| !t.is_empty() && t != "not set"))
    })
}

/// After the install's build answers: a first install asks; an upgrade asks for a Developer ID
/// build only (macOS asks for every ad hoc build again, so it would ask at each one). Only the
/// live agent asks: the launchd test's own one sees the real home folder. Never fails the
/// install.
pub fn at_install(ctx: &Ctx, first: bool) {
    if ctx.direct || ctx.label != super::ctx::LABEL || !cfg!(target_os = "macos") {
        return;
    }
    if !first && !developer_id(&ctx.program()) {
        ctx.say("This build is signed ad hoc: macOS asks for its folders again on first use; terminum folders asks now");
        return;
    }
    if let Err(e) = ask_and_report(ctx, INSTALL_WAIT) {
        ctx.say(format!("Folders not asked for: {e}; terminum folders asks"));
    }
}
