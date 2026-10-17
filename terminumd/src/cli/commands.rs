// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The commands that talk to a running terminumd over its socket (AC-08 … AC-10, AC-87):
//! status, open, signout, web, hosts; and import and logs, which work on its files.

use std::time::Duration;

use serde_json::{json, Value};

use super::ctx::Ctx;
use super::{importer, install, launch, sockhttp};

pub const HOST_ACTIONS: [&str; 6] = ["remove", "enable", "files-remove", "off", "on", "retry"];
const MIN_PASSWORD: usize = 8;

/// terminumd refused: its error code and message.
pub struct Refused {
    pub code: Option<String>,
    pub text: String,
}

/// The answer of terminumd; Ok(Err) for a refusal, Err when it is down (with why).
fn call(ctx: &Ctx, method: &str, path: &str, body: Option<Value>) -> Result<Result<Value, Refused>, String> {
    let (status, answer) = sockhttp::call(&ctx.sock(), method, path, body.as_ref(), Duration::from_secs(5))
        .map_err(|_| format!("{} (see: terminum status)", launch::describe(ctx, None, None)))?;
    let code = answer.as_ref().and_then(|a| a.get("error")).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    if status >= 400 || code.is_some() {
        return Ok(Err(Refused { code, text: sockhttp::message(answer.as_ref(), status) }));
    }
    Ok(Ok(answer.filter(Value::is_object).unwrap_or_else(|| json!({}))))
}

fn checked(ctx: &Ctx, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    call(ctx, method, path, body)?.map_err(|r| r.text)
}

// ── status ──────────────────────────────────────────────────────────────────

pub fn uptime(seconds: u64) -> String {
    let (d, h, m, s) = (seconds / 86400, seconds % 86400 / 3600, seconds % 3600 / 60, seconds % 60);
    let parts: Vec<String> = [(d, "d"), (h, "h"), (m, "m")].iter().filter(|(v, _)| *v > 0).map(|(v, u)| format!("{v}{u}")).collect();
    if parts.is_empty() { format!("{s}s") } else { parts.join(" ") }
}

fn plural(n: u64, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "?".into(),
        other => other.to_string(),
    }
}

/// A server's state: a word, or {"state": word, "why": text} as terminumd tells it.
fn state_text(state: &Value) -> String {
    if state.is_object() {
        let word = state["state"].as_str().filter(|s| !s.is_empty()).unwrap_or("?");
        return match state["why"].as_str().filter(|s| !s.is_empty()) {
            Some(why) => format!("{word} ({why})"),
            None => word.into(),
        };
    }
    if state.is_null() || state == "" { "?".into() } else { text(state) }
}

/// Session names (terminumd lists them) or a count.
fn sessions_text(sessions: &Value) -> String {
    match sessions.as_array() {
        Some(list) if list.is_empty() => plural(0, "session"),
        Some(list) => format!("{} ({})", plural(list.len() as u64, "session"), list.iter().map(text).collect::<Vec<_>>().join(", ")),
        None => plural(sessions.as_u64().unwrap_or(0), "session"),
    }
}

pub fn web_line(web: &Value) -> String {
    let enabled = web["enabled"].as_bool().unwrap_or(false);
    if let Some(e) = web["error"].as_str().filter(|e| !e.is_empty()) {
        return format!("web access: {} — {e}", if enabled { "on" } else { "off" });
    }
    if !enabled {
        return "web access: off (terminum web on)".into();
    }
    let urls: Vec<String> = web["urls"].as_array().map(|u| u.iter().map(text).collect()).unwrap_or_default();
    if web["loopback"].as_bool() == Some(true) {
        return format!("web access: on — {} only, for a proxy on this Mac (tailscale serve)", web_addr(web));
    }
    format!("web access: on — {}", if urls.is_empty() { "no address".into() } else { urls.join(", ") })
}

/// The web listener's host as the status gives it; every interface when it does not.
fn web_host(web: &Value) -> &str {
    web["host"].as_str().unwrap_or("0.0.0.0")
}

