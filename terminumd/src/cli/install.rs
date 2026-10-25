// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Install, upgrade, roll back and uninstall terminum (AC-08, AC-16).
//!
//! Everything lives under ~/.terminum (0700): bin/, builds/, daemon/, logs/, state/. Each build
//! lives in its own folder builds/<build>/ (a copy of its package: BUILD, agents/<platform>/terminumd,
//! release-signers, and terminumd linked to the agent of this Mac); the links `current` and
//! `previous` name the live and the last build, each switched with one rename. launchd runs
//! daemon/terminumd, a copy of the live build's terminumd renamed into place before every
//! start: macOS knows the binary that started the tmux server by that path, and refuses its
//! programs protected folders unasked once that binary is deleted (AC-08). A build that does
//! not report itself on the socket within 10 s is switched back. bin/terminum links the
//! newest build's terminumd, which is the command when run by that name.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::ctx::Ctx;
use super::launch;

pub const TMUX_HINT: &str = "tmux is needed for terminals: brew install tmux";
const STARTUP_FILES: [&str; 7] = [".zshenv", ".zprofile", ".zshrc", ".bash_profile", ".bashrc", ".profile", ".config/fish/config.fish"];

/// The build a link names, or None.
pub fn target(ctx: &Ctx, name: &str) -> Option<String> {
    std::fs::read_link(ctx.lib().join(name)).ok().map(|p| p.display().to_string())
}

/// Points `link` at `dest` with one rename: readers see the old or the new one, never none.
fn swap_link(link: &Path, dest: &Path) -> Result<(), String> {
    let name = link.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = link.with_file_name(format!(".{name}.new"));
    let err = |e: std::io::Error| format!("{}: {e}", link.display());
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(dest, &tmp).map_err(err)?;
    std::fs::rename(&tmp, link).map_err(err)
}

fn relink(ctx: &Ctx, name: &str, build: &str) -> Result<(), String> {
    swap_link(&ctx.lib().join(name), Path::new(build))
}

fn unlink(ctx: &Ctx, name: &str) {
    let _ = std::fs::remove_file(ctx.lib().join(name));
}

/// The Mac's own architecture: an x86_64 build under Rosetta would say x86_64 on Apple Silicon.
pub fn mac_platform() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        let mut v: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>();
        // SAFETY: a NUL-terminated name and an int-sized buffer with its length
        let r = unsafe { libc::sysctlbyname(c"hw.optional.arm64".as_ptr(), (&raw mut v).cast(), &mut len, std::ptr::null_mut(), 0) };
        if r == 0 {
            return if v == 1 { "macos-aarch64" } else { "macos-x86_64" };
        }
    }
    if cfg!(target_arch = "aarch64") { "macos-aarch64" } else { "macos-x86_64" }
}

/// Copies a package's folders and plain files; a link or any other kind of entry is refused
/// (whatever way the package was unpacked: upgrade, install.sh, make install).
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let (src, dest) = (entry.path(), to.join(entry.file_name()));
        let meta = std::fs::symlink_metadata(&src)?;
        if meta.is_dir() {
            copy_tree(&src, &dest)?;
        } else if meta.is_file() {
            std::fs::copy(&src, &dest)?;
        } else {
            return Err(std::io::Error::other(format!("{}: not a plain file or folder", src.display())));
        }
    }
    Ok(())
}

/// Copies the package into builds/<build> through a hidden folder renamed at the end, so a
/// half-copied build is never linked. The Mac's terminumd is the agent of its architecture;
/// the others stay for remote hosts (AC-83).
fn copy_build(ctx: &Ctx, build: &str, pkg: &Path) -> Result<(), String> {
    let dest = ctx.lib().join(build);
    if dest.is_dir() {
        return Ok(());
    }
    let tmp = ctx.lib().join(format!(".{build}.tmp"));
    let _ = std::fs::remove_dir_all(&tmp);
    let err = |e: std::io::Error| format!("copying the package into {}: {e}", ctx.shown(&dest));
    std::fs::create_dir_all(ctx.lib()).map_err(err)?;
    copy_tree(pkg, &tmp).map_err(err)?;
    std::os::unix::fs::symlink(format!("agents/{}/terminumd", mac_platform()), tmp.join("terminumd")).map_err(err)?;
    std::fs::rename(&tmp, &dest).map_err(err)
}

