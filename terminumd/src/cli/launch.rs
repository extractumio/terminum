// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Running terminumd (AC-08, AC-14, AC-16): the launchd agent io.terminum.terminumd, which
//! holds the page's port (socket activation) and starts terminumd now, at every login and
//! after a crash; or, with TM_NO_LAUNCHD=1, a plain background process (direct.rs). Health is
//! asked over the private socket, never over TCP. launchctl is only ever called for the live
//! label when the folders are the real home's, so a test cannot replace the live agent.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::ctx::{Ctx, LABEL, SYSTEM_LAUNCHCTL};
use super::{direct, sockhttp};

/// launchd starts agents with a bare PATH; tmux and ssh live in Homebrew's folders or the system's.
const BASE_PATH: [&str; 6] = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"];

/// The PATH terminumd gets: in direct mode the caller's; under launchd the usual folders,
/// then the installing shell's absolute folders outside the home folder (MacPorts, Nix; not
/// a project's venv or node_modules), each once.
pub fn service_path(ctx: &Ctx) -> String {
    let path = ctx.var("PATH").unwrap_or("");
    if ctx.direct {
        return path.to_string();
    }
    let mut dirs: Vec<&str> = BASE_PATH.to_vec();
    let home = format!("{}/", ctx.home.display());
    for d in path.split(':') {
        if d.starts_with('/') && !d.starts_with(&home) && !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    dirs.join(":")
}

/// Environment for terminumd: PATH, and the folders when they were moved (tests).
pub fn service_env(ctx: &Ctx) -> Vec<(String, String)> {
    let mut env = vec![("PATH".to_string(), service_path(ctx))];
    if ctx.var("TM_HOME").is_some_and(|v| !v.is_empty()) {
        env.push(("TM_HOME".into(), ctx.root.display().to_string()));
    }
    if ctx.var("TM_APP_DIR").is_some_and(|v| !v.is_empty()) {
        env.push(("TM_APP_DIR".into(), ctx.app_dir.display().to_string()));
    }
    env
}

/// The page's port as terminumd reads it from settings.json (48080 by default).
pub fn port(ctx: &Ctx) -> u16 {
    crate::settings::load(&ctx.root.join("settings.json")).0.port
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The agent's plist: terminumd of `current`, kept alive, its log, and launchd's listening
/// socket on 127.0.0.1:<port> handed to it (`Listeners`, see activation.rs).
pub fn plist(ctx: &Ctx, port: u16) -> String {
    let s = |v: &str| format!("<string>{}</string>", xml(v));
    let env: String = service_env(ctx).iter().map(|(k, v)| format!("\t\t<key>{}</key>\n\t\t{}\n", xml(k), s(v))).collect();
    let log = s(&ctx.log().display().to_string());
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n<dict>\n\
\t<key>Label</key>\n\t{label}\n\
\t<key>ProgramArguments</key>\n\t<array>\n\t\t{program}\n\t\t<string>serve</string>\n\t</array>\n\
\t<key>RunAtLoad</key>\n\t<true/>\n\
\t<key>KeepAlive</key>\n\t<true/>\n\
\t<key>ThrottleInterval</key>\n\t<integer>1</integer>\n\
\t<key>EnvironmentVariables</key>\n\t<dict>\n{env}\t</dict>\n\
\t<key>StandardOutPath</key>\n\t{log}\n\
\t<key>StandardErrorPath</key>\n\t{log}\n\
\t<key>Sockets</key>\n\t<dict>\n\t\t<key>Listeners</key>\n\t\t<dict>\n\
\t\t\t<key>SockNodeName</key>\n\t\t\t<string>127.0.0.1</string>\n\
\t\t\t<key>SockServiceName</key>\n\t\t\t<string>{port}</string>\n\
\t\t\t<key>SockFamily</key>\n\t\t\t<string>IPv4</string>\n\
\t\t\t<key>SockType</key>\n\t\t\t<string>stream</string>\n\
\t\t</dict>\n\t</dict>\n\
</dict>\n</plist>\n",
        label = s(&ctx.label),
        program = s(&ctx.program().display().to_string()),
    )
}

/// Whether the plist this build writes differs from the loaded one in more than the PATH,
/// which follows the shell the command runs in (a new PATH waits for `terminum restart`).
pub fn plist_changed(ctx: &Ctx) -> bool {
    let without_path = |t: &str| {
        let mut skip = false;
        t.lines().filter(|l| !std::mem::replace(&mut skip, l.trim() == "<key>PATH</key>") && l.trim() != "<key>PATH</key>").collect::<Vec<_>>().join("\n")
    };
    let old = std::fs::read_to_string(ctx.plist()).unwrap_or_default();
    without_path(&old) != without_path(&plist(ctx, port(ctx)))
}

/// The port the written plist has launchd hold, if there is one.
pub fn plist_port(ctx: &Ctx) -> Option<u16> {
    let text = std::fs::read_to_string(ctx.plist()).ok()?;
    let after = text.split("<key>SockServiceName</key>").nth(1)?;
    after.split("<string>").nth(1)?.split("</string>").next()?.trim().parse().ok()
}

/// The private folders and the log file, made here so they are 0700 and 0600 (launchd
/// would create the log world-readable).
pub fn prepare(ctx: &Ctx) -> Result<(), String> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let err = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    for d in [&ctx.root, &ctx.app_dir, &ctx.log_dir()] {
        std::fs::create_dir_all(d).map_err(|e| err(d, e))?;
        std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700)).map_err(|e| err(d, e))?;
    }
    let log = ctx.log();
    std::fs::OpenOptions::new().append(true).create(true).mode(0o600).open(&log).map_err(|e| err(&log, e))?;
    std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o600)).map_err(|e| err(&log, e))
}

