// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What a coding agent in a pane is doing, for the list's status icon (AC-39): working,
//! waiting for the user, failed, or done. Claude Code puts a spinner in front of its terminal
//! title while it works and ✳ when it is idle (measured: ◐ ◑ ◒ ◓ turning, ✳ at rest); a
//! question or a failure shows only on its screen, so the last rows are read for those. Codex
//! (measured on 0.162) keeps its title while it works and marks it only when it asks.
//! Ported from iterm-enhancer's `agentstate.py`, its regular expressions as small matchers.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Rows of the screen's end that count.
pub const ROWS: usize = 25;
/// A question is near the bottom: older text above it is history.
pub const ASK_ROWS: usize = 12;
/// How long a state stands while the title stays the same.
pub const STATE_EVERY: Duration = Duration::from_secs(6);
/// Panes remembered before the cache starts anew (panes long closed).
const KEEP: usize = 500;

const WAITING: [&str; 10] = ["do you want to", "would you like to", "esc to cancel", "(y/n)", "[y/n]",
                             "allow command", "allow edit", "allow this", "press enter to",
                             "enter to submit answer"];
const FAILED: [&str; 10] = ["api error", "overloaded_error", "request timed out",
                            "credit balance is too low", "usage limit reached", "claude code crashed",
                            "hit your usage limit", "out of credits", "exceeded retry limit",
                            "experiencing high load"];
/// Codex's own words on its screen (its header, composer, approval and question): a pane whose
/// program is `node` (Codex installed with npm) runs Codex while its screen shows one.
const CODEX_WORDS: [&str; 5] = ["openai codex (v", "ask codex to do anything", "tell codex what to do differently",
                                "enter to submit answer", "tab to add notes"];
/// What Claude Code's footer counts of the work it runs in the background, after the number
/// ("1 shell, 1 monitor", "2 local agents"; measured in its code): its session is not done while
/// one runs. An Artifact comment monitor, a dream or a memory import is no work of a turn.
/// Rows of its footer: the status line, the mode and the counts, a notice.
const FOOTER: usize = 4;
const BACKGROUND: [&str; 9] = ["shell", "monitor", "local agent", "team", "background dynamic workflow",
                               "remote dynamic workflow", "cloud session", "mcp task", "background task"];
/// Codex's title while it waits for the user, blinking: "[ ! ] Action Required | <thread> | <folder>".
pub const CODEX_ASKS: [&str; 2] = ["[ ! ] Action Required", "[ . ] Action Required"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Working,
    Waiting,
    Failed,
    Done,
}

impl State {
    /// The layout item's `state`.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Working => "working",
            State::Waiting => "waiting",
            State::Failed => "failed",
            State::Done => "done",
        }
    }
}

/// A spinner frame: ◐◑◒◓, ✢✶✻✽, ·, braille.
fn spinner(c: char) -> bool {
    matches!(c, '\u{25d0}'..='\u{25d3}' | '\u{2722}' | '\u{2736}' | '\u{273b}' | '\u{273d}' | '\u{b7}'
             | '\u{2800}'..='\u{28ff}')
}

fn spinning(title: &str) -> bool {
    title.chars().next().is_some_and(spinner)
}

/// Claude Code's own marks in front of a title (a spinner or ✳).
pub fn claude_marked(title: &str) -> bool {
    spinning(title) || title.starts_with('\u{2733}')
}

/// Whether Codex's title says it waits for the user.
pub fn codex_asks(title: &str) -> bool {
    CODEX_ASKS.iter().any(|m| title.starts_with(m))
}

/// Whether a screen is Codex's (for a `node` pane); the title is not asked, as a killed Codex
/// can leave its mark behind for the next program.
pub fn codex_screen(rows: &[String]) -> bool {
    any_of(&rows.join("\n"), &CODEX_WORDS)
}

fn any_of(text: &str, words: &[&str]) -> bool {
    let low = text.to_lowercase();
    words.iter().any(|w| low.contains(w))
}

