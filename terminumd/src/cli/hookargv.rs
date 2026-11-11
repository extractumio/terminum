// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The command line a coding agent started with, as its hook sees it (`terminum-agents.md` AC-153's
//! summary): the nearest of the hook's ancestors whose program is the agent (an alias's expansion
//! and a wrapper script's child are: `ccdsp` runs `claude --dangerously-skip-permissions`), else
//! the nearest that is not a shell (a renamed link: `cc`), read with `ps` one process at a time (a
//! few ms each; the whole table costs tens), once a session, when terminumd asks. Its words are
//! read exactly (the kernel's argument vector: `ps` joins them with spaces), its program named by
//! the agent (`codex --yolo`, not a vendor binary's path); secrets' values are left out. The options
//! its resume takes again go with it (`hookopts`), only from the agent's own exact words: a process
//! of another name may be a wrapper the agent runs hooks under.

use std::process::Command;

use serde_json::{json, Map, Value};

use crate::termtext::shell_name as name;

/// The ancestors looked at: the hook's shells, a sandbox, the agent, an interpreter.
const UP: usize = 4;
/// Programs passed over besides shells: what runs the hook.
const RUNNERS: [&str; 3] = ["env", "terminum", "terminumd"];
/// Programs whose script is the program (`node …/codex`).
const INTERPRETERS: [&str; 3] = ["node", "bun", "deno"];

/// The `Argv` event's fields for `agent`: `argv` (shown; "" where no ancestor is the agent or `ps`
/// does not answer) and, where known, `opts`.
pub fn started_as(agent: &str) -> Map<String, Value> {
    let (mut other, mut pid) = (None, std::os::unix::process::parent_id());
    for _ in 0..UP {
        let Some(out) = Command::new("ps").args(["-ww", "-o", "ppid=,args=", "-p", &pid.to_string()]).output().ok() else { break };
        let row = String::from_utf8_lossy(&out.stdout);
        let Some((ppid, args)) = row.trim().split_once(char::is_whitespace) else { break };
        let words: Vec<String> = args.split_whitespace().map(str::to_string).collect();
        if let Some(at) = program_at(&words, agent) {
            return match exact(pid).and_then(|e| Some((program_at(&e, agent)?, e))) {
                Some((at, exact)) => said(agent, &exact, Some(at), true),
                None => said(agent, &words, Some(at), false),
            };
        }
        let first = words.first().map_or("", String::as_str);
        if other.is_none() && !crate::tmux::titles::is_shell(first) && !RUNNERS.contains(&name(first)) {
            other = Some((pid, words));
        }
        let Some(next) = ppid.trim().parse().ok().filter(|p| *p > 1) else { break };
        pid = next;
    }
    match other {
        Some((pid, words)) => said(agent, &exact(pid).filter(|e| !e.is_empty()).unwrap_or(words), None, false),
        None => Map::from_iter([("argv".to_string(), json!(""))]),
    }
}

/// Where in `words` the agent's program is: first, or an interpreter's script.
fn program_at(words: &[String], agent: &str) -> Option<usize> {
    let at = usize::from(words.len() > 1 && INTERPRETERS.contains(&name(&words[0])));
    (name(words.get(at)?) == agent).then_some(at)
}

/// The fields for an ancestor's words `words`, its program at `at` where it is the agent; `exact`:
/// they are the agent's own as the kernel keeps them (not split from `ps`), so its options are resumed.
fn said(agent: &str, words: &[String], at: Option<usize>, exact: bool) -> Map<String, Value> {
    let (program, rest) = match at {
        Some(at) => (agent, &words[at + 1..]),
        None => (name(words.first().map_or("", String::as_str)), words.get(1..).unwrap_or_default()),
    };
    let shown = std::iter::once(program.to_string()).chain(rest.iter().map(|w| super::commands::quote(w))).collect::<Vec<_>>().join(" ");
    let argv: String = crate::sent::redact(&shown).chars().take(crate::tmux::hookstate::ARGV_MOST).collect();
    let mut out = Map::from_iter([("argv".to_string(), json!(argv))]);
    if exact && let Some(opts) = super::hookopts::resumed(agent, rest) {
        out.insert("opts".into(), json!(opts));
    }
    out
}

