// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Absolute row numbers of a shown pane's history (AC-02, AS-04, AS-09, OQ-05). The page
//! numbers every row; tmux only says how many rows are above the screen (`history_size`).
//! While that grows, the screen's top is the count; once tmux drops the oldest rows at its
//! limit, a scroll is found by looking for the last frame's rows shifted up; a shrink (a
//! cleared scrollback) or a new width (tmux re-wrapped the history) starts the page anew.

/// What a frame means for the page's history.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// send the history anew ("hist" reset) and a full screen
    Reset,
    /// rows `from..to` (absolute) scrolled off the screen: send them ("hist" append)
    Append { from: i64, to: i64 },
    /// nothing scrolled
    Same,
}

#[derive(Default)]
pub struct Numbering {
    /// absolute number of the screen's first row; None before the first frame
    top: Option<i64>,
    /// tmux's history_size at the last frame
    history: i64,
    width: usize,
    /// hashes of the last frame's rows (plain text), to find a scroll at tmux's limit
    rows: Vec<u64>,
}

/// Rows are matched by their text; a shift needs this many equal rows that are not blank.
const MIN_MATCH: usize = 2;

impl Numbering {
    pub fn top(&self) -> Option<i64> {
        self.top
    }

    /// tmux's history_size at the last frame: what the screen's top was counted from.
    pub fn history(&self) -> i64 {
        self.history
    }

    /// The oldest row a page may load: what tmux still has, and at most `keep` rows back.
    pub fn oldest(&self, keep: i64) -> i64 {
        let top = self.top.unwrap_or(0);
        (top - self.history).max(top - keep)
    }

    /// Whether tmux has rows older than the page may load.
    pub fn truncated(&self, keep: i64) -> bool {
        self.oldest(keep) > self.top.unwrap_or(0) - self.history
    }

    /// Starts anew (a pane switch, a lost client): the next frame is a reset.
    pub fn forget(&mut self) {
        *self = Numbering::default();
    }

    /// The step for a frame with `history` rows above the screen, `width` columns and these
    /// screen rows (their plain text); `alt`: a full-screen program (its screen scrolls
    /// nothing into the history); `limit`: tmux's history-limit. At its limit tmux drops the
    /// oldest tenth at once, so the count falls by limit/10 while rows still scroll off.
    pub fn step(&mut self, history: i64, width: usize, rows: &[String], alt: bool, limit: i64) -> Step {
        let full = limit > 0 && history >= limit;
        let hashes: Vec<u64> = rows.iter().map(|r| hash(r)).collect();
        let step = match self.top {
            None => Step::Reset,
            Some(_) if width != self.width => Step::Reset,
            Some(top) => {
                let grown = history - self.history;
                let near = limit > 0 && (self.history + grown.max(0) >= limit || grown < 0);
                if near && !alt {
                    // tmux at its limit: before each row it adds, a full history loses its oldest
                    // tenth; the rows that scrolled off are the fewest that lead to `history`
                    let ks = scrolled(self.history, history, limit);
                    let n = rows.len();
                    // a candidate the screen's shift confirms, else the first the screen cannot show
                    let pick = ks.iter().copied().find(|&k| k > 0 && (k as usize) < n && shift_is(&self.rows, &hashes, rows, k as usize))
                        .or_else(|| ks.iter().copied().find(|&k| k as usize >= n))
                        .or_else(|| ks.first().copied());
                    match pick {
                        Some(0) if full => match shift(&self.rows, &hashes, rows) {
                            Some(k) if k > 0 => Step::Append { from: top, to: top + k as i64 },
                            _ => Step::Same,
                        },
                        Some(0) => Step::Same,
                        Some(k) if ks.first() == Some(&0) && !shift_is(&self.rows, &hashes, rows, k as usize) => Step::Same,
                        Some(k) => Step::Append { from: top, to: top + k },
                        None => Step::Reset,
                    }
                } else if grown > 0 {
                    Step::Append { from: top, to: top + grown }
                } else if grown < 0 {
                    Step::Reset
                } else if alt || !full {
                    Step::Same
                } else {
                    match shift(&self.rows, &hashes, rows) {
                        Some(k) if k > 0 => Step::Append { from: top, to: top + k as i64 },
                        _ => Step::Same,
                    }
                }
            }
        };
        // the rows tmux dropped at its limit stay counted: top = dropped + history
        let dropped = self.top.map_or(0, |t| t - self.history);
        self.top = Some(match step {
            Step::Append { to, .. } => to,
            _ => dropped + history.max(0),
        });
        self.history = history;
        self.width = width;
        self.rows = hashes;
        step
    }
}

/// How many rows the screen moved up: the smallest k (0 included) for which the old rows
/// from k on are the new rows from 0 on, with enough non-blank rows to be sure.
fn shift(old: &[u64], new: &[u64], text: &[String]) -> Option<usize> {
    let blank = hash("");
    let n = old.len().min(new.len());
    (0..n).find(|&k| {
        let overlap = n - k;
        let same = (0..overlap).all(|i| old[k + i] == new[i]);
        let solid = (0..overlap).filter(|&i| new[i] != blank && !text[i].trim().is_empty()).count();
        same && solid >= MIN_MATCH.min(overlap)
    })
}