/// Where a proxy on this Mac reaches the web listener: `host:port`, 127.0.0.1 for every interface.
pub(crate) fn web_addr(web: &Value) -> String {
    let port = match web["port"].as_u64() {
        Some(p) => p.to_string(),
        None => {
            let first = web["urls"].get(0).and_then(Value::as_str).unwrap_or("");
            first.trim_end_matches('/').rsplit_once(':').map_or(crate::auth::web::DEFAULT_PORT.to_string(), |(_, p)| p.to_string())
        }
    };
    match web_host(web) {
        "0.0.0.0" | "::" => format!("127.0.0.1:{port}"),
        h if h.contains(':') => format!("[{h}]:{port}"),
        h => format!("{h}:{port}"),
    }
}

/// The status report from /health (AC-08).
pub fn status_lines(h: &Value) -> Vec<String> {
    let field = |k: &str| if h[k].is_null() { "?".to_string() } else { text(&h[k]) };
    let mut out = vec![
        format!("terminumd:  running, build {}, pid {}, up {}", field("build"), field("pid"), uptime(h["uptime_s"].as_u64().unwrap_or(0))),
        format!("page:       http://127.0.0.1:{}/ (sign in: terminum open)", field("port")),
        web_line(&h["web"]),
    ];
    let tmux_ok = h["tmux"]["ok"].as_bool().unwrap_or(true);
    if !tmux_ok {
        out.push(format!("tmux:       {}", h["tmux"]["message"].as_str().filter(|m| !m.is_empty()).unwrap_or("not available")));
    }
    let servers = h["servers"].as_array().cloned().unwrap_or_default();
    for s in &servers {
        let name = s["name"].as_str().unwrap_or("?");
        out.push(format!("tmux:       {name}: {}, {}, {}", sessions_text(&s["sessions"]), plural(s["panes"].as_u64().unwrap_or(0), "pane"), state_text(&s["state"])));
    }
    if servers.is_empty() && tmux_ok {
        out.push("tmux:       no server yet (a new session starts one)".into());
    }
    for x in h["hosts"].as_array().into_iter().flatten() {
        let files = x["files"].as_str().filter(|f| !f.is_empty()).map(|f| format!(", files {f}")).unwrap_or_default();
        let why = x["why"].as_str().filter(|w| !w.is_empty()).map(|w| format!(" — {w}")).unwrap_or_default();
        out.push(format!("host:       {}: {}{files}{why}", x["name"].as_str().unwrap_or("?"), x["state"].as_str().unwrap_or("?")));
    }
    out.push(format!("pages:      {} connected", h["pages"].as_u64().unwrap_or(0)));
    out
}

pub fn status(ctx: &Ctx) {
    match launch::health(ctx) {
        Some(h) => status_lines(&h).into_iter().for_each(|l| ctx.say(l)),
        None => {
            ctx.say(format!("terminumd:  not running — {}", launch::describe(ctx, None, None)));
            ctx.say(format!("            log: {}", ctx.shown(&ctx.log())));
        }
    }
    if !ctx.direct && let Some(held) = launch::plist_port(ctx) {
        let wanted = launch::port(ctx);
        if wanted != held {
            ctx.say(format!("port:       settings.json says {wanted}, launchd holds {held}: run terminum restart"));
        }
    }
    let link = |n| install::target(ctx, n).unwrap_or_else(|| "-".into());
    ctx.say(format!("builds:     current {}, previous {}", link("current"), link("previous")));
}

// ── open, signout, web ──────────────────────────────────────────────────────

pub fn open_page(ctx: &Ctx, print_only: bool) -> Result<(), String> {
    let answer = checked(ctx, "POST", "/code", None)?;
    let Some(url) = answer["url"].as_str().filter(|u| !u.is_empty()) else { return Err("terminumd gave no link".into()) };
    ctx.say(format!("{url} (valid 60 s, once)"));
    if !print_only {
        let _ = std::process::Command::new(&ctx.opener).arg(url).status();
    }
    Ok(())
}

pub fn signout(ctx: &Ctx) -> Result<(), String> {
    let answer = checked(ctx, "POST", "/signout", Some(json!({"all": true})))?;
    let n = answer["revoked"].as_u64().map(|n| format!(" ({n})")).unwrap_or_default();
    ctx.say(format!("Signed out every page on this Mac{n}. Sign in again with: terminum open"));
    Ok(())
}