/// bin/terminum is the command of the newest installed build, not of `current`: after a
/// rollback into an older build it still knows this layout (a rollback never moves it).
fn place_command(ctx: &Ctx, build: &str) -> Result<(), String> {
    std::fs::create_dir_all(ctx.bin()).map_err(|e| format!("{}: {e}", ctx.bin().display()))?;
    swap_link(&ctx.bin().join("terminum"), &ctx.lib().join(build).join("terminumd"))
}

/// Keeps `current` and `previous`; drops other builds.
fn prune(ctx: &Ctx) {
    let keep = [target(ctx, "current"), target(ctx, "previous")];
    let Ok(dir) = std::fs::read_dir(ctx.lib()) else { return };
    for d in dir.flatten() {
        let name = d.file_name().to_string_lossy().into_owned();
        let is_dir = d.file_type().is_ok_and(|t| t.is_dir());
        // a hidden folder is a copy in progress, or one an interrupted install left (.<build>.tmp)
        let stale = name.starts_with('.') && name.ends_with(".tmp");
        if is_dir && (stale || !name.starts_with('.')) && !keep.iter().any(|k| k.as_deref() == Some(name.as_str())) {
            let _ = std::fs::remove_dir_all(d.path());
        }
    }
}

/// One install at a time; the lock goes with the process (and the returned file). The root
/// is private (0700).
pub fn lock(ctx: &Ctx) -> Result<File, String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    std::fs::create_dir_all(ctx.lib()).map_err(|e| format!("{}: {e}", ctx.lib().display()))?;
    let _ = std::fs::set_permissions(&ctx.root, std::fs::Permissions::from_mode(0o700));
    let path = ctx.root.join("install.lock");
    let f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    // SAFETY: flock on a descriptor we own; released when the file closes
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("Another install is running".into());
    }
    Ok(f)
}