/// The row counts that take tmux's history count from `from` to `to`, fewest first (tmux,
/// before adding a row to a history of `limit` rows or more, drops its oldest tenth, so they
/// differ by tenths); none for a clear.
fn scrolled(from: i64, to: i64, limit: i64) -> Vec<i64> {
    let trim = (limit / 10).max(1);
    let mut h = from;
    let mut out = Vec::new();
    for k in 0..=(trim * 4 + limit) {
        if h == to {
            out.push(k);
            if out.len() == 4 {
                break;
            }
        }
        if h >= limit {
            h -= trim;
        }
        h += 1;
    }
    out
}

/// Whether the old rows from `k` on are the new rows from 0 on, with enough non-blank ones.
fn shift_is(old: &[u64], new: &[u64], text: &[String], k: usize) -> bool {
    let n = old.len().min(new.len());
    k < n && (0..n - k).all(|i| old[k + i] == new[i]) && (0..n - k).filter(|&i| !text[i].trim().is_empty()).count() >= MIN_MATCH.min(n - k)
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.trim_end().hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn growing_history_appends_the_rows_that_scrolled_off() {
        let mut n = Numbering::default();
        assert_eq!(n.step(100, 80, &rows(&["a", "b", "c"]), false, 0), Step::Reset);
        assert_eq!(n.top(), Some(100));
        assert_eq!(n.step(102, 80, &rows(&["c", "d", "e"]), false, 0), Step::Append { from: 100, to: 102 });
        assert_eq!(n.top(), Some(102));
        assert_eq!(n.step(102, 80, &rows(&["c", "d", "f"]), false, 0), Step::Same, "a row redrawn in place");
    }

    #[test]
    fn a_cleared_scrollback_or_a_new_width_starts_anew() {
        let mut n = Numbering::default();
        n.step(500, 80, &rows(&["a"]), false, 0);
        assert_eq!(n.step(0, 80, &rows(&["$"]), false, 0), Step::Reset, "clear-history");
        assert_eq!(n.top(), Some(0), "a cleared scrollback reuses the numbers (AC-32)");
        n.step(10, 80, &rows(&["x"]), false, 0);
        assert_eq!(n.step(12, 60, &rows(&["x"]), false, 0), Step::Reset, "a resize re-wraps the history (AS-09)");
    }

    #[test]
    fn at_the_limit_a_scroll_is_found_by_the_rows() {
        let mut n = Numbering::default();
        n.step(2000, 80, &rows(&["l1", "l2", "l3", "l4"]), false, 2000);
        assert_eq!(n.step(1802, 80, &rows(&["l3", "l4", "l5", "l6"]), false, 2000), Step::Append { from: 2000, to: 2002 });
        assert_eq!(n.step(1802, 80, &rows(&["l3", "l4", "l5", "l7"]), false, 2000), Step::Same, "redrawn in place");
        assert_eq!(n.step(1802, 80, &rows(&["x", "y", "z", "w"]), false, 2000), Step::Same, "a redraw: no guess, no reset");
        assert_eq!(n.oldest(10_000), 2002 - 1802);
        assert_eq!(n.oldest(1000), 2002 - 1000);
        assert!(n.truncated(1000) && !n.truncated(10_000));
    }

    #[test]
    fn a_trimmed_tenth_at_the_limit_is_a_scroll_not_a_reset() {
        let mut n = Numbering::default();
        n.step(1999, 80, &rows(&["a", "b"]), false, 2000);
        assert_eq!(n.step(2000, 80, &rows(&["b", "c"]), false, 2000), Step::Append { from: 1999, to: 2000 });
        // two more rows scroll off: tmux drops 200 before the first, so 2000 - 200 + 2
        assert_eq!(n.step(1802, 80, &rows(&["d", "e"]), false, 2000), Step::Append { from: 2000, to: 2002 });
        assert_eq!(n.top(), Some(2002));
        assert_eq!(n.step(1803, 80, &rows(&["e", "f"]), false, 2000), Step::Append { from: 2002, to: 2003 });
        assert_eq!(n.step(0, 80, &rows(&["$"]), false, 2000), Step::Reset, "a cleared scrollback still resets");
        // 24 rows scroll off a history at 81 of 100: tmux drops 10 once, ending at 95; the
        // screen (6 rows) cannot show it, so the candidate past the screen is taken (14 or 24:
        // the first such is 14 here, the screen's rows decide when they can)
        let mut n = Numbering::default();
        n.step(81, 80, &rows(&["a", "b", "c", "d", "e", "f"]), false, 100);
        assert!(matches!(n.step(95, 80, &rows(&["u", "v", "w", "x", "y", "z"]), false, 100), Step::Append { from: 81, .. }));
        let mut n = Numbering::default();
        n.step(99, 80, &rows(&["l1", "l2", "l3", "l4", "l5", "l6"]), false, 100);
        assert_eq!(n.step(93, 80, &rows(&["l5", "l6", "l7", "l8", "l9", "l10"]), false, 100), Step::Append { from: 99, to: 103 }, "4 rows: 99, 100, trim to 90, 91, 92, 93");
    }

    #[test]
    fn a_full_screen_program_scrolls_nothing_into_the_history() {
        let mut n = Numbering::default();
        n.step(50, 80, &rows(&["a", "b", "c"]), true, 2000);
        assert_eq!(n.step(50, 80, &rows(&["b", "c", "d"]), true, 2000), Step::Same);
    }

    #[test]
    fn blank_rows_never_prove_a_scroll() {
        let mut n = Numbering::default();
        n.step(9, 80, &rows(&["", "", "", "x"]), false, 2000);
        assert_eq!(n.step(9, 80, &rows(&["", "", "x", ""]), false, 2000), Step::Same);
    }
}
