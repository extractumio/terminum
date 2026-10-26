// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! systemd's user service on Linux (AC-08, AC-14, AC-16) against a stub systemctl and loginctl:
//! the unit (terminumd alone stopped, never its tmux; started at login; restarted for ever),
//! what install, restart and uninstall ask of systemctl, the live label refused outside the
//! real home, a missing user manager said in a sentence, and the linger warning.

use std::path::{Path, PathBuf};

use super::super::systemd;
use super::{said, write_exec, Home};

/// A systemctl and a loginctl that write their arguments to `calls`; `nobus` makes systemctl
/// answer as with no user manager, `linger` holds loginctl's answer.
fn stubs(dir: &Path) -> (PathBuf, PathBuf) {
    let (sc, lc) = (dir.join("systemctl"), dir.join("loginctl"));
    write_exec(&sc, "#!/bin/sh\nd=$(dirname \"$0\")\necho \"systemctl $*\" >> \"$d/calls\"\n\
        [ -f \"$d/nobus\" ] && { echo 'Failed to connect to bus: No medium found' >&2; exit 1; }\nexit 0\n");
    write_exec(&lc, "#!/bin/sh\nd=$(dirname \"$0\")\necho \"loginctl $*\" >> \"$d/calls\"\ncat \"$d/linger\" 2>/dev/null\nexit 0\n");
    (sc, lc)
}

fn calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("calls")).unwrap_or_default().lines().map(str::to_string).collect()
}

fn ctx(home: &Home) -> (super::super::Ctx, PathBuf) {
    let bin = home.path("stub");
    std::fs::create_dir_all(&bin).unwrap();
    let units = home.path("units").display().to_string();
    let mut ctx = home.ctx(&[("TM_SYSTEMD_UNITS", &units), ("USER", "alex")]);
    (ctx.systemctl, ctx.loginctl) = stubs(&bin);
    ctx.systemd = true;
    (ctx, bin)
}

#[test]
fn the_unit_stops_terminumd_alone_starts_at_login_and_restarts_for_ever() {
    let home = Home::new();
    let (ctx, _) = ctx(&home);
    let unit = systemd::unit(&ctx).unwrap();
    for line in ["KillMode=process", "Restart=always", "RestartSec=1", "StartLimitIntervalSec=0", "WantedBy=default.target"] {
        assert!(unit.lines().any(|l| l == line), "{line} in:\n{unit}");
    }
    let program = ctx.program().display().to_string();
    assert!(unit.contains(&format!("ExecStart=\"{program}\" serve")), "{unit}");
    assert!(unit.contains(&format!("StandardOutput=append:{}", ctx.log().display())), "{unit}");
    assert!(unit.lines().any(|l| l.starts_with("Environment=\"PATH=")), "{unit}");
}

#[test]
fn a_home_with_a_percent_or_a_quote_is_escaped_and_one_with_a_space_refused() {
    let home = Home::new();
    let (mut ctx, _) = ctx(&home);
    ctx.root = home.path("a%b\"c");
    let unit = systemd::unit(&ctx).unwrap();
    assert!(unit.contains("a%%b\\\"c/daemon/terminumd"), "{unit}");
    let log = ctx.log().display().to_string().replace('%', "%%");
    assert!(log.contains("a%%b\"c") && unit.contains(&format!("StandardOutput=append:{log}\nStandardError=append:{log}\n")), "the log's path a path, not a specifier:\n{unit}");
    ctx.root = home.path("with space");
    assert!(systemd::unit(&ctx).unwrap_err().contains("systemd cannot append to a path with spaces"));
}

#[test]
fn start_writes_the_unit_reloads_lets_a_failed_one_start_enables_and_restarts() {
    let home = Home::new();
    let (ctx, bin) = ctx(&home);
    systemd::start(&ctx).unwrap();
    assert!(systemd::unit_path(&ctx).is_file());
    let s = |a: &str| format!("systemctl --user {a}");
    let svc = "io.terminum.terminumd.service";
    assert_eq!(calls(&bin), [s("daemon-reload"), s(&format!("reset-failed {svc}")), s(&format!("enable {svc}")), s(&format!("restart {svc}"))]);
    std::fs::remove_file(bin.join("calls")).unwrap();
    systemd::start(&ctx).unwrap();
    assert!(!calls(&bin).contains(&s("daemon-reload")), "an unchanged unit is not read again: {:?}", calls(&bin));
    std::fs::remove_file(bin.join("calls")).unwrap();
    systemd::stop(&ctx).unwrap();
    assert_eq!(calls(&bin), [s(&format!("disable --now {svc}")), s("daemon-reload")]);
    assert!(!systemd::unit_path(&ctx).exists(), "uninstall removes the unit");
}

#[test]
fn no_user_manager_is_said_and_the_live_label_stays_in_the_real_home() {
    let home = Home::new();
    let (mut ctx, bin) = ctx(&home);
    std::fs::write(bin.join("nobus"), "").unwrap();
    let e = systemd::start(&ctx).unwrap_err();
    assert!(e.starts_with("systemd's user manager is not reachable (no session bus): log in as this user"), "{e}");
    ctx.systemctl = PathBuf::from(super::super::ctx::SYSTEM_SYSTEMCTL);
    let e = systemd::start(&ctx).unwrap_err();
    assert!(e.starts_with("systemd runs only the install in "), "a test's home never reaches the live service: {e}");
}

#[test]
fn linger_off_is_said_with_the_command_that_keeps_terminals_after_logout() {
    let home = Home::new();
    let (ctx, bin) = ctx(&home);
    std::fs::write(bin.join("linger"), "no\n").unwrap();
    let hint = systemd::linger_hint(&ctx).unwrap();
    assert!(hint.starts_with("linger is off: terminumd and its terminals end when you log out") && hint.ends_with("sudo loginctl enable-linger alex"), "{hint}");
    std::fs::write(bin.join("linger"), "yes\n").unwrap();
    assert_eq!(systemd::linger_hint(&ctx), None);
    std::fs::write(bin.join("linger"), "").unwrap();
    assert_eq!(systemd::linger_hint(&ctx), None, "loginctl cannot say: nothing guessed");
    assert!(said(&ctx).is_empty());
}
