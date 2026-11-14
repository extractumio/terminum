// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! git as the integration queue runs it: never asking (no terminal prompt, ssh in batch mode with a connect timeout
//! unless the user set their own ssh command), never signing what it makes, so one hung command never holds the
//! engine's one loop for every project (`terminum-integration.md` §9).

use std::path::Path;
use std::process::{Command, Output};

fn git(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(dir).args(["-c", "commit.gpgSign=false"]).env("GIT_TERMINAL_PROMPT", "0");
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        c.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o ConnectTimeout=20");
    }
    c
}

/// What git printed and how it ended.
pub fn output(dir: &Path, args: &[&str]) -> Result<Output, String> {
    git(dir).args(args).output().map_err(|e| format!("git: {e}"))
}

/// Its words: what it says on stderr, else on stdout.
pub fn said(o: &Output) -> String {
    let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
    if e.is_empty() { String::from_utf8_lossy(&o.stdout).trim().to_string() } else { e }
}

/// Its trimmed stdout, or its words as the error.
pub fn run(dir: &Path, args: &[&str]) -> Result<String, String> {
    let o = output(dir, args)?;
    if o.status.success() { Ok(String::from_utf8_lossy(&o.stdout).trim().to_string()) } else { Err(said(&o)) }
}

/// Whether it ended well (`merge-base --is-ancestor`, `rev-parse --verify`).
pub fn ok(dir: &Path, args: &[&str]) -> Result<bool, String> {
    Ok(output(dir, args)?.status.success())
}

/// A commit as people read it: its first nine characters.
pub fn short(sha: &str) -> &str {
    &sha[..9.min(sha.len())]
}
