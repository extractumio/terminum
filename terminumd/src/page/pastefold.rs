// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A paste Claude Code folds into "[Pasted text #1 +6 lines]" is pasted once more, which unfolds it,
//! so the input shows its text (AC-03); only where its box with the text stays inside the pane, or
//! Claude Code runs full screen and scrolls its own box: on the normal screen a box taller than the
//! pane breaks its redraw, and its top is out of reach of the cursor and of the eye (measured in
//! 2.1.322: ↑ held stops at the pane's first row, the text's first lines never drawn again).

use unicode_width::UnicodeWidthStr;

use crate::screen::frame::{Fields, FIELDS};
use crate::tmux::agentstate::is_rule;
use crate::tmux::control::Control;
use crate::tmux::server::Wake;

/// Claude Code's words under a paste it folded; the same paste again unfolds it (read in its
/// program, 2.1.318).
const FOLDED: &str = "paste again to expand";
/// How long its words are waited for: keys typed next wait behind them.
const FOLD_WAIT: std::time::Duration = std::time::Duration::from_millis(800);
/// Rows of the pane the unfolded text leaves above the box and its footer (as Ask's: ask.js SPARE).
const SPARE: usize = 2;
/// Columns of the box's lines that are not text: its prompt and margin (ask.js boxRows).
const MARGIN: usize = 4;

/// Whether Claude Code folds this paste: over 800 characters or two line breaks.
pub(super) fn folds(text: &str) -> bool {
    text.chars().nth(800).is_some() || lines(text).nth(3).is_some()
}

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split("\r\n").flat_map(|l| l.split(['\n', '\r']))
}

/// Rows `text` takes in a box `cols` wide, by the cells its characters take (a tab 4, as ask.js
/// counts); the count stops once past `most` (a paste may be 1 MB).
fn box_rows(text: &str, cols: usize, most: usize) -> usize {
    let width = cols.saturating_sub(MARGIN).max(1);
    let mut n = 0;
    for l in lines(text) {
        let cells = if l.contains('\t') { l.replace('\t', "    ").width() } else { l.width() };
        n += cells.div_ceil(width).max(1);
        if n > most {
            break;
        }
    }
    n
}

/// Whether `text` unfolded fits a pane `cols` × `rows` whose screen `shown` holds the folded box:
/// what Claude Code redraws with it, from the rows written right above the box's top rule (a busy
/// agent's spinner and its list) to the last row written below (the bottom rule, the footer), with
/// the folded prompt's row taken by the text. No folded prompt or no top rule in sight (the box
/// already taller than the pane): no.
pub(super) fn fits(text: &str, shown: &[String], cols: usize, rows: usize) -> bool {
    let blank = |r: &String| r.trim().is_empty();
    let Some(prompt) = shown.iter().rposition(|r| r.contains("[Pasted text #")) else { return false };
    let Some(rule) = shown[..prompt].iter().rposition(|r| is_rule(r)) else { return false };
    let top = shown[..rule].iter().rposition(blank).map_or(0, |b| b + 1);
    let end = shown.len() - shown.iter().rev().take_while(|r| blank(r)).count();   // past the last row written
    let room = rows.saturating_sub(end - top - 1 + SPARE);
    box_rows(text, cols, room) <= room
}