/// Claude Code's status line while it works, "✻ Combobulating… (6m 8s · ↓ 31.5k tokens)": at a
/// line's start, blanks, a mark (✢…✽, ·, *), blanks, 1 to 80 characters of one line, "…",
/// blanks, "(" (the regular expression of `agentstate.py`, matched by hand).
fn status_line(tail: &[char]) -> bool {
    let ws = |c: char| c.is_whitespace();
    let mark = |c: char| matches!(c, '\u{2722}'..='\u{273d}' | '\u{b7}' | '*');
    (0..tail.len()).filter(|&p| p == 0 || tail[p - 1] == '\n').any(|p| {
        let mut i = p;
        while i < tail.len() && ws(tail[i]) {
            i += 1;
        }
        if !tail.get(i).copied().is_some_and(mark) || !tail.get(i + 1).copied().is_some_and(ws) {
            return false;
        }
        // the 1..80 characters may begin anywhere in the blanks after the first one
        let mut start = i + 2;
        loop {
            if text_then_paren(tail, start) {
                return true;
            }
            if start < tail.len() && ws(tail[start]) {
                start += 1;
            } else {
                return false;
            }
        }
    })
}

/// Whether `tail[start..]` is 1 to 80 characters of one line, "…", blanks, then "(".
fn text_then_paren(tail: &[char], start: usize) -> bool {
    (start + 1..=start + 80).take_while(|&k| k <= tail.len() && tail[k - 1] != '\n').any(|k| {
        if tail.get(k) != Some(&'\u{2026}') {
            return false;
        }
        let mut j = k + 1;
        while j < tail.len() && tail[j].is_whitespace() {
            j += 1;
        }
        tail.get(j) == Some(&'(')
    })
}

/// Whether Claude Code's newest turn line says its subagents still run: "✻ Waiting for 2
/// background agents to finish" (also "1 background agent and 1 dynamic workflow", "dynamic
/// workflows"; measured in its code), not a later "✻ Worked for 3m 2s" (a turn after they ended).
/// Its turn goes on until they end.
fn background_work(tail: &str) -> bool {
    let mark = |c: char| matches!(c, '\u{2722}'..='\u{273d}' | '\u{b7}' | '*');
    let turn_line = |l: &str| -> Option<bool> {
        let mut c = l.trim_start().chars();
        if !c.next().is_some_and(mark) || c.next() != Some(' ') {
            return None;
        }
        let rest = c.as_str().to_lowercase();
        if rest.starts_with("waiting for") && rest.contains("to finish") && (rest.contains(" background agent") || rest.contains(" dynamic workflow")) {
            return Some(true);
        }
        // "Worked for 2m 12s", "Cooked for 9s": its verb, " for ", a duration
        let (verb, after) = rest.split_once(" for ")?;
        (!verb.contains(' ') && after.starts_with(|c: char| c.is_ascii_digit())).then_some(false)
    };
    tail.lines().rev().find_map(turn_line).unwrap_or(false)
}

/// Whether Claude Code's footer counts work in the background: the rows (at most FOOTER) under
/// its input box (a rule, "❯ " and what is typed, a rule), "⏵⏵ bypass permissions on · 1 shell,
/// 1 monitor · ← 1 agent": each item exactly a number and a word of BACKGROUND. It is live; a turn
/// line's "· 1 shell still running" stays when the shell has ended.
pub fn background_tasks(rows: &[String]) -> bool {
    let rule = |r: &String| r.trim_start().starts_with("\u{2500}\u{2500}");
    let Some(bottom) = rows.iter().rposition(rule) else { return false };
    let Some(top) = rows[..bottom].iter().rposition(rule) else { return false };
    if !rows[top + 1].trim_start().starts_with('\u{276f}') {
        return false;                                       // not Claude Code's input box (Codex: "›")
    }
    rows[bottom + 1..].iter().take(FOOTER).any(|r| r.split(" \u{b7} ").flat_map(|part| part.split(", ")).any(|item| {
        let item = item.trim().trim_start_matches(|c: char| !c.is_alphanumeric()).to_lowercase();   // a mark before ("◇ 1 cloud session")
        let Some((n, what)) = item.split_once(' ') else { return false };
        let what = what.trim_end();
        n.parse::<u32>().is_ok_and(|n| n > 0) && BACKGROUND.iter().any(|w| what == *w || what.strip_suffix('s') == Some(*w))
    }))
}