/// A command's output, or why it gave none within `limit` (it is killed then).
pub fn run_timeout(mut cmd: Command, limit: Duration) -> Result<Output, String> {
    let child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let pid = child.id() as libc::pid_t;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(limit) {
        Ok(r) => r.map_err(|e| e.to_string()),
        Err(_) => {
            // SAFETY: signalling the child we started and have not reaped yet
            unsafe { libc::kill(pid, libc::SIGKILL) };
            Err(format!("no answer within {} s", limit.as_secs()))
        }
    }
}

fn domain() -> String {
    // SAFETY: getuid has no preconditions
    format!("gui/{}", unsafe { libc::getuid() })
}

fn target(ctx: &Ctx) -> String {
    format!("{}/{}", domain(), ctx.label)
}

/// launchctl for the agent. The live label is one per user, so the system's launchctl is
/// used for it only with the real home's folders: anything else (a test's home) would
/// replace the live agent. Unit tests give a stub launchctl.
fn launchctl(ctx: &Ctx, args: &[&str]) -> Result<Output, String> {
    let home = &ctx.real_home;
    let system = ctx.launchctl == Path::new(SYSTEM_LAUNCHCTL);
    if system && ctx.label == LABEL && (ctx.root != home.join(".terminum") || ctx.agents != home.join("Library/LaunchAgents")) {
        return Err(format!("launchd runs only the install in {}/.terminum; set TM_NO_LAUNCHD=1 for another folder", home.display()));
    }
    let mut cmd = Command::new(&ctx.launchctl);
    cmd.args(args);
    run_timeout(cmd, Duration::from_secs(30)).map_err(|e| format!("launchctl {}: {e}", args[0]))
}

fn loaded(ctx: &Ctx) -> Result<bool, String> {
    Ok(launchctl(ctx, &["print", &target(ctx)])?.status.success())
}

/// Writes the agent's plist with one rename; true when it changed.
fn write_plist(ctx: &Ctx) -> Result<bool, String> {
    use std::os::unix::fs::PermissionsExt;
    let data = plist(ctx, port(ctx));
    let path = ctx.plist();
    if std::fs::read(&path).is_ok_and(|old| old == data.as_bytes()) {
        return Ok(false);
    }
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    std::fs::create_dir_all(&ctx.agents).map_err(err)?;
    let tmp = ctx.agents.join(format!(".{}.plist.new", ctx.label));
    std::fs::write(&tmp, &data).map_err(err)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).map_err(err)?;
    std::fs::rename(&tmp, &path).map_err(err)?;
    Ok(true)
}

fn bootout(ctx: &Ctx) -> Result<(), String> {
    launchctl(ctx, &["bootout", &target(ctx)])?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while loaded(ctx)? && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    // launchd says unloaded before the old terminumd has exited, and one that bound the port
    // itself (0.5.0's web listener on 48080) would still hold it: launchd's bind at the next
    // bootstrap would then fail for good. Wait until nothing answers on the port.
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port(ctx)));
    let deadline = Instant::now() + Duration::from_secs(10);
    while std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// Loads the agent; launchd may still be unloading the old one for a moment.
