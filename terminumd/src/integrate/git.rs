// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! git as the integration queue runs it: never asking (no terminal prompt, ssh in batch mode with a connect timeout
//! unless the user set their own ssh command), never signing what it makes, so one hung command never holds the
//! engine's one loop for every project (`terminum-integration.md` §9).

use std::path::{Path, PathBuf};
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

/// How many files `git status --porcelain` lists: changed, staged or untracked, ignored ones not; whatever the
/// user's config hides (untracked files, submodules) counted too.
pub fn uncommitted(dir: &Path) -> Result<usize, String> {
    Ok(run(dir, &["status", "--porcelain", "--untracked-files=normal", "--ignore-submodules=none"])?.lines().count())
}

/// The files `head` changed since it left `base`, as git lists them.
pub fn changed(repo: &Path, base: &str, head: &str) -> Result<Vec<String>, String> {
    // quoted as git quotes by default, whatever the user set: no control or invisible character reaches a prompt
    Ok(run(repo, &["-c", "core.quotePath=true", "diff", "--name-only", &format!("{base}...{head}")])?.lines().map(str::to_string).collect())
}

/// A commit as people read it: its first nine characters.
pub fn short(sha: &str) -> &str {
    &sha[..9.min(sha.len())]
}

pub(crate) fn is_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A Supervisor observation reads only local Git objects, including in a partial clone.
pub(crate) fn local_output(dir: &Path, args: &[&str]) -> Result<String, String> {
    let o = git(dir).args(["-c", "core.fsmonitor=false"]).env("GIT_NO_LAZY_FETCH", "1").env("GIT_NO_REPLACE_OBJECTS", "1")
        .args(args).output().map_err(|e| format!("git: {e}"))?;
    if o.status.success() {
        String::from_utf8(o.stdout).map_err(|_| "Git returned a path that is not valid UTF-8.".into())
    } else {
        Err(said(&o))
    }
}

pub(crate) struct Checkout {
    pub root: PathBuf,
    pub common: String,
    pub head: String,
}

pub(crate) fn checkout(dir: &Path) -> Result<Checkout, String> {
    let out = local_output(dir, &["rev-parse", "--path-format=absolute", "--show-toplevel", "--git-common-dir", "HEAD"])?;
    let mut lines = out.lines();
    let (Some(root), Some(common), Some(head), None) = (lines.next(), lines.next(), lines.next(), lines.next()) else {
        return Err("Git did not return a checkout and its HEAD.".into());
    };
    if !is_commit(head) { return Err("Git did not return a full HEAD commit.".into()); }
    Ok(Checkout { root: root.into(), common: common.into(), head: head.into() })
}