/// Whether a paste Claude Code folded is to be pasted once more: its words came (none: a paste it
/// left as it is would be doubled, and another version may not say them) and the text fits (`fits`)
/// or Claude Code runs full screen; the pane not measured: no. `wake`: from before the paste, so
/// its words' output is not missed; the screen is read only when the pane printed.
pub(super) async fn unfolds(client: &Control, pane: &str, text: &str, mut wake: Wake) -> bool {
    let until = tokio::time::Instant::now() + FOLD_WAIT;
    while tokio::time::timeout_at(until, wake.notified()).await.is_ok() {
        let cmds = vec![format!("display -p -t {pane} '{FIELDS}'"), format!("capture-pane -p -t {pane}")];
        let Ok(answers) = client.run_all(cmds).await.into_iter().collect::<Result<Vec<_>, _>>() else { continue };
        let [size, shown] = &answers[..] else { continue };
        if shown.iter().any(|r| r.contains(FOLDED)) {
            let size = size.first().and_then(|l| Fields::parse(l));
            return size.is_some_and(|f| f.alt || fits(text, shown, f.width, f.height));
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_codes_folding_rule() {
        assert!(!folds("one\ntwo\nthree"), "two breaks stay");
        assert!(folds("one\ntwo\nthree\nfour"));
        assert!(folds("one\r\ntwo\r\nthree\r\nfour"));
        assert!(!folds("one\r\ntwo\r\nthree"), "a CRLF is one break");
        assert!(!folds(&"я".repeat(800)), "characters, not bytes");
        assert!(folds(&"я".repeat(801)));
    }

    /// A 30-row screen as Claude Code 2.1.322 draws a folded paste: output, the box at rows 15–17, a footer.
    fn screen() -> Vec<String> {
        let mut s: Vec<String> = (0..14).map(|i| format!("output {i}")).collect();
        s.push(String::new());
        s.push("─".repeat(100));
        s.push("❯ [Pasted text #1 +120 lines]".into());
        s.push("─".repeat(100));
        s.push("  paste again to expand".into());
        s.resize(30, String::new());
        s
    }

    /// `n` short lines as the page pastes them (a line ends in CR).
    fn text(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\r")
    }

    #[test]
    fn ac03_an_unfolded_paste_taller_than_the_pane_stays_folded() {
        assert!(!fits(&text(120), &screen(), 100, 30));
    }

    #[test]
    fn ac03_a_paste_that_fits_is_unfolded() {
        // the box from its top rule to the footer: 4 rows, the prompt's taken by the text; 25 text rows + 3 + 2 spare = 30
        assert!(fits(&text(25), &screen(), 100, 30));
        assert!(!fits(&text(26), &screen(), 100, 30), "one row more is over");
    }

    #[test]
    fn ac03_lines_count_as_the_rows_they_wrap_to() {
        let all = usize::MAX;
        assert_eq!(box_rows(&"a".repeat(96), 100, all), 1);
        assert_eq!(box_rows(&"a".repeat(97), 100, all), 2);
        assert_eq!(box_rows(&"字".repeat(48), 100, all), 1, "two cells each");
        assert_eq!(box_rows(&"字".repeat(49), 100, all), 2);
        assert_eq!(box_rows(&format!("\t{}", "a".repeat(92)), 100, all), 1, "a tab four cells");
        assert_eq!(box_rows(&format!("\t{}", "a".repeat(93)), 100, all), 2);
        assert_eq!(box_rows("a\r\rb", 100, all), 3, "a blank line is a row");
        assert_eq!(box_rows("x", 3, all), 1, "a pane narrower than the margin still counts");
        assert_eq!(box_rows(&text(100_000), 100, 30), 31, "the count stops past the most asked");
    }

    #[test]
    fn ac03_rows_above_the_placeholder_and_a_busy_agents_spinner_count() {
        // an earlier paste and a typed line above the folded one: two more rows in the box
        let mut two = screen();
        two.splice(16..16, ["❯ [Pasted text #1 +40 lines]".to_string(), "  and a typed line".to_string()]);
        two.truncate(30);
        assert!(!fits(&text(25), &two, 100, 30));
        assert!(fits(&text(23), &two, 100, 30));
        // a busy agent's spinner and its list right above the box are redrawn with it:
        // rows 12–18 less the prompt's: 6; 22 text rows + 6 + 2 spare = 30
        let mut busy = screen();
        busy[11] = String::new();
        busy[12] = "✻ Working… (3s)".into();
        busy[13] = "  ⎿  ☐ a step".into();
        busy[14] = "  ☐ another".into();
        assert!(fits(&text(22), &busy, 100, 30));
        assert!(!fits(&text(23), &busy, 100, 30));
    }

    #[test]
    fn ac03_no_folded_prompt_or_no_top_rule_in_sight_is_not_measured_as_fitting() {
        let mut no_rule = screen();
        no_rule[15] = "  typed text above, its rule scrolled off".into();
        assert!(!fits(&text(4), &no_rule, 100, 30));
        let shown: Vec<String> = vec!["  paste again to expand".into()];
        assert!(!fits(&text(4), &shown, 100, 30));
    }
}