/// The state from the raw title and the screen's last rows.
pub fn state_of(title: &str, rows: &[String]) -> State {
    let rows: Vec<String> = rows.iter().map(|r| r.replace('\0', " ")).collect();   // skipped cells
    let tail = rows[rows.len().saturating_sub(ROWS)..].join("\n");
    if codex_asks(title) || any_of(&rows[rows.len().saturating_sub(ASK_ROWS)..].join("\n"), &WAITING) {
        return State::Waiting;
    }
    let chars: Vec<char> = tail.chars().collect();
    let working = spinning(title) || any_of(&tail, &["esc to interrupt"]) || status_line(&chars) || background_work(&tail)
        || background_tasks(&rows[rows.len().saturating_sub(ROWS)..]);
    if !working && any_of(&tail, &FAILED) {
        return State::Failed;
    }
    if working { State::Working } else { State::Done }
}

/// The last state per pane key, read again only when the title changed (Claude Code turns its
/// spinner while it works) or [`STATE_EVERY`] passed: the list asks every 2 s for every agent.
#[derive(Debug, Default)]
pub struct States {
    /// key -> title, read at, state, and whether a `node` pane's screen was Codex's
    known: HashMap<String, (String, Instant, State, bool)>,
}

impl States {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether pane `key` with `title` needs its screen read at `now`.
    pub fn due(&self, key: &str, title: &str, now: Instant) -> bool {
        match self.known.get(key) {
            Some((t, at, ..)) => t != title || now.saturating_duration_since(*at) >= STATE_EVERY,
            None => true,
        }
    }

    /// The state last stored for `key`.
    pub fn last(&self, key: &str) -> Option<State> {
        self.known.get(key).map(|k| k.2)
    }

    /// Whether the screen last read for `key` was Codex's.
    pub fn codex(&self, key: &str) -> bool {
        self.known.get(key).is_some_and(|k| k.3)
    }

    /// Remembers `state`, read at `now` for `title`, and whether the screen was Codex's.
    pub fn store(&mut self, key: &str, title: &str, now: Instant, state: State, codex: bool) {
        if self.known.len() >= KEEP && !self.known.contains_key(key) {
            self.known.clear();
        }
        self.known.insert(key.to_string(), (title.to_string(), now, state, codex));
    }

