// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Shared Codex app-server detection (AC-207, AC-208) and the commands to start a Codex
//! in its own process or disable and stop the shared server.

use super::{argv, parent_id, program_at, ps};

/// A config override keeps Codex in its own process (AS-128); this one also keeps it inline.
pub(crate) const OWN_OPTION: &str = "-c tui.alternate_screen=never";
pub(crate) const NO_DAEMON: &str = "--no-daemon";
pub(crate) const NO_ATTACH: &str = "codex features disable daemon_auto_start";
pub(crate) const STOP_SERVER: &str = "codex app-server daemon stop";

/// The ancestors looked at: the agent's program may run a hook through a shell.
const UP: usize = 3;

/// Whether `words` (a command line) are a Codex app-server listening where other Codex
/// processes attach: `--listen unix://…` or `ws://…` (a stdio one is its client's alone).
fn is_shared(words: &[impl AsRef<str>]) -> bool {
    if program_at(words, "codex").is_none() || !words.iter().any(|word| word.as_ref() == "app-server") {
        return false;
    }
    let listens = |url: &str| url.starts_with("unix://") || url.starts_with("ws://");
    words.iter().map(AsRef::as_ref).enumerate().any(|(i, word)| match word.strip_prefix("--listen") {
        Some("") => words.get(i + 1).is_some_and(|url| listens(url.as_ref())),
        Some(rest) => rest.strip_prefix('=').is_some_and(listens),
        None => false,
    })
}

/// Whether a shared app-server started this process: one of its nearest ancestors' exact words
/// (the kernel's, no `ps`: a hook pays microseconds); a Codex of its own (the agent) ends the look.
pub fn under_shared() -> bool {
    let mut pid = std::os::unix::process::parent_id();
    for _ in 0..UP {
        if let Some(words) = argv(pid) && program_at(&words, "codex").is_some() {
            return is_shared(&words);
        }
        match parent_id(pid) {
            Some(p) if p > 1 => pid = p,
            _ => break,
        }
    }
    false
}

/// The process id of a shared app-server of this user that runs, if one does (`ps`, once).
pub fn running() -> Option<u32> {
    // SAFETY: getuid has no preconditions
    let uid = unsafe { libc::getuid() }.to_string();
    let out = ps(&["-ww", "-U", &uid, "-o", "pid=,args="])?;
    out.lines().find_map(|row| {
        let mut words = row.split_whitespace();
        let pid = words.next()?.parse().ok()?;
        is_shared(&words.collect::<Vec<_>>()).then_some(pid)
    })
}

/// Whether a Codex started with `words` attaches to a shared server when one runs: no `-c`, `--config` or
/// `--no-daemon` (each keeps it in its own process, AS-128), and its TUI (not `codex exec`).
pub fn may_attach(words: &[String]) -> bool {
    let Some(at) = program_at(words, "codex") else { return false };
    let rest = &words[at + 1..];
    let own = rest.iter().any(|w| w.starts_with("-c") || w == "--config" || w.starts_with("--config=") || w == NO_DAEMON);
    !own && !matches!(crate::agentopts::codex_subcommand(rest), Some("exec" | "e"))
}

#[cfg(test)]
mod codexdaemon_tests {
    use super::*;

    fn w(s: &str) -> Vec<&str> {
        s.split(' ').collect()
    }

    #[test]
    fn ac207_a_listening_app_server_is_shared() {
        for line in ["/Users/alex/.codex/packages/app-server-daemon/releases/0.171.0-aarch64-apple-darwin/bin/codex app-server --listen unix:// --analytics-default-enabled",
                     "codex app-server --listen unix:///Users/alex/.codex/app.sock", "codex app-server --listen=ws://127.0.0.1:4500",
                     "node /opt/homebrew/bin/codex app-server --listen unix://"] {
            assert!(is_shared(&w(line)), "{line}");
        }
    }

    #[test]
    fn ac207_a_stdio_app_server_or_another_codex_is_not_shared() {
        for line in ["codex app-server", "codex app-server --listen stdio://", "codex app-server --stdio", "codex app-server --listen off",
                     "codex app-server --listen", "codex --no-alt-screen", "codex app-server daemon pid-update-loop",
                     "codex exec --listen unix:// fix", "claude app-server --listen unix://", "codex-helper app-server --listen unix://", ""] {
            assert!(!is_shared(&w(line)), "{line}");
        }
    }
}
