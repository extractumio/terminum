// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-08 / AC-16: install, upgrade, rollback and uninstall in a temporary home, terminumd run
//! directly (TM_NO_LAUNCHD=1) as a fake that answers on the socket.

use std::path::PathBuf;
use std::time::Duration;

use super::*;
use crate::cli::{direct, install, launch};

/// A home with a direct-mode context whose terminumd is stopped at the end.
struct T {
    home: Home,
    ctx: Ctx,
}

impl T {
    fn new() -> T {
        let home = Home::new();
        let tbin = home.path("tbin");
        write_exec(&tbin.join("tmux"), "#!/bin/sh\n");
        let path = format!("{}:/usr/bin:/bin", tbin.display());
        let ctx = home.ctx(&[("TM_NO_LAUNCHD", "1"), ("TM_NO_OPEN", "1"), ("PATH", &path)]);
        T { home, ctx }
    }
    fn install(&self, build: &str) -> Result<(), String> {
        install::install(&self.ctx, &package(&self.home, build, None, false))
    }
    fn link(&self, name: &str) -> Option<String> {
        install::target(&self.ctx, name)
    }
    fn running(&self) -> Option<String> {
        launch::health(&self.ctx).and_then(|h| h["build"].as_str().map(str::to_string))
    }
    fn command(&self) -> PathBuf {
        std::fs::read_link(self.ctx.bin().join("terminum")).unwrap()
    }
    /// daemon/ is `build`'s: its terminumd a copy (a file, not a link) and its agents linked.
    fn daemon_is(&self, build: &str) -> bool {
        let d = self.ctx.root.join("daemon");
        let file = std::fs::symlink_metadata(d.join("terminumd")).is_ok_and(|m| m.is_file());
        let same = std::fs::read(d.join("terminumd")).ok() == std::fs::read(self.ctx.lib().join(build).join("terminumd")).ok();
        let agents = std::fs::read_link(d.join("agents")).ok() == Some(PathBuf::from(format!("../builds/{build}/agents")));
        file && same && agents && mode(&d.join("terminumd")) == 0o755 && !d.join(".terminumd.new").exists()
    }
}

impl Drop for T {
    fn drop(&mut self) {
        direct::stop(&self.ctx);
    }
}

#[test]
fn fresh_install_upgrade_same_build_and_prune() {
    let t = T::new();
    let root = &t.ctx.root;
    t.install("a").unwrap();
    assert_eq!((t.link("current").as_deref(), t.running().as_deref()), (Some("a"), Some("a")));
    assert!(said(&t.ctx).contains(&"Installed a. Open http://127.0.0.1:48080/ and sign in with your password (terminum open)".to_string()));
    assert!(said(&t.ctx).contains(&"Other devices: serve it over HTTPS on your tailnet: tailscale serve --bg --https=443 http://127.0.0.1:48080 (it takes the Mac's name; a name of its own: README \"Over Tailscale\")".to_string()));
    assert!(std::fs::read_link(root.join("builds/a/terminumd")).unwrap().to_string_lossy().starts_with("agents/macos-"), "the Mac's own terminumd");
    assert!(root.join("builds/a/agents/linux-x86_64/terminumd").is_file(), "the helpers for hosts come along");
    assert_eq!(t.command(), root.join("builds/a/terminumd"), "the command is the build's terminumd");
    assert!(t.daemon_is("a"), "terminumd runs from daemon/, a's copy");
    assert!(root.join("daemon/agents/linux-x86_64/terminumd").is_file(), "the build's files beside it");
    assert_eq!(mode(root), 0o700);
    assert_eq!(mode(&root.join("logs/terminumd.log")), 0o600);
    assert_eq!(mode(&root.join("state/terminumd.pid")), 0o600);
    let first = launch::health(&t.ctx).unwrap()["pid"].clone();
    t.install("b").unwrap();
    assert_eq!((t.link("current"), t.link("previous"), t.running()), (Some("b".into()), Some("a".into()), Some("b".into())));
    assert!(said(&t.ctx).contains(&"Upgraded a → b".to_string()));
    assert!(t.daemon_is("b"), "the same path, now b's");
    let pid = launch::health(&t.ctx).unwrap()["pid"].clone();
    assert_ne!(pid, first, "the old terminumd was replaced");
    t.install("b").unwrap();
    assert_eq!(launch::health(&t.ctx).unwrap()["pid"], pid, "same build, running: nothing restarted");
    assert!(said(&t.ctx).contains(&"b is already installed and running".to_string()));
    t.install("c").unwrap();
    let mut builds: Vec<String> = std::fs::read_dir(root.join("builds")).unwrap().flatten()
        .filter(|d| d.file_type().unwrap().is_dir() && !d.file_name().to_string_lossy().starts_with('.'))
        .map(|d| d.file_name().to_string_lossy().into_owned()).collect();
    builds.sort();
    assert_eq!(builds, ["b", "c"], "only current and previous are kept");
    assert!(t.daemon_is("c"), "the path launchd runs outlives every pruned build");
}

