// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! `terminum statusline [command…]` (`terminum-dispatcher.md` AC-167): Claude Code's status line, the one place it
//! says its subscription's windows. The user puts it in Claude Code's settings themselves; terminum never edits them.
//! It sends terminumd the windows' numbers alone, as a `Limits` hook event of its pane, and runs the user's own status
//! line command with the same input: one word through `sh -c`, as Claude Code runs it, several as given. Its output and
//! exit code are the user's; terminumd down or slow costs it the hook's budget at most. With no command, one short line.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

use crate::dispatcher::subscriptions::{self, Window};

/// The most of its input kept: Claude Code's is a few KB.
const KEEP: u64 = 1 << 20;

/// The user's command, started with its input on the way, its output to `out`; None with none given.
fn start(args: &[String], input: &[u8], out: Stdio) -> Option<std::io::Result<std::process::Child>> {
    let (prog, rest) = args.split_first()?;
    let mut cmd = if rest.is_empty() { let mut c = Command::new("/bin/sh"); c.arg("-c").arg(prog); c } else { let mut c = Command::new(prog); c.args(rest); c };
    Some(cmd.stdin(Stdio::piped()).stdout(out).spawn().map(|mut child| {
        // a command that does not read it is its own business: a closed pipe is no error here
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input);
        }
        child
    }))
}

/// The windows told to terminumd, from the pane the status line runs in; nothing outside tmux or a pane.
fn tell(input: &[u8]) {
    let (Ok(v), Ok(tmux), Ok(pane)) = (serde_json::from_slice::<Value>(input), std::env::var("TMUX"), std::env::var("TMUX_PANE")) else { return };
    let (Some(windows), Some(ev)) = (subscriptions::of_claude(&v), super::hook::event("claude", &json!({"hook_event_name": "Limits"}), &tmux, &pane)) else { return };
    let _ = super::sinks::post_all(&super::sinks::all(), "/hook", &super::hook::limits_event(&ev, &windows), super::hook::BUDGET);
}

/// The line it prints when it runs alone: each window's percent.
pub fn own_line(windows: &[Window]) -> String {
    windows.iter().map(|w| format!("{} {}%", if w.minutes == 300 { "5h".to_string() } else if w.minutes == 10080 { "week".to_string() } else { format!("{}h", w.minutes / 60) }, w.used.round()))
        .collect::<Vec<_>>().join(" · ")
}

pub fn main(args: &[String]) -> i32 {
    let mut input = Vec::new();
    let _ = std::io::stdin().lock().take(KEEP).read_to_end(&mut input);
    let child = start(args, &input, Stdio::inherit());
    tell(&input);
    match child {
        None => {
            let windows = serde_json::from_slice::<Value>(&input).ok().and_then(|v| subscriptions::of_claude(&v)).unwrap_or_default();
            let _ = writeln!(std::io::stdout().lock(), "{}", own_line(&windows));
            0
        }
        Some(Err(e)) => {
            eprintln!("terminum statusline: {}: {e}", args[0]);
            127
        }
        Some(Ok(mut child)) => child.wait().ok().and_then(|s| s.code()).unwrap_or(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `args` run on `input`: the exit code and what it printed.
    fn run(args: &[&str], input: &str) -> std::io::Result<(Option<i32>, String)> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let out = start(&args, input.as_bytes(), Stdio::piped()).unwrap()?.wait_with_output()?;
        Ok((out.status.code(), String::from_utf8_lossy(&out.stdout).trim().to_string()))
    }

    #[test]
    fn ac167_the_users_command_gets_the_same_input_and_says_what_it_says() {
        assert_eq!(run(&["read l; echo \"mine: $l\"; exit 3"], "{\"model\":1}\n").unwrap(), (Some(3), "mine: {\"model\":1}".into()), "its output and exit code");
        assert_eq!(run(&["echo ignores it"], &"x".repeat(200_000)).unwrap(), (Some(0), "ignores it".into()), "one that reads nothing is no error");
        assert!(start(&[], b"", Stdio::null()).is_none(), "no command: its own line");
    }

    #[test]
    fn ac167_one_word_goes_through_sh_several_as_given() {
        assert_eq!(run(&["echo a b | wc -w"], "").unwrap().1, "2", "one word: a shell's line, as Claude Code runs it");
        assert_eq!(run(&["/bin/echo", "a b", "c"], "").unwrap().1, "a b c", "several: the program and its arguments");
        assert!(run(&["/no/such/program", "a"], "").is_err(), "said, not hidden");
    }

    #[test]
    fn ac167_its_own_line() {
        let w = |minutes, used| Window { minutes, used, resets_at: None };
        assert_eq!(own_line(&[w(300, 34.4), w(10080, 21.0)]), "5h 34% · week 21%");
        assert_eq!(own_line(&[w(1440, 2.0)]), "24h 2%");
        assert_eq!(own_line(&[]), "", "an API key: an empty line");
    }
}
