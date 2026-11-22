// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-227: the owner chooses the executable. A notice can reach only its standard input.

use std::{io, os::unix::process::ExitStatusExt, process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::{Child, Command}, time::{sleep, timeout}};

pub(super) const DEADLINE: Duration = Duration::from_secs(5);

pub(super) async fn send(path: &str, check: bool, text: &str, limit: Duration) -> Result<(), String> {
    let mut cmd = Command::new(path);
    if check {
        cmd.arg("--check");
    }
    let child = cmd.env_clear().env("PATH", "/usr/bin:/bin").env("LANG", "C").current_dir("/")
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
        .process_group(0).kill_on_drop(true).spawn()
        .map_err(|_| "Could not start the command.".to_string())?;
    let mut group = Group { pid: child.id().map(|pid| pid as libc::pid_t), child };
    let mut input = group.child.stdin.take().expect("piped stdin");
    let result = timeout(limit, async {
        let wrote = input.write_all(text.as_bytes()).await;
        drop(input);
        group.exited().await?;
        Ok::<_, io::Error>(wrote)
    }).await;
    group.kill();
    let status = group.child.wait().await.map_err(|_| "Could not read the exit status.")?;
    let wrote = result.map_err(|_| "Command timed out.")?.map_err(|_| "Could not observe the command.")?;
    if !status.success() {
        return Err(match (status.code(), status.signal()) {
            (Some(code), _) => format!("Exit status {code}."),
            (_, Some(signal)) => format!("Ended by signal {signal}."),
            _ => "Command failed.".into(),
        });
    }
    wrote.map_err(|_| "Command did not accept the notice.".into())
}

struct Group {
    pid: Option<libc::pid_t>,
    child: Child,
}

impl Group {
    async fn exited(&self) -> io::Result<()> {
        let pid = self.pid.expect("new child has a process ID");
        while !ready_to_reap(pid)? {
            sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }

    fn kill(&mut self) {
        if let Some(pid) = self.pid.take() {
            // SAFETY: process_group(0) created this group for the unreaped child.
            unsafe { libc::killpg(pid, libc::SIGKILL) };
        }
    }
}

impl Drop for Group {
    fn drop(&mut self) { self.kill(); }
}

fn ready_to_reap(pid: libc::pid_t) -> io::Result<bool> {
    // Keep the leader unreaped until we signal its group. This also
    // ends a helper that the command left running on successful exit.
    // SAFETY: waitid writes to the supplied siginfo for our own child.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT)
    };
    if result == 0 {
        // SAFETY: a zeroed record or one filled by waitid has a valid pid field.
        return Ok(unsafe { info.si_pid() } == pid);
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::Interrupted { Ok(false) } else { Err(error) }
}