#[test]
fn a_build_that_does_not_come_up_is_rolled_back() {
    let t = T::new();
    t.install("z").unwrap();
    t.install("a").unwrap();
    let msg = install::install(&t.ctx, &package(&t.home, "b", None, true)).unwrap_err();
    assert!(msg.starts_with("Upgrade to b failed (no answer as b within 3 s; terminumd does not answer: port 48080 is in use"), "{msg}");
    assert!(msg.ends_with("rolled back to a — see ~/.terminum/logs/"), "{msg}");
    assert_eq!((t.link("current"), t.link("previous"), t.running()), (Some("a".into()), Some("z".into()), Some("a".into())), "the build before stays the one to roll back to");
    assert_eq!(t.command(), t.ctx.root.join("builds/a/terminumd"), "the command stays the working build's");
    assert!(t.ctx.root.join("builds/b").is_dir(), "kept for a look; pruned by the next good install");
    assert!(t.daemon_is("a"), "the build that runs again is the one at daemon/");
    install::rollback(&t.ctx).unwrap();
    assert_eq!(t.running().as_deref(), Some("z"));
    assert!(t.daemon_is("z"), "a rollback puts the older build there");
}

#[test]
fn a_first_install_that_does_not_come_up_says_so() {
    let t = T::new();
    let msg = install::install(&t.ctx, &package(&t.home, "a", None, true)).unwrap_err();
    assert!(msg.contains("Install of a failed") && msg.contains("port 48080 is in use"), "{msg}");
    assert!(t.ctx.bin().join("terminum").exists(), "the command is there to say why");
}

#[test]
fn tmux_missing_is_said_and_the_install_goes_through() {
    let home = Home::new();
    let ctx = home.ctx(&[("TM_NO_LAUNCHD", "1"), ("TM_NO_OPEN", "1"), ("PATH", "/usr/bin:/bin")]);
    let t = T { home, ctx };
    t.install("a").unwrap();
    assert_eq!(said(&t.ctx).last().map(String::as_str), Some(install::TMUX_HINT));
    assert_eq!(t.running().as_deref(), Some("a"));
}

