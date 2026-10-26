// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The agents link on the files helper's ssh (remote contract "Transport"): with a host's
//! `agents` on, that ssh also carries `-R <host's ~/.terminum/run>/m-<macid>-<rand8>.sock:<the
//! Mac's link socket>` and the helper gets `--link <that path> --mac <macid>`. The helper says
//! `terminumd-agent link: up` or `link: refused` before its ready line; a refusal is retried
//! with -R after 10 min while files keep working.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::app::Shared;

/// A refused forward is tried again after this long.
pub const RETRY: Duration = Duration::from_secs(600);
/// sshd's remote socket path limit kept with a margin (contract: ≤100 bytes).
const REMOTE_MAX: usize = 100;
/// The Mac's socket path limit (sun_path without its NUL).
const LOCAL_MAX: usize = 103;

/// Whether `host` has coding agents' progress enabled (`"agents": "enabled"` in its record).
pub fn wanted(app: &Shared, host: &str) -> bool {
    app.looks.settings.read().hosts.get(host).is_some_and(|r| r["agents"].as_str() == Some("enabled"))
}

/// Applies a host's changed `agents` setting (the CLI and the host menu call it after saving):
/// the helper's ssh restarts with or without the link, or ends when files are off too.
// its callers (`terminum hosts agents`, the host menu) come with the next round
#[allow(dead_code)]
pub fn changed(app: &Shared, host: &str) {
    super::tunnel::stop(app, host);
    super::tunnel::reconcile(app, host);
}

/// This Mac's id on hosts: a random 12-hex id kept in `<app>/mac-id`, made once.
pub fn mac_id(dir: &Path) -> String {
    let f = dir.join("mac-id");
    if let Ok(s) = std::fs::read_to_string(&f) {
        let s = s.trim();
        if !s.is_empty() && s.len() <= 16 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return s.to_string();
        }
    }
    let id = crate::auth::crypto::random_hex::<6>();
    let _ = std::fs::write(&f, &id);
    id
}

/// The remote end of the forward: `<home>/.terminum/run/m-<macid>-<rand8>.sock`, or why not.
pub fn remote_path(home: &str, mac: &str, rand8: &str) -> Result<String, String> {
    if !home.starts_with('/') || home.contains(['\n', '\0', ':']) {
        return Err(format!("the host's home {home:?} is not a usable absolute path"));
    }
    let p = format!("{}/.terminum/run/m-{mac}-{rand8}.sock", home.trim_end_matches('/'));
    if p.len() > REMOTE_MAX {
        return Err(format!("the host's link path {p} is longer than {REMOTE_MAX} bytes"));
    }
    Ok(p)
}

/// The Mac's link socket path, checked: absolute (a bare name is read as host:port) and short.
pub fn local_ok(local: &Path) -> Result<(), String> {
    let s = local.to_string_lossy();
    if !local.is_absolute() || s.contains(':') {
        return Err(format!("{s} is not an absolute path without ':'"));
    }
    if s.len() > LOCAL_MAX {
        return Err(format!("{s} is longer than {LOCAL_MAX} bytes"));
    }
    Ok(())
}

/// The ssh options agents add. With agents off the helper's ssh stays exactly as it was
/// (`ExitOnForwardFailure=yes`, the user's ControlPath): only this case changes them.
/// ControlPath=none: a mux master's -R would outlive us under ControlPersist. Forward
/// failure does not end ssh (a user's own RemoteForward may fail): the helper tells.
pub fn ssh_args(link: Option<(&str, &Path)>) -> Vec<String> {
    let Some((remote, local)) = link else { return vec!["-o".into(), "ExitOnForwardFailure=yes".into()] };
    vec!["-o".into(), "ControlPath=none".into(), "-o".into(), "ExitOnForwardFailure=no".into(), "-R".into(), format!("{remote}:{}", local.display())]
}

/// The helper's flags for the link.
pub fn helper_args(remote: &str, mac: &str) -> String {
    format!(" --link {remote} --mac {mac}")
}

/// What the helper said about the link.
#[derive(Debug, PartialEq, Eq)]
pub enum Said {
    Up,
    Refused,
}

pub fn said(line: &str) -> Option<Said> {
    match line.trim() {
        "terminumd-agent link: up" => Some(Said::Up),
        "terminumd-agent link: refused" => Some(Said::Refused),
        _ => None,
    }
}

/// The agents state the host listing shows when its sshd refused the forward.
pub fn refused_why(host: &str) -> String {
    format!("forwarding refused by {host}'s sshd (AllowTcpForwarding/AllowStreamLocalForwarding/DisableForwarding, or Dropbear)")
}

/// Whether to ask for -R now: never refused, or refused at least RETRY ago.
pub fn try_forward(refused_at: Option<Instant>, now: Instant) -> bool {
    refused_at.is_none_or(|t| now.duration_since(t) >= RETRY)
}

/// The probe's command: a marker line, then `uname -sm` and $HOME, so rc noise before it
/// cannot be read as the answer.
pub fn probe_command(marker: &str) -> String {
    format!("echo {marker}; uname -sm; printf '%s\\n' \"$HOME\"")
}

/// (system, machine, home) from the lines after the marker.
pub fn parse_probe(out: &str, marker: &str) -> Option<(String, String, String)> {
    let mut lines = out.lines().skip_while(|l| l.trim() != marker).skip(1);
    let mut w = lines.next()?.split_whitespace();
    let (s, m) = (w.next()?.to_string(), w.next()?.to_string());
    let home = lines.next()?.trim_end_matches('\r').to_string();
    Some((s, m, home))
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;
