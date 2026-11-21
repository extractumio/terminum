// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Local process readers for hooks, pane notices and peer ancestry (AC-153, AC-208, AC-178).
//! Async callers run them off their workers. The dispatcher reads resource figures with
//! its own local or remote runner and asks for no arguments.

use std::collections::HashMap;
use std::process::Command;

use crate::termtext::shell_name;

pub(crate) mod codex;

fn ps(args: &[&str]) -> Option<String> {
    let out = Command::new("ps").args(args).output().ok()?;
    Some(String::from_utf8(out.stdout).unwrap_or_else(|err| String::from_utf8_lossy(err.as_bytes()).into_owned()))
}

/// Parent and approximate arguments from ps, which puts spaces between the words.
pub(crate) fn parent_and_words(pid: u32) -> Option<(u32, Vec<String>)> {
    let row = ps(&["-ww", "-o", "ppid=,args=", "-p", &pid.to_string()])?;
    let (ppid, args) = row.trim().split_once(char::is_whitespace)?;
    Some((ppid.parse().ok()?, args.split_whitespace().map(str::to_string).collect()))
}

/// The local process table's parent ids, with no arguments.
pub(crate) fn parents() -> Option<HashMap<u32, u32>> {
    let out = ps(&["-A", "-o", "pid=,ppid="])?;
    Some(out.lines().filter_map(|line| {
        let mut fields = line.split_whitespace();
        Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
    }).collect())
}

/// The foreground group's leader and whether it has children on the same tty.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Foreground {
    pub pid: u32,
    pub words: Vec<String>,
    /// seconds it has run
    pub age: u64,
    /// whether the leader has a direct child on the tty
    pub children: bool,
}

/// The foreground run in `ps -o pid=,ppid=,tpgid=,etime=` rows of one tty.
fn foreground_of(rows: &str) -> Option<(u32, u64, bool)> {
    let rows: Vec<[&str; 4]> = rows.lines().filter_map(|l| <[&str; 4]>::try_from(l.split_whitespace().collect::<Vec<_>>()).ok()).collect();
    let lead = rows.iter().find(|r| r[0] == r[2])?;
    Some((lead[0].parse().ok()?, seconds(lead[3]) as u64, rows.iter().any(|r| r[1] == lead[0])))
}

/// The foreground run of terminal `tty` (`/dev/ttys004`), by `ps` and the kernel's words.
pub(crate) fn foreground(tty: &str) -> Option<Foreground> {
    let name = tty.strip_prefix("/dev/").filter(|n| !n.is_empty() && !n.contains(['/', ' ']))?;
    let rows = ps(&["-t", name, "-o", "pid=,ppid=,tpgid=,etime="])?;
    let (pid, age, children) = foreground_of(&rows)?;
    Some(Foreground { pid, words: argv(pid)?, age, children })
}

/// `[[dd-]hh:]mm:ss`, as `ps -o etime` and `-o time` say it, in seconds.
pub(crate) fn seconds(t: &str) -> f64 {
    let (days, rest) = t.split_once('-').map_or((0.0, t), |(d, r)| (d.parse().unwrap_or(0.0), r));
    rest.split(':').fold(0.0, |sum, part| sum * 60.0 + part.parse::<f64>().unwrap_or(0.0)) + days * 86400.0
}

/// Programs whose script is the program (`node …/codex`).
const INTERPRETERS: [&str; 3] = ["node", "bun", "deno"];

/// Where in `words` the agent's program is: first, or an interpreter's script (npm's Codex: `node …/bin/codex.js`,
/// where an older Node does not replace itself with the native program).
pub(crate) fn program_at(words: &[impl AsRef<str>], agent: &str) -> Option<usize> {
    let at = usize::from(words.len() > 1 && INTERPRETERS.contains(&shell_name(words[0].as_ref())));
    let program = shell_name(words.get(at)?.as_ref());
    (program == agent || (at == 1 && program.strip_suffix(".js") == Some(agent))).then_some(at)
}

/// Process `pid`'s argument vector as the kernel keeps it.
#[cfg(target_os = "linux")]
pub(crate) fn argv(pid: u32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(raw.split(|b| *b == 0).filter(|w| !w.is_empty()).map(|w| String::from_utf8_lossy(w).into_owned()).collect())
}

/// Process `pid`'s argument vector as the kernel keeps it (`KERN_PROCARGS2`: argc, the program's
/// path, padding, then argc words, each ending in NUL).
#[cfg(target_os = "macos")]
pub(crate) fn argv(pid: u32) -> Option<Vec<String>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    let mut most: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: mib names an int, `most` is one, `size` its size
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 2, (&raw mut most).cast(), &mut size, std::ptr::null_mut(), 0) } != 0 || most <= 0 {
        return None;
    }
    let mut buf = vec![0u8; most as usize];
    let mut size = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, libc::c_int::try_from(pid).ok()?];
    // SAFETY: buf holds `size` bytes, which the kernel fills and sets `size` to
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) } != 0 || size < 4 {
        return None;
    }
    let argc = usize::try_from(i32::from_ne_bytes(buf[..4].try_into().ok()?)).ok()?;
    let mut parts = buf[4..size].split(|b| *b == 0).skip(1).skip_while(|w| w.is_empty());
    let words: Vec<String> = parts.by_ref().take(argc).map(|w| String::from_utf8_lossy(w).into_owned()).collect();
    (words.len() == argc).then_some(words)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn argv(_pid: u32) -> Option<Vec<String>> {
    None
}

/// Process `pid`'s parent.
#[cfg(target_os = "macos")]
fn parent_id(pid: u32) -> Option<u32> {
    // SAFETY: proc_bsdinfo is plain data; proc_pidinfo fills at most `size` bytes of it
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let got = unsafe { libc::proc_pidinfo(libc::c_int::try_from(pid).ok()?, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (got == size).then_some(info.pbi_ppid)
}

/// Process `pid`'s parent: the field after the state, past the command's closing parenthesis.
#[cfg(target_os = "linux")]
fn parent_id(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn parent_id(_pid: u32) -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac207_the_kernel_reports_the_parent_pid() {
        assert_eq!(parent_id(std::process::id()), Some(std::os::unix::process::parent_id()));
        assert_eq!(parent_id(u32::MAX), None, "no such process");
    }

    #[test]
    fn ac208_the_foreground_run_is_the_ttys_foreground_groups_leader() {
        // pid ppid tpgid etime: the shell, Codex in front, its MCP server under it
        let rows = "  100     1   200    01:02:03\n  200   100   200       00:07\n  201   200   200       00:06\n";
        assert_eq!(foreground_of(rows), Some((200, 7, true)));
        // an attached Codex has nothing of its own under it; days count
        assert_eq!(foreground_of("100 1 300 1-00:00:01\n300 100 300 2-01:00:00\n"), Some((300, 2 * 86_400 + 3600, false)));
        assert_eq!(foreground_of("100 1 0 00:01\n"), None, "no foreground group");
        assert_eq!(foreground_of(""), None);
    }
}
