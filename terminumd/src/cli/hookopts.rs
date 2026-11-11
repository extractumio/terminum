// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Which of an agent's launch words its resume takes again (`terminum-agents.md` AC-153's summary):
//! its options (`--dangerously-skip-permissions`, `--model opus`), never its prompt, which would be
//! sent again, nor the options that pick or start a session. A word is a value or the prompt by the
//! agent's own options (`claude --help` of 2.1, `codex --help` and `codex resume --help` of 0.167);
//! where that cannot be told (an option this table does not know or one whose value is optional,
//! followed by a bare word), none is taken and the summary points at the command line shown.

/// Claude Code's options and Codex's, by what follows them and whether a resume takes them again.
struct Table {
    /// kept, with one word after them
    value: &'static [&'static str],
    /// kept, with every bare word up to the next option
    many: &'static [&'static str],
    /// kept, alone
    flag: &'static [&'static str],
    /// left out (they pick, start or end a session), alone
    gone: &'static [&'static str],
    /// left out, with the bare word after them (a session's id or name, optional)
    gone_value: &'static [&'static str],
    /// left out, with every bare word after them (the first prompt's images)
    gone_many: &'static [&'static str],
}

const CLAUDE: Table = Table {
    value: &["--agent", "--agents", "--append-system-prompt", "--append-system-prompt-file", "--system-prompt", "--system-prompt-file",
             "--autocompact", "--debug-file", "--effort", "--environment", "--fallback-model", "--input-format", "--json-schema",
             "--max-budget-usd", "--max-turns", "--model", "-n", "--name", "--output-format", "--permission-mode", "--permission-prompts",
             "--permission-prompt-tool", "--plugin-dir", "--plugin-url", "--remote-control-session-name-prefix", "--setting-sources",
             "--settings", "--system-prompt-snapshot"],
    many: &["--add-dir", "--allowedTools", "--allowed-tools", "--disallowedTools", "--disallowed-tools", "--betas", "--file",
            "--mcp-config", "--tools"],
    flag: &["--dangerously-skip-permissions", "--allow-dangerously-skip-permissions", "--verbose", "--ide", "--chrome", "--no-chrome",
            "--brief", "--bare", "--strict-mcp-config", "--disable-slash-commands", "--ax-screen-reader", "--safe-mode", "--restricted",
            "--exclude-dynamic-system-prompt-sections", "--no-session-persistence", "--forward-subagent-text", "--include-hook-events",
            "--include-partial-messages", "--replay-user-messages"],
    gone: &["-c", "--continue", "-p", "--print", "--fork-session", "--tmux", "--bg", "--background", "--desktop"],
    gone_value: &["-r", "--resume", "--from-pr", "--teleport", "--cloud", "-w", "--worktree", "--session-id"],
    gone_many: &[],
};

const CODEX: Table = Table {
    value: &["-c", "--config", "--enable", "--disable", "--remote", "--remote-auth-token-env", "-m", "--model", "--local-provider",
             "-p", "--profile", "-s", "--sandbox", "-a", "--ask-for-approval", "-C", "--cd", "--add-dir"],
    many: &[],
    flag: &["--strict-config", "--oss", "--approve-for-me", "--dangerously-bypass-approvals-and-sandbox", "--yolo",
            "--dangerously-bypass-hook-trust", "--search", "--no-alt-screen", "--no-daemon", "--full-auto"],
    gone: &["--last", "--all", "--include-non-interactive", "--worktree"],
    gone_value: &[],
    gone_many: &["-i", "--image"],
};