/// Makes daemon/ the running build's folder as terminumd sees it beside itself: its
/// terminumd copied in and renamed into place (the path launchd runs stays the same, a
/// running binary is never written into), `agents` and `soundfonts` linked to the build's.
/// Any build, an older one rolled back to too, finds its files there as in its own folder.
fn place_daemon(ctx: &Ctx, build: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let dest = ctx.program();
    let dir = ctx.root.join("daemon");
    let tmp = dir.join(".terminumd.new");
    let err = |e: std::io::Error| format!("placing {build}'s terminumd at {}: {e}", ctx.shown(&dest));
    std::fs::create_dir_all(&dir).map_err(err)?;
    let _ = std::fs::remove_file(&tmp);
    std::fs::copy(ctx.lib().join(build).join("terminumd"), &tmp).map_err(err)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(err)?;
    File::open(&tmp).and_then(|f| f.sync_all()).map_err(err)?;
    std::fs::rename(&tmp, &dest).map_err(err)?;
    for name in ["agents", "soundfonts"] {
        if ctx.lib().join(build).join(name).is_dir() {
            swap_link(&dir.join(name), &Path::new("../builds").join(build).join(name))?;
        } else {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }
    Ok(())
}

/// (Re)starts terminumd as `build` (its copy at daemon/terminumd) and waits until it reports it.
fn go_live(ctx: &Ctx, build: &str) -> Result<(), String> {
    place_daemon(ctx, build)?;
    let mark = launch::log_mark(ctx);
    launch::start(ctx)?;
    if !launch::wait_healthy(ctx, Some(build)) {
        return Err(format!("no answer as {build} within {} s; {}", ctx.wait.as_secs(), launch::describe(ctx, Some(build), Some(mark))));
    }
    Ok(())
}

/// tmux on the PATH terminumd gets.
pub fn tmux_found(ctx: &Ctx) -> bool {
    launch::service_path(ctx).split(':').filter(|d| !d.is_empty()).any(|d| Path::new(d).join("tmux").is_file())
}

/// The first-install greeting: the page opened in the browser, or how to set the password
/// first; and how other devices reach it.
fn open_line(ctx: &Ctx, build: &str) {
    let h = launch::health(ctx).unwrap_or_default();
    let url = super::commands::page_url(&h["port"]);
    if h["password"]["set"].as_bool() == Some(true) {
        ctx.done(&format!("Installed {build}. Open {url} and sign in with your password (terminum open)"));
        if cfg!(target_os = "macos") && ctx.var("TM_NO_OPEN") != Some("1") {
            let _ = std::process::Command::new(&ctx.opener).arg(&url).output();
        }
    } else {
        ctx.done(&format!("Installed {build}. Set the password every browser signs in with: terminum password; then open {url} (terminum open)"));
    }
    ctx.say(super::commands::serve_hint(&h["port"]));
}

/// The `--version` a package's terminumd answers within 10 s.
fn version_of(binary: &Path) -> String {
    let mut cmd = std::process::Command::new(binary);
    cmd.arg("--version");
    match launch::run_timeout(cmd, Duration::from_secs(10)) {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => "unknown (no answer to --version within 10 s)".into(),
    }
}

/// Installs (or upgrades to) the package in folder `pkg`.
pub fn install(ctx: &Ctx, pkg: &Path) -> Result<(), String> {
    let Ok(text) = std::fs::read_to_string(pkg.join("BUILD")) else {
        return Err(format!("{} is not a package (no BUILD): from a checkout, run make install", pkg.display()));
    };
    let build = text.trim().to_string();
    let binary = pkg.join("agents").join(mac_platform()).join("terminumd");
    if !binary.is_file() {
        return Err(format!("the package has no terminumd for {}", mac_platform()));
    }
    let got = version_of(&binary);
    if got != build {
        return Err(format!("{} is build {}, expected {build}", binary.display(), if got.is_empty() { "unknown" } else { &got }));
    }
    let old = target(ctx, "current");
    if old.as_deref() == Some(build.as_str()) && launch::health(ctx).is_some_and(|h| h["build"].as_str() == Some(build.as_str())) {
        place_command(ctx, &build)?;
        ctx.done(&format!("{build} is already installed and running"));
        return Ok(());
    }
    copy_build(ctx, &build, pkg)?;
    let upgrade = old.as_ref().is_some_and(|o| *o != build);
    let before = target(ctx, "previous");
    if let (true, Some(o)) = (upgrade, &old) {
        relink(ctx, "previous", o)?;
    }
    relink(ctx, "current", &build)?;
    if !upgrade {
        place_command(ctx, &build)?; // also when it does not come up: `terminum status` says why
    }
    let logs = ctx.shown(&ctx.log_dir());
    launch::migrate_legacy(ctx)?;
    if let Err(e) = go_live(ctx, &build) {
        let Some(old) = old.filter(|_| upgrade) else {
            return Err(format!("Install of {build} failed ({e}) — see {logs}/"));
        };
        relink(ctx, "current", &old)?;
        match &before {
            Some(b) => relink(ctx, "previous", b)?,
            None => unlink(ctx, "previous"),
        }
        // a build from before the name terminumd runs only as its own agent: that one comes back
        if !ctx.lib().join(&old).join("terminumd").exists() {
            return match launch::restore_legacy(ctx) {
                Ok(true) => Err(format!("Upgrade to {build} failed ({e}); {old} runs again as before — see {logs}/")),
                Ok(false) => Err(format!("Upgrade to {build} failed ({e}); {old} cannot be started by this version: install again — see {logs}/")),
                Err(again) => Err(format!("Upgrade to {build} failed ({e}); {old} did not start again ({again}) — see {logs}/")),
            };
        }
        if let Err(again) = go_live(ctx, &old) {
            return Err(format!("Upgrade to {build} failed ({e}); switched back to {old}, which did not come up either ({again})"));
        }
        return Err(format!("Upgrade to {build} failed ({e}); rolled back to {old} — see {logs}/"));
    }
    place_command(ctx, &build)?;
    launch::forget_legacy(ctx);
    prune(ctx);
    match (&old, upgrade) {
        (Some(o), true) => ctx.done(&format!("Upgraded {o} → {build}")),
        _ => open_line(ctx, &build),
    }
    super::folders::at_install(ctx, !upgrade);
    if !tmux_found(ctx) {
        ctx.say(TMUX_HINT);
    }
    Ok(())
}

/// Is the command reachable already: on this shell's PATH, or set up in a startup file (a
/// shell opened before that edit has not read it, and must not be told to do it again).
pub fn path_ready(ctx: &Ctx) -> bool {
    let bin = ctx.bin().display().to_string();
    if ctx.var("PATH").unwrap_or("").split(':').any(|d| d == bin) {
        return true;
    }
    let mut files: Vec<PathBuf> = STARTUP_FILES.iter().map(|f| ctx.home.join(f)).collect();
    if let Some(z) = ctx.var("ZDOTDIR").filter(|z| !z.is_empty()) {
        files.extend([".zshenv", ".zprofile", ".zshrc"].map(|f| Path::new(z).join(f)));
    }
    files.iter().filter_map(|f| std::fs::read(f).ok()).any(|raw| {
        String::from_utf8_lossy(&raw).lines().any(|l| !l.trim_start().starts_with('#') && l.contains(".terminum/bin"))
    })
}

/// After the result, a blank line and a cornflower-blue tip on how to reach the command,
/// unless the user did that already (a fixed 256-color shade: palette grays vanish in some themes).
pub fn path_tip(ctx: &Ctx) {
    if path_ready(ctx) {
        return;
    }
    let tip = format!(
        "Tip: add {} to your PATH to use the terminum command:\n  echo 'export PATH=\"$HOME/.terminum/bin:$PATH\"' >> ~/.zshrc",
        ctx.bin().display()
    );
    ctx.say("");
    ctx.say(if ctx.color() { format!("\x1b[38;5;69m{tip}\x1b[0m") } else { tip });
}

pub fn rollback(ctx: &Ctx) -> Result<(), String> {
    let (prev, cur) = (target(ctx, "previous"), target(ctx, "current"));
    let Some(prev) = prev.filter(|p| ctx.lib().join(p).is_dir()) else {
        return Err("No previous build to roll back to".into());
    };
    if !ctx.lib().join(&prev).join("terminumd").exists() {
        return Err(format!("{prev} is from before the server was named terminumd and cannot run as this agent: install that version again instead"));
    }
    relink(ctx, "current", &prev)?;
    match &cur {
        Some(c) => relink(ctx, "previous", c)?,
        None => unlink(ctx, "previous"),
    }
    if let Err(e) = go_live(ctx, &prev) {
        // as install does: the build that ran comes back
        let Some(c) = cur.as_deref() else {
            return Err(format!("Rolled back to {prev}, but it did not come up: {e} — see {}/", ctx.shown(&ctx.log_dir())));
        };
        relink(ctx, "current", c)?;
        relink(ctx, "previous", &prev)?;
        let back = go_live(ctx, c).map_or_else(|e2| format!("{c} did not come up either: {e2}"), |_| format!("{c} runs again"));
        return Err(format!("{prev} did not come up: {e}; {back} — see {}/", ctx.shown(&ctx.log_dir())));
    }
    ctx.done(&format!("Rolled back {} → {prev}", cur.as_deref().unwrap_or("-")));
    Ok(())
}

pub fn uninstall(ctx: &Ctx) -> Result<(), String> {
    launch::migrate_legacy(ctx)?;
    launch::stop(ctx)?;
    launch::forget_legacy(ctx);
    let _ = std::fs::remove_dir_all(ctx.lib());
    let _ = std::fs::remove_dir_all(ctx.root.join("daemon"));
    let _ = std::fs::remove_file(ctx.bin().join("terminum"));
    let _ = std::fs::remove_dir(ctx.bin()); // only when empty
    ctx.done(&format!(
        "Removed terminum: terminumd stopped, its launchd agent, builds and command removed. Settings, state and logs kept in {}; tmux sessions untouched.",
        ctx.root.display()
    ));
    Ok(())
}

/// `terminum restart`: the agent loaded again (the port bound anew), then terminumd's answer.
pub fn restart(ctx: &Ctx) -> Result<(), String> {
    match target(ctx, "current") {
        // an install from before daemon/ has none yet: the plist written now names it
        Some(c) => place_daemon(ctx, &c)?,
        None if !ctx.direct => return Err("terminum is not installed (no builds/current): nothing to restart".into()),
        None => {}
    }
    let mark = launch::log_mark(ctx);
    launch::reload(ctx)?;
    let up = launch::wait_healthy(ctx, None);
    match launch::health(ctx).filter(|_| up) {
        Some(h) => {
            ctx.done(&format!("Restarted terminumd: build {}, port {}", h["build"].as_str().unwrap_or("?"), h["port"]));
            Ok(())
        }
        None => Err(format!("terminumd did not answer within {} s: {}", ctx.wait.as_secs(), launch::describe(ctx, None, Some(mark)))),
    }
}
