// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The command line a coding agent started with, as its hook sees it (`terminum-agents.md` AC-153's
//! summary): the nearest of the hook's ancestors whose program is the agent (Claude Code runs a
//! hook through a shell; Codex's `node …/codex` runs its own `codex`), read with `ps` one process at
//! a time (a few ms each; the whole table costs tens), once a session, when terminumd asks. Its program is named by the agent alone
//! (`codex --yolo`, not a vendor binary's path); secrets' values are left out.

use std::process::Command;

/// The ancestors looked at: the hook's shell, the agent, an interpreter.
const UP: usize = 4;

/// The agent's command line, None where no ancestor is it (or `ps` does not answer).
pub fn started_as(agent: &str) -> Option<String> {
    let mut pid = std::os::unix::process::parent_id();
    for _ in 0..UP {
        let out = Command::new("ps").args(["-ww", "-o", "ppid=,args=", "-p", &pid.to_string()]).output().ok()?;
        let row = String::from_utf8_lossy(&out.stdout);
        let (ppid, args) = row.trim().split_once(char::is_whitespace)?;
        if let Some(line) = as_agent(args, agent) {
            return Some(line);
        }
        pid = ppid.trim().parse().ok().filter(|p| *p > 1)?;
    }
    None
}

/// `args` as the agent's command line when its program (or an interpreter's script, the second
/// word) is `agent`: the agent's name, then the words after it, redacted and cut.
fn as_agent(args: &str, agent: &str) -> Option<String> {
    let words: Vec<&str> = args.split_whitespace().collect();
    let at = words.iter().take(2).position(|w| w.rsplit('/').next() == Some(agent))?;
    let line = std::iter::once(agent).chain(words[at + 1..].iter().copied()).collect::<Vec<_>>().join(" ");
    Some(crate::sent::redact(&line).chars().take(crate::tmux::hookstate::ARGV_MOST).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac153_the_agents_program_is_named_by_the_agent_and_its_words_follow() {
        assert_eq!(as_agent("claude --dangerously-skip-permissions --model opus", "claude").as_deref(),
                   Some("claude --dangerously-skip-permissions --model opus"));
        assert_eq!(as_agent("/Users/alex/.local/bin/claude", "claude").as_deref(), Some("claude"));
        assert_eq!(as_agent("node /opt/homebrew/bin/codex --yolo", "codex").as_deref(), Some("codex --yolo"), "an interpreter's script");
        assert_eq!(as_agent("/opt/homebrew/lib/node_modules/@openai/codex/vendor/aarch64-apple-darwin/codex/codex -m gpt", "codex").as_deref(),
                   Some("codex -m gpt"));
        for not in ["/bin/sh -c terminum hook claude", "terminum hook claude", "zsh", "claude-helper x", ""] {
            assert_eq!(as_agent(not, "claude"), None, "{not:?}");
        }
        assert_eq!(as_agent("claude", "codex"), None, "another agent");
    }

    #[test]
    fn ac153_secrets_in_the_command_line_are_left_out_and_it_is_cut() {
        let got = as_agent("claude --api-key opensesame-value --model opus", "claude").unwrap();
        assert!(!got.contains("opensesame"), "{got}");
        assert!(got.starts_with("claude --api-key"), "{got}");
        assert_eq!(as_agent(&format!("claude {}", "x".repeat(1000)), "claude").unwrap().chars().count(), crate::tmux::hookstate::ARGV_MOST);
    }

    #[test]
    fn ac153_no_ancestor_of_this_test_is_an_agent() {
        assert_eq!(started_as("no-such-agent"), None);
    }
}
