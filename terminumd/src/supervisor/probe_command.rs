// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-260: cap stdout while reading and reap the entire command group.

use std::{path::Path, process::Stdio, time::Duration};

use tokio::{io::AsyncReadExt, process::Command, time::timeout};

use super::line_command::Group;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure { Unavailable, Start, Exit, Timeout, Output, Invalid }

impl Failure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "source unavailable",
            Self::Start => "could not start",
            Self::Exit => "unsuccessful exit",
            Self::Timeout => "timed out",
            Self::Output => "output limit exceeded",
            Self::Invalid => "invalid data",
        }
    }
}

pub(super) fn command(path: &str, dir: &Path) -> Command {
    let mut cmd = Command::new(path);
    cmd.env_clear().envs([("PATH", "/usr/bin:/bin"), ("LANG", "C"), ("LC_ALL", "C"), ("TZ", "UTC")])
        .current_dir(dir).stdin(Stdio::null()).stderr(Stdio::null()).stdout(Stdio::piped());
    cmd
}

pub(super) async fn output(mut cmd: Command, limit: Duration, cap: usize) -> Result<Vec<u8>, Failure> {
    let mut child = Group::spawn(&mut cmd).map_err(|_| Failure::Start)?;
    let mut stdout = child.child.stdout.take().expect("piped probe stdout").take(cap as u64 + 1);
    let result = timeout(limit, async {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.map_err(|_| Failure::Output)?;
        if bytes.len() > cap { return Err(Failure::Output); }
        child.exited().await.map_err(|_| Failure::Exit)?;
        Ok(bytes)
    }).await;
    child.kill();
    let status = child.child.wait().await.map_err(|_| Failure::Exit)?;
    let bytes = result.map_err(|_| Failure::Timeout)??;
    if !status.success() { return Err(Failure::Exit); }
    Ok(bytes)
}

pub(super) async fn shell(dir: &Path, script: &str, limit: Duration, cap: usize) -> Result<Vec<u8>, Failure> {
    let mut cmd = command("/bin/sh", dir);
    cmd.args(["-c", script]);
    output(cmd, limit, cap).await
}