/// Process `pid`'s argument vector as the kernel keeps it.
#[cfg(target_os = "linux")]
fn exact(pid: u32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(raw.split(|b| *b == 0).filter(|w| !w.is_empty()).map(|w| String::from_utf8_lossy(w).into_owned()).collect())
}

/// Process `pid`'s argument vector as the kernel keeps it (`KERN_PROCARGS2`: argc, the program's
/// path, padding, then argc words, each ending in NUL).
#[cfg(target_os = "macos")]
fn exact(pid: u32) -> Option<Vec<String>> {
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
fn exact(_pid: u32) -> Option<Vec<String>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<String> {
        s.split(' ').map(str::to_string).collect()
    }

    #[test]
    fn ac153_the_agents_program_is_named_by_the_agent_and_its_words_follow_quoted() {
        let got = said("claude", &["/Users/alex/.local/bin/claude".into(), "--model".into(), "opus".into(), "fix the bug".into()], Some(0), true);
        assert_eq!(got["argv"], "claude --model opus 'fix the bug'");
        assert_eq!(got["opts"], json!(["--model", "opus"]), "the prompt is not resumed");
        assert_eq!(said("codex", &w("node /opt/homebrew/bin/codex --yolo"), Some(1), true)["argv"], "codex --yolo", "an interpreter's script");
        assert_eq!(said("codex", &w("/opt/homebrew/lib/node_modules/@openai/codex/vendor/aarch64-apple-darwin/codex/codex -m gpt"), Some(0), true)["argv"],
                   "codex -m gpt");
        for (words, at) in [("claude x", Some(0)), ("node /a/claude", Some(1)), ("/bin/sh -c terminum hook claude", None),
                            ("claude-helper x", None), ("zsh", None), ("", None)] {
            assert_eq!(program_at(&w(words), "claude"), at, "{words:?}");
        }
    }

    #[test]
    fn ac153_a_program_of_another_name_is_shown_but_its_options_never_resumed() {
        let got = said("claude", &w("/Users/alex/bin/cc --dangerously-skip-permissions"), None, false);
        assert_eq!(got["argv"], "cc --dangerously-skip-permissions");
        assert!(got.get("opts").is_none(), "it may be a wrapper the agent runs hooks under: {got:?}");
    }

    #[test]
    fn ac153_words_split_from_ps_are_shown_but_never_resumed() {
        let got = said("claude", &w("claude --model opus"), Some(0), false);
        assert_eq!(got["argv"], "claude --model opus");
        assert!(got.get("opts").is_none(), "{got:?}");
    }

    #[test]
    fn ac153_secrets_in_the_command_line_are_left_out_and_it_is_cut() {
        let got = said("claude", &w("claude --api-key opensesame-value --model opus"), Some(0), true);
        assert!(!got["argv"].as_str().unwrap().contains("opensesame"), "{got:?}");
        assert!(got.get("opts").is_none(), "a secret among them: none resumed: {got:?}");
        let long = said("claude", &["claude".into(), "x".repeat(1000)], Some(0), true);
        assert_eq!(long["argv"].as_str().unwrap().chars().count(), crate::tmux::hookstate::ARGV_MOST);
    }

    #[test]
    fn ac153_shells_and_login_shells_are_passed_over() {
        let passed_over = |w: &str| crate::tmux::titles::is_shell(w) || RUNNERS.contains(&name(w));
        for s in ["/bin/sh", "-zsh", "bash", "/usr/bin/env", "terminum"] {
            assert!(passed_over(s), "{s}");
        }
        assert!(!passed_over("/Users/alex/.local/bin/claude"));
    }

    #[test]
    fn ac153_this_processs_parent_is_read_exactly() {
        let parent = std::os::unix::process::parent_id();
        let words = exact(parent).expect("the kernel's vector");
        assert!(!words.is_empty() && !words[0].is_empty(), "{words:?}");
    }
}
