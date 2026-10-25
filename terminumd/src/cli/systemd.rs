// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Running terminumd on Linux (AC-08, AC-14, AC-16): the systemd user service
//! io.terminum.terminumd, started now, at every login (from boot with linger) and after a
//! crash; terminumd binds the page's port itself. `KillMode=process`: a stop ends terminumd
//! alone, never the tmux server it started (which lives in the service's cgroup), so the
//! terminals outlive restarts, upgrades and an uninstall, as under launchd. systemctl is only
//! ever called for the live label when the folders are the real home's, so a test cannot
//! replace the live service.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use super::ctx::{Ctx, LABEL};
use super::launch;

/// The service's unit file.
pub fn unit_path(ctx: &Ctx) -> PathBuf {
    ctx.units.join(format!("{}.service", ctx.label))
}

/// One word of a unit file: quoted, `%` (a specifier), `\` and `"` escaped.
fn word(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%"))
}

/// The unit: terminumd of daemon/ kept running, its environment, its log appended to.
pub fn unit(ctx: &Ctx) -> Result<String, String> {
    let log = ctx.log().display().to_string();
    if log.chars().any(char::is_whitespace) {
        return Err(format!("{log}: systemd cannot append to a path with spaces; set TM_HOME to another folder"));
    }
    let env: String = launch::service_env(ctx).iter().map(|(k, v)| format!("Environment={}\n", word(&format!("{k}={v}")))).collect();
    Ok(format!(
        "# {label}: written by `terminum install`; `terminum restart` writes it again\n\
[Unit]\nDescription=terminum: terminals in the browser\nStartLimitIntervalSec=0\n\n\
[Service]\nExecStart={program} serve\n{env}Restart=always\nRestartSec=1\nKillMode=process\n\
StandardOutput=append:{log}\nStandardError=append:{log}\n\n\
[Install]\nWantedBy=default.target\n",
        label = ctx.label,
        program = word(&ctx.program().display().to_string()),
    ))
}

/// A tool of the user's service manager (systemctl --user, loginctl). The live label only with
/// the real home's folders; never without a user manager to talk to, said in a sentence.
fn tool(ctx: &Ctx, program: &Path, args: &[&str]) -> Result<Output, String> {
    let home = &ctx.real_home;
    let real = program == Path::new(super::ctx::SYSTEM_SYSTEMCTL) || program == Path::new(super::ctx::SYSTEM_LOGINCTL);
    if real && ctx.label == LABEL && (ctx.root != home.join(".terminum") || ctx.units != home.join(".config/systemd/user")) {
        return Err(format!("systemd runs only the install in {}/.terminum; set TM_NO_LAUNCHD=1 for another folder", home.display()));
    }
    let mut cmd = Command::new(program);
    cmd.args(args);
    let name = program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let out = launch::run_timeout(cmd, Duration::from_secs(30)).map_err(|e| format!("{name} {}: {e}", args.join(" ")))?;
    if String::from_utf8_lossy(&out.stderr).contains("Failed to connect to bus") {
        return Err("systemd's user manager is not reachable (no session bus): log in as this user (not su or sudo -u), \
                    or run terminumd yourself with TM_NO_LAUNCHD=1".into());
    }
    Ok(out)
}

fn systemctl(ctx: &Ctx, args: &[&str]) -> Result<Output, String> {
    let mut all = vec!["--user"];
    all.extend_from_slice(args);
    tool(ctx, &ctx.systemctl, &all)
}

/// `args` done, or Err with what systemctl said.
fn must(ctx: &Ctx, args: &[&str]) -> Result<(), String> {
    let out = systemctl(ctx, args)?;
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr }).trim().to_string();
    Err(format!("systemctl --user {}: {}", args.join(" "), if said.is_empty() { format!("{}", out.status) } else { said }))
}

/// Writes the unit with one rename and has systemd read it again when it changed.
fn write_unit(ctx: &Ctx) -> Result<(), String> {
    let data = unit(ctx)?;
    let path = unit_path(ctx);
    if std::fs::read(&path).is_ok_and(|old| old == data.as_bytes()) {
        return Ok(());
    }
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    std::fs::create_dir_all(&ctx.units).map_err(err)?;
    let tmp = ctx.units.join(format!(".{}.service.new", ctx.label));
    std::fs::write(&tmp, &data).map_err(err)?;
    std::fs::rename(&tmp, &path).map_err(err)?;
    must(ctx, &["daemon-reload"])
}

/// terminumd of daemon/ running now and at every login; restarted when it runs (an upgrade,
/// a rollback, `terminum restart`). A build that crashed often is let start again.
pub fn start(ctx: &Ctx) -> Result<(), String> {
    write_unit(ctx)?;
    let name = format!("{}.service", ctx.label);
    let _ = systemctl(ctx, &["reset-failed", &name]);
    must(ctx, &["enable", &name])?;
    must(ctx, &["restart", &name])
}

/// Stops terminumd and removes the service (uninstall). tmux keeps running (KillMode=process).
pub fn stop(ctx: &Ctx) -> Result<(), String> {
    let path = unit_path(ctx);
    if !path.exists() {
        return Ok(());
    }
    must(ctx, &["disable", "--now", &format!("{}.service", ctx.label)])?;
    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    must(ctx, &["daemon-reload"])
}

/// Whether the user manager outlives the user's sessions (linger): None when loginctl cannot
/// say. Without it a logout ends terminumd and the terminals with it.
pub fn lingers(ctx: &Ctx) -> Option<bool> {
    // SAFETY: getuid has no preconditions
    let uid = unsafe { libc::getuid() }.to_string();
    let out = tool(ctx, &ctx.loginctl, &["show-user", &uid, "-p", "Linger", "--value"]).ok()?;
    match String::from_utf8_lossy(&out.stdout).trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

/// The line install and status say while linger is off.
pub fn linger_hint(ctx: &Ctx) -> Option<String> {
    let user = ctx.var("USER").filter(|u| !u.is_empty()).unwrap_or("$USER");
    match lingers(ctx) {
        Some(false) => Some(format!("linger is off: terminumd and its terminals end when you log out; \
                                     to keep them from boot and after logout: sudo loginctl enable-linger {user}")),
        _ => None,
    }
}