fn ask_password(ctx: &Ctx) -> Result<String, String> {
    let pw = match ctx.var("TM_WEB_PASSWORD") {
        Some(pw) => pw.to_string(),
        None => {
            let pw = super::term::read_secret("Web access password (8 or more characters): ")?;
            if super::term::read_secret("Again: ")? != pw {
                return Err("The two passwords differ; nothing changed.".into());
            }
            pw
        }
    };
    if pw.chars().count() < MIN_PASSWORD {
        return Err(format!("The password needs at least {MIN_PASSWORD} characters; nothing changed."));
    }
    Ok(pw)
}

pub fn web(ctx: &Ctx, cmd: Option<&str>) -> Result<(), String> {
    match cmd {
        None | Some("status") => {
            let h = launch::health(ctx).ok_or_else(|| format!("web access: unknown — {}", launch::describe(ctx, None, None)))?;
            ctx.say(web_line(&h["web"]));
            return Ok(());
        }
        Some("password") => {
            checked(ctx, "POST", "/web/password", Some(json!({"password": ask_password(ctx)?})))?;
            ctx.say("web access: password set; browsers signed in before must sign in again");
            return Ok(());
        }
        Some("off") => {
            checked(ctx, "POST", "/web/off", None)?;
            ctx.say("web access: off; every browser is signed out");
            return Ok(());
        }
        _ => {}
    }
    let answer = match call(ctx, "POST", "/web/on", None)? {
        Ok(a) => a,
        Err(e) if e.code.as_deref() != Some("no_password") && !e.text.starts_with("Set a password first") => {
            return Err(format!("web access: not on — {}", e.text));
        }
        Err(_) => {
            ctx.say("Web access needs a password first.");
            checked(ctx, "POST", "/web/password", Some(json!({"password": ask_password(ctx)?})))?;
            call(ctx, "POST", "/web/on", None)?.map_err(|again| format!("web access: not on — {}", again.text))?
        }
    };
    let web = if answer["web"].is_object() { &answer["web"] } else { &answer };
    if web["error"].as_str().is_some_and(|e| !e.is_empty()) || !web["enabled"].as_bool().unwrap_or(true) {
        return Err(web_line(web));
    }
    ctx.say(web_line(web));
    let serve = format!("tailscale serve --bg --https=443 http://{} (it replaces what port 443 serves: see tailscale serve status)", web_addr(web));
    ctx.say(match web_host(web) {
        _ if web["loopback"].as_bool() == Some(true) => format!("To serve it to your tailnet over HTTPS: {serve}"),
        "0.0.0.0" | "::" => format!("Plain HTTP on every network of this Mac. For HTTPS only, set \"host\": \"127.0.0.1\" in web.json (terminum web off, then on) and serve it to your tailnet: {serve}"),
        _ => format!("Plain HTTP: on a network you do not trust, serve it over HTTPS: {serve}"),
    });
    Ok(())
}

// ── hosts ───────────────────────────────────────────────────────────────────

/// A word as a POSIX shell reads it back (Python's shlex.quote).
pub fn quote(word: &str) -> String {
    let safe = |c: char| c.is_alphanumeric() || "_@%+=:,./-".contains(c);
    if !word.is_empty() && word.chars().all(safe) {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', "'\"'\"'"))
}

fn join(words: &[String]) -> String {
    words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")
}

pub fn hosts_list(ctx: &Ctx) -> Result<(), String> {
    let answer = checked(ctx, "GET", "/hosts", None)?;
    let hosts = answer["hosts"].as_array().cloned().unwrap_or_default();
    if hosts.is_empty() {
        ctx.say("No hosts yet: terminum hosts add '<ssh destination and options>' [--files]");
    }
    for x in hosts {
        let recipe = match x["ssh"].as_array() {
            Some(a) => join(&a.iter().map(text).collect::<Vec<_>>()),
            None => x["ssh"].as_str().filter(|s| !s.is_empty()).unwrap_or("?").to_string(),
        };
        let files = x["files"].as_str().filter(|f| !f.is_empty()).unwrap_or("not enabled");
        let helper = x["helper"].as_str().filter(|h| !h.is_empty()).map(|h| format!(", helper {h}")).unwrap_or_default();
        let why = x["why"].as_str().filter(|w| !w.is_empty()).map(|w| format!(" — {w}")).unwrap_or_default();
        ctx.say(format!("{}: {recipe}  {}, files {files}{helper}{why}", x["name"].as_str().unwrap_or("?"), x["state"].as_str().unwrap_or("?")));
    }
    Ok(())
}