fn bootstrap(ctx: &Ctx) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let plist = ctx.plist().display().to_string();
    loop {
        let r = launchctl(ctx, &["bootstrap", &domain(), &plist])?;
        if r.status.success() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let said = String::from_utf8_lossy(if r.stderr.is_empty() { &r.stdout } else { &r.stderr }).trim().to_string();
            let code = r.status.code().map_or("a signal".into(), |c| c.to_string());
            return Err(format!("launchctl bootstrap: {}", if said.is_empty() { format!("exit {code}") } else { said }));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Runs terminumd of `current` now and at every login; restarts it when it runs (the new
/// process resolves `current` again). A loaded agent is only kickstarted, so launchd keeps
/// the port (upgrades, rollbacks); its plist is written and loaded again by `restart` only.
pub fn start(ctx: &Ctx) -> Result<(), String> {
    prepare(ctx)?;
    if ctx.direct {
        direct::stop(ctx);
        return direct::start(ctx);
    }
    if loaded(ctx)? {
        // a build whose plist differs (sockets, arguments) needs it loaded again
        if !plist_changed(ctx) && launchctl(ctx, &["kickstart", "-k", &target(ctx)])?.status.success()
        {
            return Ok(());
        }
        bootout(ctx)?;
    }
    write_plist(ctx)?;
    bootstrap_bound(ctx)
}

/// `terminum restart`: the plist written again and the agent loaded again, so launchd binds
/// the port anew (a changed port, or one another program held at the last bootstrap).
pub fn reload(ctx: &Ctx) -> Result<(), String> {
    prepare(ctx)?;
    if ctx.direct {
        direct::stop(ctx);
        return direct::start(ctx);
    }
    write_plist(ctx)?;
    if loaded(ctx)? {
        bootout(ctx)?;
    }
    bootstrap_bound(ctx)
}

/// What became of a bootstrap: terminumd answers, launchd could not bind the port (the
/// reason terminumd logged), or something else that the caller's health wait reports.
enum Bound {
    Up,
    Unbound(String),
    Unknown,
}

fn bound(ctx: &Ctx, mark: u64) -> Bound {
    let deadline = Instant::now() + ctx.wait;
    loop {
        // the log first: an unbound terminumd never answers, an answer may still be the last one's
        if let Some(why) = log_reason(ctx, Some(mark)) {
            return if why.contains(crate::activation::UNBOUND) { Bound::Unbound(why) } else { Bound::Unknown };
        }
        if health(ctx).is_some() {
            return Bound::Up;
        }
        if Instant::now() >= deadline {
            return Bound::Unknown;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Loads the agent and makes sure launchd could bind the port: when another program held it,
/// launchd hands an unbound socket and never tries again, so that fails here with the reason
/// (launchd binds through a closing connection's TIME_WAIT, measured on macOS 15).
fn bootstrap_bound(ctx: &Ctx) -> Result<(), String> {
    let mark = log_mark(ctx);
    bootstrap(ctx)?;
    match bound(ctx, mark) {
        Bound::Unbound(why) => Err(why),
        _ => Ok(()),
    }
}

/// The agent of installs from before the server was named terminumd: it holds the port
/// through launchd, so it is unloaded before this agent is loaded. Its plist is set aside
/// (not loaded at login) until the new build answers: `forget_legacy` then removes it,
/// `restore_legacy` loads it again when the new build does not come up.
const LEGACY_LABEL: &str = "io.terminum.terminusd";

fn legacy_paths(ctx: &Ctx) -> (PathBuf, PathBuf) {
    (ctx.agents.join(format!("{LEGACY_LABEL}.plist")), ctx.agents.join(format!(".{LEGACY_LABEL}.plist.retired")))
}

/// An install of the old name becomes this one: its agent unloaded (set aside), its log kept
/// in front of terminumd.log. Only `install` and `uninstall` call it, before terminumd starts.
pub fn migrate_legacy(ctx: &Ctx) -> Result<(), String> {
    retire_legacy(ctx)?;
    merge_legacy_log(ctx);
    Ok(())
}

fn retire_legacy(ctx: &Ctx) -> Result<(), String> {
    let (plist, aside) = legacy_paths(ctx);
    if ctx.direct || ctx.label != LABEL || !plist.exists() {
        return Ok(());
    }
    let legacy = format!("{}/{LEGACY_LABEL}", domain());
    launchctl(ctx, &["bootout", &legacy])?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while launchctl(ctx, &["print", &legacy])?.status.success() {
        if Instant::now() > deadline {
            return Err(format!("{LEGACY_LABEL} did not stop: run `launchctl bootout {legacy}`, then install again"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    std::fs::rename(&plist, &aside).map_err(|e| format!("{}: {e}", plist.display()))
}

/// The old name's log goes in front of terminumd.log, so `terminum logs` keeps its history
/// (before terminumd is started: launchd opens the log path anew at each start).
fn merge_legacy_log(ctx: &Ctx) {
    let old = ctx.log_dir().join("terminusd.log");
    let Ok(mut text) = std::fs::read(&old) else { return };
    text.extend(std::fs::read(ctx.log()).unwrap_or_default());
    if crate::config::write_private(&ctx.log(), &text).is_ok() {
        let _ = std::fs::remove_file(&old);
    }
}

/// The new build answers: the old agent's plist and its socket and pid files go.
pub fn forget_legacy(ctx: &Ctx) {
    let _ = std::fs::remove_file(legacy_paths(ctx).1);
    let _ = std::fs::remove_file(ctx.sock().with_file_name("terminusd.sock"));
    let _ = std::fs::remove_file(ctx.root.join("state/terminusd.pid"));
}

/// The new build did not come up: this agent is unloaded and the old one loaded again.
/// True when there was one to bring back.
pub fn restore_legacy(ctx: &Ctx) -> Result<bool, String> {
    let (plist, aside) = legacy_paths(ctx);
    if !aside.exists() {
        return Ok(false);
    }
    if loaded(ctx)? {
        bootout(ctx)?;
    }
    let _ = std::fs::remove_file(ctx.plist());
    std::fs::rename(&aside, &plist).map_err(|e| format!("{}: {e}", aside.display()))?;
    let out = launchctl(ctx, &["bootstrap", &domain(), &plist.display().to_string()])?;
    if !out.status.success() {
        return Err(format!("launchctl bootstrap {}: {}", plist.display(), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(true)
}

/// Stops terminumd and removes the launchd agent (uninstall). tmux keeps running.
pub fn stop(ctx: &Ctx) -> Result<(), String> {
    if ctx.direct {
        direct::stop(ctx);
        return Ok(());
    }
    if loaded(ctx)? {
        bootout(ctx)?;
    }
    match std::fs::remove_file(ctx.plist()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", ctx.plist().display())),
        _ => Ok(()),
    }
}

/// terminumd's /health over the socket, or None while it does not answer.
pub fn health(ctx: &Ctx) -> Option<Value> {
    match sockhttp::call(&ctx.sock(), "GET", "/health", None, Duration::from_secs(1)) {
        Ok((200, Some(h))) if h.is_object() => Some(h),
        _ => None,
    }
}

/// True once terminumd answers on the socket (as `build`, when given).
pub fn wait_healthy(ctx: &Ctx, build: Option<&str>) -> bool {
    let deadline = Instant::now() + ctx.wait;
    loop {
        if let Some(h) = health(ctx) && build.is_none_or(|b| h["build"].as_str() == Some(b)) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The log's size now: later failures are read from what follows it.
pub fn log_mark(ctx: &Ctx) -> u64 {
    std::fs::metadata(ctx.log()).map_or(0, |m| m.len())
}

fn is_error_line(line: &str) -> bool {
    let word = |i: usize, len: usize| {
        let b = line.as_bytes();
        let edge = |c: Option<&u8>| c.is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_'));
        edge(i.checked_sub(1).and_then(|j| b.get(j))) && edge(b.get(i + len))
    };
    line.contains("level=error") || line.contains("panicked") || line.contains("error:") || line.match_indices("ERROR").any(|(i, _)| word(i, 5))
}

/// The text of `msg="…"` in a log line, its escapes kept as written.
fn msg_field(line: &str) -> Option<&str> {
    let start = line.find("msg=\"")? + 5;
    let rest = &line[start..];
    let mut escaped = false;
    for (i, c) in rest.char_indices() {
        match c {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return Some(&rest[..i]),
            _ => escaped = false,
        }
    }
    None
}

/// The last error terminumd logged (after `since`, else in its last 16 KB), e.g.
/// "port 48080 is in use (set `port` in ~/.terminum/settings.json)".
pub fn log_reason(ctx: &Ctx, since: Option<u64>) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(ctx.log()).ok()?;
    let size = f.seek(SeekFrom::End(0)).ok()?;
    let start = since.unwrap_or(size.saturating_sub(16384)).max(size.saturating_sub(65536));
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut raw = Vec::new();
    f.read_to_end(&mut raw).ok()?;
    let text = String::from_utf8_lossy(&raw);
    let line = text.lines().rev().find(|l| is_error_line(l))?;
    Some(msg_field(line).unwrap_or(line.trim()).chars().take(300).collect())
}

/// What runs instead of `build`, for status and failure messages.
pub fn describe(ctx: &Ctx, build: Option<&str>, since: Option<u64>) -> String {
    if let Some(h) = health(ctx) {
        return match build {
            Some(_) => format!("terminumd answers as build {}", h["build"].as_str().unwrap_or("unknown")),
            None => "terminumd answers".into(),
        };
    }
    match log_reason(ctx, since) {
        Some(why) => format!("terminumd does not answer: {why}"),
        None => "terminumd does not answer".into(),
    }
}