#[test]
fn tmux_is_looked_for_on_the_path_terminumd_gets() {
    let home = Home::new();
    let bin = home.path("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let path = bin.display().to_string();
    let direct = home.ctx(&[("TM_NO_LAUNCHD", "1"), ("PATH", &path)]);
    assert!(!install::tmux_found(&direct));
    write_exec(&bin.join("tmux"), "#!/bin/sh\n");
    assert!(install::tmux_found(&direct));
    let launchd = home.ctx(&[("PATH", &path)]);
    assert!(!launch::service_path(&launchd).contains(&path), "a folder in the home is not given to launchd's terminumd");
}

#[test]
fn wrong_binary_is_refused_before_anything_changes() {
    let t = T::new();
    let msg = install::install(&t.ctx, &package(&t.home, "b", Some("other"), false)).unwrap_err();
    assert!(msg.contains("is build other, expected b"), "{msg}");
    assert_eq!(t.link("current"), None);
    assert!(!t.ctx.bin().join("terminum").exists());
}

#[test]
fn not_a_package() {
    let t = T::new();
    let msg = install::install(&t.ctx, &t.home.dir).unwrap_err();
    assert!(msg.contains("not a package"), "{msg}");
}

#[test]
fn rollback_and_nothing_to_roll_back() {
    let t = T::new();
    assert!(install::rollback(&t.ctx).unwrap_err().contains("No previous build"));
    t.install("a").unwrap();
    t.install("b").unwrap();
    install::rollback(&t.ctx).unwrap();
    assert_eq!((t.link("current"), t.link("previous"), t.running()), (Some("a".into()), Some("b".into()), Some("a".into())));
    assert!(said(&t.ctx).contains(&"Rolled back b → a".to_string()));
    assert_eq!(t.command(), t.ctx.root.join("builds/b/terminumd"), "after a rollback the command is still the newest build's");
}

#[test]
fn a_build_of_the_old_name_is_not_rolled_back_to() {
    let t = T::new();
    t.install("a").unwrap();
    t.install("b").unwrap();
    let old = t.ctx.root.join("builds/a");
    std::fs::rename(old.join("terminumd"), old.join("terminusd")).unwrap();
    let msg = install::rollback(&t.ctx).unwrap_err();
    assert!(msg.contains("a is from before the server was named terminumd"), "{msg}");
    assert_eq!((t.link("current"), t.running()), (Some("b".into()), Some("b".into())), "nothing changed");
}

#[test]
fn a_half_copied_build_is_never_linked() {
    let t = T::new();
    std::fs::create_dir_all(t.ctx.root.join("builds/.b.tmp/agents")).unwrap();
    t.install("b").unwrap();
    assert!(t.ctx.root.join("builds/b/terminumd").is_file());
    assert!(!t.ctx.root.join("builds/.b.tmp").exists());
}

#[test]
fn the_root_is_private_and_one_install_runs_at_a_time() {
    use std::os::unix::fs::PermissionsExt;
    let t = T::new();
    std::fs::create_dir_all(&t.ctx.root).unwrap();
    std::fs::set_permissions(&t.ctx.root, std::fs::Permissions::from_mode(0o755)).unwrap();
    let held = install::lock(&t.ctx).unwrap();
    assert_eq!(mode(&t.ctx.root), 0o700);
    assert_eq!(install::lock(&t.ctx).unwrap_err(), "Another install is running");
    drop(held);
    // a process another test forks at this moment shares the descriptor until it execs
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while install::lock(&t.ctx).is_err() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(install::lock(&t.ctx).is_ok(), "the lock goes with its holder");
}

#[test]
fn uninstall_stops_terminumd_and_keeps_state_and_logs() {
    let t = T::new();
    t.install("a").unwrap();
    std::fs::write(t.ctx.root.join("state/workspaces.json"), "{}").unwrap();
    std::fs::write(t.ctx.root.join("settings.json"), "{}").unwrap();
    install::uninstall(&t.ctx).unwrap();
    assert!(launch::health(&t.ctx).is_none());
    assert!(!t.ctx.root.join("builds").exists() && !t.ctx.root.join("bin").exists() && !t.ctx.root.join("daemon").exists());
    for kept in ["state/workspaces.json", "settings.json", "logs/terminumd.log"] {
        assert!(t.ctx.root.join(kept).exists(), "{kept}");
    }
    assert!(said(&t.ctx).last().unwrap().contains("tmux sessions untouched"));
}

#[test]
fn restart_in_direct_mode_starts_terminumd_again() {
    let t = T::new();
    t.install("a").unwrap();
    let pid = launch::health(&t.ctx).unwrap()["pid"].clone();
    install::restart(&t.ctx).unwrap();
    assert_ne!(launch::health(&t.ctx).unwrap()["pid"], pid);
    assert!(said(&t.ctx).last().unwrap().starts_with("Restarted terminumd: build a, port 48080"));
}

#[test]
fn a_restart_of_an_install_from_before_daemon_puts_it_there() {
    let t = T::new();
    t.install("a").unwrap();
    std::fs::remove_dir_all(t.ctx.root.join("daemon")).unwrap();
    install::restart(&t.ctx).unwrap();
    assert!(t.daemon_is("a"));
    assert_eq!(t.running().as_deref(), Some("a"));
}

// The result line and the PATH tip: colors only on a terminal that takes them, and no tip
// for a user who has done what it says.

fn tty(home: &Home, env: &[(&str, &str)]) -> Ctx {
    let mut all = vec![("TERM", "xterm-256color")];
    all.extend_from_slice(env);
    let mut ctx = home.ctx(&all);
    ctx.tty = true;
    ctx
}

#[test]
fn colors_only_where_they_work() {
    let home = Home::new();
    assert!(tty(&home, &[]).color());
    assert!(!tty(&home, &[("NO_COLOR", "1")]).color());
    assert!(!tty(&home, &[("TERM", "dumb")]).color());
    assert!(!home.ctx(&[("TERM", "xterm-256color")]).color(), "not a terminal");
    let ctx = tty(&home, &[]);
    ctx.done("Upgraded a → b");
    assert_eq!(said(&ctx), ["\x1b[32m✓\x1b[0m Upgraded a → b"]);
    let plain = tty(&home, &[("NO_COLOR", "1")]);
    plain.done("Upgraded a → b");
    assert_eq!(said(&plain), ["Upgraded a → b"]);
}

#[test]
fn the_tip_and_when_it_is_not_needed() {
    let home = Home::new();
    let tip = |env: &[(&str, &str)]| {
        let ctx = tty(&home, env);
        install::path_tip(&ctx);
        said(&ctx)
    };
    let shown = tip(&[]);
    assert_eq!(shown[0], "");
    assert!(shown[1].starts_with("\x1b[38;5;69mTip: add "), "{shown:?}");
    let on_path = format!("/usr/bin:{}", home.path(".terminum/bin").display());
    assert!(tip(&[("PATH", &on_path)]).is_empty(), "on PATH");
    std::fs::write(home.path(".zshrc"), "export PATH=\"$HOME/.terminum/bin:$PATH\"\n").unwrap();
    assert!(tip(&[]).is_empty(), "added to .zshrc, this shell has not read it yet");
    std::fs::write(home.path(".zshrc"), "  # export PATH=\"$HOME/.terminum/bin:$PATH\"\n").unwrap();
    assert_eq!(tip(&[]).len(), 2, "a comment does not count");
    std::fs::remove_file(home.path(".zshrc")).unwrap();
    std::fs::create_dir_all(home.path(".config/fish")).unwrap();
    std::fs::write(home.path(".config/fish/config.fish"), "fish_add_path ~/.terminum/bin\n").unwrap();
    assert!(tip(&[]).is_empty(), "fish");
    std::fs::remove_file(home.path(".config/fish/config.fish")).unwrap();
    std::fs::create_dir_all(home.path("zdot")).unwrap();
    std::fs::write(home.path("zdot/.zshrc"), "path+=(~/.terminum/bin)\n").unwrap();
    let zdot = home.path("zdot").display().to_string();
    assert!(tip(&[("ZDOTDIR", &zdot)]).is_empty(), "ZDOTDIR");
}

#[test]
fn the_package_of_a_binary_run_by_path_or_by_link() {
    let home = Home::new();
    let pkg = package(&home, "a", None, false);
    let exe = pkg.join("agents/macos-aarch64/terminumd");
    assert_eq!(crate::cli::ctx::package_of(&exe.display().to_string()), Some(pkg.clone()));
    let link = home.path("bin/terminum");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&exe, &link).unwrap();
    assert_eq!(crate::cli::ctx::package_of(&link.display().to_string()), Some(pkg), "links are followed");
}