/// `hosts add [<name>] '<recipe>' [--files]`: no name, terminumd names it after the destination.
pub fn hosts_add(ctx: &Ctx, name: Option<&str>, recipe: &[String], files: bool) -> Result<(), String> {
    if recipe.is_empty() {
        return Err("usage: terminum hosts add [<name>] '<ssh destination and options>' [--files]".into());
    }
    let text = if recipe.len() == 1 { recipe[0].clone() } else { join(recipe) };
    let answer = checked(ctx, "POST", "/hosts/add", Some(json!({"name": name.unwrap_or(""), "recipe": text, "files": files})))?;
    let added = answer["name"].as_str().or(name).unwrap_or("the host");
    let then = if files { "; its files are set up once it attaches" } else { "" };
    ctx.say(answer["message"].as_str().map_or_else(|| format!("Added {added}; it attaches within seconds (terminum hosts){then}"), str::to_string));
    Ok(())
}

pub fn hosts_action(ctx: &Ctx, action: &str, name: &str) -> Result<(), String> {
    let answer = checked(ctx, "POST", &format!("/hosts/{action}"), Some(json!({"name": name})))?;
    ctx.say(answer["message"].as_str().map_or_else(|| format!("{name}: {action} done"), str::to_string));
    Ok(())
}

/// `terminum tmux`: the attached sessions of the user's own tmux and their state (AC-89).
pub fn tmux_list(ctx: &Ctx) -> Result<(), String> {
    let answer = checked(ctx, "GET", "/tmux", None)?;
    let list = answer["attached"].as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        ctx.say("No session of your tmux is attached: terminum tmux attach <session>");
    }
    for x in list {
        let on = match (x["host"].as_str(), x["socket"].as_str().unwrap_or("default")) {
            (Some(h), _) => format!(" (on {h})"),
            (None, "default") => String::new(),
            (None, socket) => format!(" (tmux -L {socket})"),
        };
        ctx.say(format!("{}{on}: {}", x["session"].as_str().unwrap_or("?"), x["state"].as_str().unwrap_or("?")));
    }
    Ok(())
}

/// `flag`: "socket" or "host" (empty: plain `tmux` here).
pub fn tmux_action(ctx: &Ctx, action: &str, session: &str, flag: &str, value: &str) -> Result<(), String> {
    let mut body = json!({"session": session});
    if !flag.is_empty() {
        body[flag] = json!(value);
    }
    checked(ctx, "POST", &format!("/tmux/{action}"), Some(body))?;
    ctx.say(if action == "attach" { format!("Attached {session}: it shows in the list while it runs") } else { format!("Detached {session}: it goes on running in your tmux") });
    Ok(())
}

// ── import, logs ────────────────────────────────────────────────────────────

/// terminumd holds its state in memory and would write it over the imported files: it is
/// stopped while they are written (under the install lock) and started again.
pub fn import_state(ctx: &Ctx) -> Result<(), String> {
    let _lock = install::lock(ctx)?;
    let h = launch::health(ctx);
    let stopped = |e: String| format!("terminumd could not be stopped or started: {e}");
    if h.is_some() {
        launch::stop(ctx).map_err(stopped)?;
    }
    let lines = importer::run(&ctx.home, &ctx.app_dir, &ctx.root.join("settings.json"));
    if h.is_some() {
        launch::start(ctx).map_err(stopped)?;
    }
    for line in lines.map_err(|e| format!("Not imported: {e}"))? {
        ctx.say(line);
    }
    if let Some(h) = h && !launch::wait_healthy(ctx, h["build"].as_str()) {
        return Err(format!("Imported, but terminumd did not come back: {}", launch::describe(ctx, None, None)));
    }
    Ok(())
}

pub fn logs(ctx: &Ctx, lines: u64, follow: bool) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    if !ctx.log().is_file() {
        return Err(format!("No log yet: {}", ctx.shown(&ctx.log())));
    }
    let mut cmd = std::process::Command::new("/usr/bin/tail");
    cmd.arg("-n").arg(lines.to_string());
    if follow {
        cmd.arg("-f");
    }
    Err(format!("tail: {}", cmd.arg(ctx.log()).exec()))
}