/// `words` (after the program) as `agent`'s options for its resume, None where that cannot be told
/// or a secret is among them.
pub fn resumed(agent: &str, words: &[String]) -> Option<Vec<String>> {
    let table = match agent {
        "claude" => &CLAUDE,
        "codex" => &CODEX,
        _ => return None,
    };
    let bare = |w: &String| !w.starts_with('-') || w == "-";
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = &words[i];
        i += 1;
        if w == "--" {
            break;                                      // the rest is the prompt
        }
        if bare(w) {
            continue;                                   // the prompt, or Codex's `resume` and its id
        }
        let (opt, inline) = match w.split_once('=') {
            Some((o, _)) if o.starts_with("--") => (o, true),
            _ => (w.as_str(), false),
        };
        let is = |list: &[&str]| list.contains(&opt);
        let left_out = is(table.gone) || is(table.gone_value) || is(table.gone_many);
        let takes = if inline { 0 }
            else if is(table.value) || is(table.gone_value) { 1 }
            else if is(table.many) || is(table.gone_many) { usize::MAX }
            else if is(table.flag) || is(table.gone) { 0 }
            else if words.get(i).is_some_and(bare) { return None } // an option not known, then a bare word: its value or the prompt
            else { 0 };
        let end = (i..words.len()).take_while(|j| bare(&words[*j])).take(takes).last().map_or(i, |j| j + 1);
        if !left_out {
            out.extend(words[i - 1..end].iter().cloned());
        }
        i = end;
    }
    let joined = out.join(" ");
    (joined.chars().count() <= crate::tmux::hookstate::OPTS_MOST && crate::sent::redact(&joined) == joined).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(agent: &str, line: &str) -> Option<String> {
        let words: Vec<String> = line.split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
        resumed(agent, &words).map(|o| o.join(" "))
    }

    #[test]
    fn ac153_claude_codes_options_are_kept_and_its_prompt_is_not() {
        assert_eq!(r("claude", "--dangerously-skip-permissions").as_deref(), Some("--dangerously-skip-permissions"));
        assert_eq!(r("claude", "--dangerously-skip-permissions --model opus fix").as_deref(), Some("--dangerously-skip-permissions --model opus"));
        assert_eq!(r("claude", "fix --model=opus --verbose").as_deref(), Some("--model=opus --verbose"), "options after the prompt");
        assert_eq!(r("claude", "--add-dir /a /b --effort high").as_deref(), Some("--add-dir /a /b --effort high"), "many values, as its parser reads them");
        assert_eq!(r("claude", "--model opus -- --not-an-option").as_deref(), Some("--model opus"));
        assert_eq!(r("claude", "").as_deref(), Some(""));
    }

    #[test]
    fn ac153_options_that_pick_or_start_a_session_are_left_out() {
        assert_eq!(r("claude", "--resume abc --model opus").as_deref(), Some("--model opus"));
        assert_eq!(r("claude", "-r --model opus").as_deref(), Some("--model opus"));
        assert_eq!(r("claude", "-c -p --session-id 9 --fork-session -w wt --verbose").as_deref(), Some("--verbose"));
        assert_eq!(r("codex", "resume 3f2a --yolo -m gpt").as_deref(), Some("--yolo -m gpt"));
        assert_eq!(r("codex", "--dangerously-bypass-approvals-and-sandbox -i a.png b.png -c k=v fix it").as_deref(),
                   Some("--dangerously-bypass-approvals-and-sandbox -c k=v"));
        assert_eq!(r("codex", "resume --last --all").as_deref(), Some(""));
    }

    #[test]
    fn ac153_an_option_not_known_before_a_bare_word_leaves_them_all_out() {
        assert_eq!(r("claude", "--new-thing fix"), None, "its value, or the prompt: not told");
        assert_eq!(r("claude", "--new-thing --model opus").as_deref(), Some("--new-thing --model opus"), "a flag: kept");
        assert_eq!(r("claude", "--new-thing=x fix").as_deref(), Some("--new-thing=x"));
        assert_eq!(r("codex", "fix the login --yolo").as_deref(), Some("--yolo"), "Codex's prompt first");
        assert_eq!(r("opencode", "--x"), None, "an agent not known");
    }

    #[test]
    fn ac153_a_secret_or_a_long_line_among_the_options_leaves_them_all_out() {
        let shaped = format!("{}{}", "sk-", "a".repeat(40));       // a key's shape, split for the commit's scan
        assert_eq!(r("claude", &format!("--model opus --append-system-prompt {shaped}")), None);
        assert_eq!(r("claude", "--model opus --api-key=opensesame-value"), None, "a value named as a secret");
        assert_eq!(r("claude", &format!("--append-system-prompt {}", "x".repeat(crate::tmux::hookstate::OPTS_MOST))), None);
    }
}