#[test]
fn a_link_in_a_package_is_refused() {
    let t = T::new();
    let pkg = package(&t.home, "b", None, false);
    std::os::unix::fs::symlink("/etc/hosts", pkg.join("README.md")).unwrap();
    let msg = install::install(&t.ctx, &pkg).unwrap_err();
    assert!(msg.contains("README.md: not a plain file or folder"), "{msg}");
    assert_eq!(t.link("current"), None);
    assert!(!t.ctx.root.join("builds/b").exists());
}

#[test]
fn an_interrupted_copy_is_pruned_by_the_next_install() {
    let t = T::new();
    t.install("a").unwrap();
    std::fs::create_dir_all(t.ctx.root.join("builds/.x.tmp")).unwrap();
    t.install("b").unwrap();
    assert!(!t.ctx.root.join("builds/.x.tmp").exists());
}

#[test]
fn a_rollback_to_a_build_that_does_not_come_up_switches_back() {
    let t = T::new();
    install::install(&t.ctx, &package(&t.home, "a", None, false)).unwrap();
    t.install("b").unwrap();
    // builds/a breaks after it was installed
    let a = std::fs::read_link(t.ctx.root.join("builds/a/terminumd")).unwrap();
    write_exec(&t.ctx.root.join("builds/a").join(a), "#!/bin/sh\nif [ \"$1\" = --version ]; then echo a; exit 0; fi\nexit 1\n");
    let msg = install::rollback(&t.ctx).unwrap_err();
    assert!(msg.starts_with("a did not come up:") && msg.contains("b runs again"), "{msg}");
    assert_eq!((t.link("current"), t.link("previous"), t.running()), (Some("b".into()), Some("a".into()), Some("b".into())));
}

#[test]
fn a_failed_upgrade_from_the_old_name_brings_the_old_agent_back() {
    let home = Home::new();
    let lc = home.path("lc");
    std::fs::create_dir_all(&lc).unwrap();
    let tbin = home.path("tbin");
    write_exec(&tbin.join("tmux"), "#!/bin/sh\n");
    let mut ctx = home.ctx(&[("TM_NO_OPEN", "1"), ("PATH", &format!("{}:/usr/bin:/bin", tbin.display()))]);
    ctx.launchctl = stub_launchctl(&lc);
    ctx.wait = Duration::from_millis(300); // the stub starts nothing: the new build never answers
    // the install of the old name: builds/a/terminusd, current -> a, its agent's plist
    let a = ctx.lib().join("a");
    std::fs::create_dir_all(&a).unwrap();
    write_exec(&a.join("terminusd"), "#!/bin/sh\n");
    std::os::unix::fs::symlink("a", ctx.lib().join("current")).unwrap();
    std::fs::create_dir_all(&ctx.agents).unwrap();
    let legacy = ctx.agents.join("io.terminum.terminusd.plist");
    std::fs::write(&legacy, "<plist/>").unwrap();
    let msg = install::install(&ctx, &package(&home, "b", None, false)).unwrap_err();
    assert!(msg.contains("a runs again as before"), "{msg}");
    assert_eq!(install::target(&ctx, "current").as_deref(), Some("a"));
    assert!(legacy.exists() && !ctx.plist().exists(), "the old agent is loaded again, the new one gone");
    assert_eq!(verbs(&lc).last().map(String::as_str), Some("bootstrap"));
}