    /// Forgets a pane (its state is read anew).
    #[cfg(test)]
    pub fn forget(&mut self, key: &str) {
        self.known.remove(key);
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use State::*;

    fn rows(r: &[&str]) -> Vec<String> {
        r.iter().map(|s| s.to_string()).collect()
    }

    fn footer() -> Vec<String> {
        rows(&["────", "❯ ", "────", "  ~/p Opus 5.5 ctx:47%"])
    }

    fn with_footer(r: &[&str]) -> Vec<String> {
        let mut v = rows(r);
        v.extend(footer());
        v
    }

    #[test]
    fn title_marks() {
        assert_eq!(state_of("◐ Enable web", &footer()), Working);
        assert_eq!(state_of("◓ Enable web", &footer()), Working);
        assert_eq!(state_of("✳ Enable web", &footer()), Done);
        assert!(claude_marked("✳ x") && claude_marked("◑ x") && claude_marked("⠋ x"));
        assert!(!claude_marked("mc [root@devbox.example]") && !claude_marked(""));
    }

    #[test]
    fn the_screen_tells_waiting_and_working() {
        let ask = rows(&["Do you want to proceed?", "❯ 1. Yes", "  2. No", "Esc to cancel"]);
        assert_eq!(state_of("✳ x", &ask), Waiting);
        assert_eq!(state_of("◐ x", &ask), Waiting, "a question wins over the spinner");
        assert_eq!(state_of("", &with_footer(&["✻ Thinking… (12s · esc to interrupt)"])), Working);
        assert_eq!(state_of("", &rows(&["Allow edit to src/a.rs?"])), Waiting);
        assert_eq!(state_of("", &rows(&["Continue? [Y/n]"])), Waiting, "case-insensitive");
    }

    #[test]
    fn claude_codes_status_line_without_title_marks() {
        let working = with_footer(&["\u{273b}\0Combobulating\u{2026} (6m\08s\0\u{b7}\0\u{2193}\x0031.5k\0tokens)", ""]);
        let done = with_footer(&["\u{273b} Worked for 2m 12s \u{b7} done 8:17 AM \u{b7} 1 shell still running", ""]);
        assert_eq!(state_of("Demo stand preparation", &working), Working);
        assert_eq!(state_of("Demo stand preparation", &done), Done);
    }

    #[test]
    fn background_subagents_keep_the_turn_going() {
        for line in ["\u{273b} Waiting for 2 background agents to finish", "\u{273b} Waiting for 1 background agent and 1 dynamic workflow to finish",
                     "\u{273b} Waiting for 3 dynamic workflows to finish"] {
            assert_eq!(state_of("\u{2733} Fix it", &with_footer(&[line, ""])), Working, "{line}");
        }
        assert_eq!(state_of("\u{2733} x", &with_footer(&["waiting for the build to finish", ""])), Done, "words of the transcript");
        let after = with_footer(&["\u{273b} Waiting for 2 background agents to finish", "\u{23fa} Both agents are done.", "\u{273b} Worked for 3m 2s", ""]);
        assert_eq!(state_of("\u{2733} x", &after), Done, "a turn after they ended: its own line counts");
    }

    #[test]
    fn background_work_in_the_footer_keeps_the_session_going() {
        let turn = ["\u{273b} Worked for 2m 12s \u{b7} 1 shell still running", "", "\u{2500}\u{2500}\u{2500}\u{2500}", "\u{276f} ", "\u{2500}\u{2500}\u{2500}\u{2500}", "  ~/p Opus 5.5 ctx:47%"];
        let with = |pill: &str| { let mut v = rows(&turn); v.push(pill.to_string()); v };
        for pill in ["  \u{23f5}\u{23f5} bypass permissions on \u{b7} 1 shell, 1 monitor \u{b7} \u{2190} 1 agent      current: 2.1.302",
                     "  1 shell", "  2 monitors \u{b7} ? for shortcuts", "  \u{23f5}\u{23f5} accept edits on \u{b7} 2 local agents",
                     "  1 background dynamic workflow", "  \u{25c7} 1 cloud session", "  3 background tasks", "  1 MCP task", "  2 teams"] {
            assert_eq!(state_of("\u{2733} Fix it", &with(pill)), Working, "{pill}");
        }
        for pill in ["  \u{23f5}\u{23f5} bypass permissions on \u{b7} \u{2190} 1 agent", "  ? for shortcuts", "  1 Artifact comment monitor", "  dreaming"] {
            assert_eq!(state_of("\u{2733} Fix it", &with(pill)), Done, "{pill}: the turn line's shell has ended");
        }
        let above = rows(&["  1 shell", "\u{2500}\u{2500}\u{2500}\u{2500}", "\u{276f} ", "\u{2500}\u{2500}\u{2500}\u{2500}", "  ~/p ctx:47%"]);
        assert_eq!(state_of("\u{2733} x", &above), Done, "words above the input box are the transcript");
        for pill in ["  1 Artifact comment monitor, 1 shell", "  dreaming, 2 shells \u{b7} ? for shortcuts", "  1 teammate \u{b7} 1 shell"] {
            assert_eq!(state_of("\u{2733} Fix it", &with(pill)), Working, "{pill}");
        }
        for pill in ["  1 shell script added", "  3 monitors found", "  1 teammate"] {
            assert_eq!(state_of("\u{2733} Fix it", &with(pill)), Done, "{pill}: no count of Claude Code's");
        }
        let md = rows(&["\u{2500}\u{2500}\u{2500}\u{2500}", "## Notes", "\u{2500}\u{2500}\u{2500}\u{2500}", "  1 shell"]);
        assert!(!background_tasks(&md), "a rule of the transcript, no input box under it");
        let codex = rows(&["\u{2500}\u{2500}\u{2500}\u{2500}", "\u{203a} Ask Codex to do anything", "\u{2500}\u{2500}\u{2500}\u{2500}", "  1 shell"]);
        assert!(!background_tasks(&codex), "Codex's composer is no Claude Code input box");
        let typed = rows(&["\u{2500}\u{2500}\u{2500}\u{2500}", "\u{276f} first line", "  second line", "\u{2500}\u{2500}\u{2500}\u{2500}", "  ~/p ctx:47%", "  1 shell"]);
        assert!(background_tasks(&typed), "a prompt of two lines in the box");
    }

    #[test]
    fn status_line_shapes() {
        let s = |t: &str| status_line(&t.chars().collect::<Vec<_>>());
        assert!(s("  * Baking… (3s)"));
        assert!(s("out\n\u{b7}   Wait\u{2026}   ("));
        assert!(s("✶  …("), "the blanks may start the text");
        assert!(!s("✶ …("), "at least one character before …");
        assert!(!s("x ✻ Baking… ("), "only at a line's start");
        assert!(!s("✻Baking… ("), "a blank after the mark");
        assert!(!s("✻ Bak\ning… ("), "one line");
        assert!(s(&format!("✻ {}… (", "a".repeat(80))));
        assert!(!s(&format!("✻ {}… (", "a".repeat(81))));
        assert!(s("✻ Baking…\n  ("), "blanks after … may cross a line, as \\s does");
    }

    /// Codex 0.162's composer and footer under its transcript (measured).
    fn codex(r: &[&str]) -> Vec<String> {
        let mut v = rows(r);
        v.extend(rows(&["", "› Ask Codex to do anything", "", "  GPT-6.1-Sol high · ~/p", "  ? for shortcuts"]));
        v
    }

    #[test]
    fn codex_measured_screens() {
        let thread = "Write tmux control mode essay | p";
        let working = codex(&["• Ran pwd", "  └ /Users/alex/p", "", "Working (10s • esc to interrupt)"]);
        assert_eq!(state_of(thread, &working), Working, "Codex keeps its title while it works");
        assert_eq!(state_of("⠋ | p", &codex(&[])), Working, "a spinner before the thread is named");
        let done = codex(&["• Created essay.md and verified it contains 400 words.", "", "  Worked for 43s • 11:04"]);
        assert_eq!(state_of(thread, &done), Done);
        let run = rows(&["• Running curl -sI https://example.com", "", "  Would you like to run the following command?",
                         "  $ curl -sI https://example.com", "› 1. Yes, proceed (y)",
                         "  3. No, and tell Codex what to do differently (esc)", "", "  Press enter to confirm or esc to cancel"]);
        assert_eq!(state_of("[ ! ] Action Required | Check headers | p", &run), Waiting);
        let ask = rows(&["  Question 1/1 (1 unanswered)", "  Which language should the script use?", "  › 1. Python",
                         "    2. Bash", "", "  tab to add notes | enter to submit answer | esc to interrupt"]);
        assert_eq!(state_of("[ . ] Action Required | Plan | p", &ask), Waiting, "the title's blink, either frame");
        assert_eq!(state_of(thread, &ask), Waiting, "a question over \"esc to interrupt\", by its screen alone");
        assert_eq!(state_of("[ ! ] Action Required | p", &codex(&[])), Waiting, "the title alone");
    }

    #[test]
    fn a_node_pane_is_codex_by_its_screen() {
        assert!(codex_screen(&codex(&[])), "the composer");
        assert!(codex_screen(&rows(&["  >_ OpenAI Codex (v0.162.1)", "› a draft I have not sent"])), "the header");
        assert!(codex_screen(&rows(&["  3. No, and tell Codex what to do differently (esc)"])), "an approval");
        assert!(codex_screen(&rows(&["  tab to add notes | enter to submit answer | esc to interrupt"])), "a question");
        assert!(!codex_screen(&rows(&["  ready - started server on 0.0.0.0:3000",
                                      "? Would you like to run the app on another port instead? › (Y/n)"])), "a dev server");
        assert!(!codex_screen(&rows(&["  VITE v5.0.0  ready in 300 ms", "  ➜  press h + enter to show help"])));
        assert!(!codex_screen(&[]));
    }

    #[test]
    fn codex_failures_and_a_cancel() {
        let limit = codex(&["■ You\u{2019}ve hit your usage limit. Visit https://chatgpt.com/settings/usage to purchase more credits"]);
        assert_eq!(state_of("t | p", &limit), Failed);
        assert_eq!(state_of("t | p", &codex(&["■ exceeded retry limit, last status: 503"])), Failed);
        assert_eq!(state_of("t | p", &codex(&["■ Codex is currently experiencing high load."])), Failed);
        let cancel = codex(&["✗ You canceled the request to run curl -sI https://example.com",
                             "■ Conversation interrupted - use /feedback if something went wrong"]);
        assert_eq!(state_of("t | p", &cancel), Done, "a turn the user stopped is not a failure");
        let quoted = codex(&["› Why did it say \"usage limit reached\"?", "Working (3s • esc to interrupt)"]);
        assert_eq!(state_of("t | p", &quoted), Working, "an error's words in the user's own prompt, while it works");
    }

    #[test]
    fn a_failure_that_ended_the_turn() {
        let err = with_footer(&["⎿  API Error: 529 {\"type\":\"overloaded_error\"}"]);
        assert_eq!(state_of("✳ x", &err), Failed);
        assert_eq!(state_of("◐ x", &err), Working, "still going after an error: not failed");
        assert_eq!(state_of("", &rows(&["Usage limit reached · resets 5pm"])), Failed);
    }

    #[test]
    fn only_the_last_rows_count() {
        let mut old = rows(&["Do you want to proceed?"]);
        old.extend(rows(&["output"; 40]));
        assert_eq!(state_of("✳ x", &old), Done);
        let mut near = rows(&["Do you want to proceed?"]);
        near.extend(rows(&["output"; 14]));
        assert_eq!(state_of("✳ x", &near), Done, "a question 15 rows up is history");
        let mut err = rows(&["API Error"]);
        err.extend(rows(&["output"; 25]));
        assert_eq!(state_of("✳ x", &err), Done, "a failure 26 rows up is history");
        assert_eq!(state_of("", &[]), Done);
    }

    #[test]
    fn the_screen_is_read_again_only_on_a_new_title_or_after_a_while() {
        let mut s = States::new();
        let t0 = Instant::now();
        let mut reads = 0;
        for (title, at) in [("✳ x", 0), ("✳ x", 1), ("◐ x", 2), ("◐ x", 3), ("◐ x", 3 + STATE_EVERY.as_secs())] {
            let now = t0 + Duration::from_secs(at);
            if s.due("tmux:local:terminum:%1", title, now) {
                reads += 1;
                s.store("tmux:local:terminum:%1", title, now, Waiting, false);
            }
            assert_eq!(s.last("tmux:local:terminum:%1"), Some(Waiting));
        }
        assert_eq!(reads, 3);
        assert!(s.due("tmux:local:terminum:%2", "✳ x", t0), "another pane");
        s.forget("tmux:local:terminum:%1");
        assert_eq!(s.last("tmux:local:terminum:%1"), None);
    }

    #[test]
    fn the_cache_stays_small() {
        let mut s = States::new();
        let now = Instant::now();
        for n in 0..KEEP + 1 {
            s.store(&format!("k{n}"), "", now, Done, false);
        }
        assert!(s.known.len() <= KEEP);
        assert_eq!(s.last(&format!("k{KEEP}")), Some(Done), "the newest is kept");
    }
}
