// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Where a coding agent's answer ended (AC-90): at a turn's end terminumd reads the pane's
//! screen (at the hooks' Stop, else when a turn read from the screen ends: manager.rs remind),
//! finds the agent's input box from the bottom ("❯ " or Codex's "› "), and keeps the last row
//! above it that can be found again by its text (not blank, not a rule, not the turn's "Worked
//! for 43s" line, 4 letters and digits at least) with its line, a hint, in the project's
//! ends.jsonl (sent.rs). Nothing is kept for a full-screen program (no history), a screen still
//! at work or one without an input box. The answer's text never comes from the hooks.

use serde_json::json;

use crate::app::Shared;
use crate::tmux::agentstate::{self, State};

/// The fewest letters and digits a row needs to be found again (bookmarks_find.rs find_each).
const FEWEST: usize = 4;

/// The row where the answer above the agent's input box ends, in `rows` (the screen); None
/// without an input box: Claude Code's "❯ " between two rules (a question's options also start
/// with "❯ 1."), or Codex's "› " composer (not its menu's "› 1. Yes").
pub fn answer_row(rows: &[String]) -> Option<usize> {
    let rule = |r: &String| r.trim_start().starts_with("\u{2500}\u{2500}");
    let option = |t: &str| t.chars().nth(2).is_some_and(|c| c.is_ascii_digit());
    let at = (0..rows.len()).rev().find(|&i| {
        let t = rows[i].trim_start();
        if t.starts_with("\u{276f} ") || t == "\u{276f}" {
            !option(t) && i > 0 && rule(&rows[i - 1]) && rows.get(i + 1).is_some_and(rule)
        } else {
            t.starts_with("\u{203a} ") && !option(t)
        }
    })?;
    (0..at).rev().find(|&i| {
        let r = &rows[i];
        !r.trim().is_empty() && !rule(r) && !agentstate::turn_line(r) && r.chars().filter(|c| c.is_alphanumeric()).count() >= FEWEST
    })
}

/// Keeps where pane `key`'s answer ended (see the module); every page hears `endnew {key}`.
pub async fn keep(app: Shared, key: String) {
    let Some((server, pane)) = app.tmux.resolve(&key) else { return };
    let answers = server.run_all(vec![format!("display -p -t {pane} '#{{history_size}}\t#{{alternate_on}}\t#{{pane_current_path}}'"),
        format!("capture-pane -p -t {pane}")]).await;
    let (Some(Ok(fields)), Some(Ok(rows))) = (answers.first(), answers.get(1)) else { return };
    let mut f = fields.first().map(String::as_str).unwrap_or("").splitn(3, '\t');
    let (Some(Ok(history)), Some("0"), Some(path)) = (f.next().map(str::parse::<i64>), f.next(), f.next()) else { return };
    if path.is_empty() || matches!(agentstate::read("", rows).0, State::Working | State::Waiting) {
        return;
    }
    let Some(row) = answer_row(rows) else { return };
    let text: String = rows[row].trim().chars().take(super::bookmarks::PRINT).collect();
    let (a, host, path, k) = (app.clone(), server.name.clone(), path.to_string(), key.clone());
    let line = history + row as i64;
    match tokio::task::spawn_blocking(move || a.sent.add_end(&host, &path, &k, line, &text)).await.map_err(|e| e.to_string()).and_then(|r| r) {
        Ok(()) => app.pages.tell(&json!({"t": "endnew", "key": key}).to_string()),
        Err(e) => tracing::warn!(event = "answer.unsaved", error = %e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(r: &[&str]) -> Vec<String> {
        r.iter().map(|s| s.to_string()).collect()
    }
    const RULE: &str = "────────────────────────────────────────";

    #[test]
    fn the_last_row_of_the_answer_above_the_input_box() {
        let claude = rows(&["❯ fix the tests", "", "⏺ Fixed both tests; the retry loop resets now.", "  All 42 pass.", "",
            "✻ Worked for 2m 12s · 1 shell still running", "", RULE, "❯ ", RULE, "  ? for shortcuts"]);
        assert_eq!(answer_row(&claude), Some(3), "not the turn line, the rules or the user's earlier prompt");
        let codex = rows(&["› explain this", "", "• Created essay.md and verified it contains 400 words.", "", "  Worked for 43s • 11:04",
            "", "› Ask Codex to do anything", "", "  GPT-6.1-Sol high · ~/p", "  ? for shortcuts"]);
        assert_eq!(answer_row(&codex), Some(2), "Codex 0.162 as measured");
        let fence = rows(&["⏺ Here it is:", "  ```", "  ok", "  ```", "", RULE, "❯ typing a new one", RULE]);
        assert_eq!(answer_row(&fence), Some(0), "rows too short to be found again are passed over; typing in the box changes nothing");
        assert_eq!(answer_row(&rows(&["$ ls", "a.txt", "$ "])), None, "a shell has no input box");
        let asking = rows(&["⏺ Ready to release.", "", "│ May I push main, then run `make release`?", "❯ 1. Yes, v0.14.0", "  2. Commit only"]);
        assert_eq!(answer_row(&asking), None, "a question's options are no input box");
        let codex_menu = rows(&["• Would you like to run it?", "› 1. Yes, proceed", "  2. No"]);
        assert_eq!(answer_row(&codex_menu), None, "nor Codex's menu");
    }

    #[test]
    fn turn_lines_of_both_agents() {
        for l in ["✻ Worked for 2m 12s", "✶ Cooked for 9s · 1 shell", "  Worked for 43s • 11:04", "* Baked for 1m"] {
            assert!(agentstate::turn_line(l), "{l}");
        }
        for l in ["⏺ Waited for the build, then ran it", "  for 3 days", "✻ Baking… (esc to interrupt)", "Worked for the client"] {
            assert!(!agentstate::turn_line(l), "{l}");
        }
    }
}
