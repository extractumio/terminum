// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What a coding agent in a pane is doing, for the list's status icon (AC-39): working,
//! waiting for the user, failed, or done. Claude Code puts a spinner in front of its terminal
//! title while it works and ✳ when it is idle (measured: ◐ ◑ ◒ ◓ turning, ✳ at rest); a
//! question or a failure shows only on its screen, so the last rows are read for those.
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

const WAITING: [&str; 9] = ["do you want to", "would you like to", "esc to cancel", "(y/n)", "[y/n]",
                            "allow command", "allow edit", "allow this", "press enter to"];
const FAILED: [&str; 6] = ["api error", "overloaded_error", "request timed out",
                           "credit balance is too low", "usage limit reached", "claude code crashed"];

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

/// The state from the raw title and the screen's last rows.
pub fn state_of(title: &str, rows: &[String]) -> State {
    let rows: Vec<String> = rows.iter().map(|r| r.replace('\0', " ")).collect();   // skipped cells
    let tail = rows[rows.len().saturating_sub(ROWS)..].join("\n");
    if any_of(&rows[rows.len().saturating_sub(ASK_ROWS)..].join("\n"), &WAITING) {
        return State::Waiting;
    }
    let chars: Vec<char> = tail.chars().collect();
    let working = spinning(title) || any_of(&tail, &["esc to interrupt"]) || status_line(&chars);
    if !working && any_of(&tail, &FAILED) {
        return State::Failed;
    }
    if working { State::Working } else { State::Done }
}

/// The last state per pane key, read again only when the title changed (Claude Code turns its
/// spinner while it works) or [`STATE_EVERY`] passed: the list asks every 2 s for every agent.
#[derive(Debug, Default)]
pub struct States {
    known: HashMap<String, (String, Instant, State)>,
}

impl States {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether pane `key` with `title` needs its screen read at `now`.
    pub fn due(&self, key: &str, title: &str, now: Instant) -> bool {
        match self.known.get(key) {
            Some((t, at, _)) => t != title || now.saturating_duration_since(*at) >= STATE_EVERY,
            None => true,
        }
    }

    /// The state last stored for `key`.
    pub fn last(&self, key: &str) -> Option<State> {
        self.known.get(key).map(|k| k.2)
    }

    /// Remembers `state`, read at `now` for `title`.
    pub fn store(&mut self, key: &str, title: &str, now: Instant, state: State) {
        if self.known.len() >= KEEP && !self.known.contains_key(key) {
            self.known.clear();
        }
        self.known.insert(key.to_string(), (title.to_string(), now, state));
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
                s.store("tmux:local:terminum:%1", title, now, Waiting);
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
            s.store(&format!("k{n}"), "", now, Done);
        }
        assert!(s.known.len() <= KEEP);
        assert_eq!(s.last(&format!("k{KEEP}")), Some(Done), "the newest is kept");
    }
}
