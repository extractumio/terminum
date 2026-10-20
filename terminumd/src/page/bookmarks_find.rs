// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A bookmark found again by its text (AC-33), nearest the line it was on: tmux numbers lines
//! anew when it drops them and when the scrollback is cleared. Pure: reading the rows from tmux
//! is the stream's.

use serde_json::{json, Value};

use super::bookmarks::{GONE, NEAR, PARTS, PRINT};

/// Where a bookmark's text is: the line where its letters and digits [start..start+size] begin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub n: i64,
    pub start: usize,
    pub size: usize,
}

/// Only letters and digits, lowercase: the same in a wrapped row, a re-joined line and
/// rendered Markdown. The page's marktext.js has the same rule.
pub fn loose(text: &str) -> String {
    text.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// The rows' loose text joined without breaks, with each row's first byte and character.
struct Joined {
    text: String,
    bytes: Vec<usize>,
    chars: Vec<usize>,
}

impl Joined {
    /// The rows' loose text, joined.
    fn of<S: AsRef<str>>(rows: &[(i64, S)]) -> Joined {
        let mut j = Joined { text: String::new(), bytes: Vec::new(), chars: Vec::new() };
        let mut count = 0;
        for (_, t) in rows {
            let s = loose(t.as_ref());
            j.bytes.push(j.text.len());
            j.chars.push(count);
            count += s.chars().count();
            j.text.push_str(&s);
        }
        j
    }

    fn row_at(&self, byte: usize) -> usize {
        self.bytes.partition_point(|&s| s <= byte).max(1) - 1
    }

    fn char_at(&self, byte: usize) -> usize {
        let r = self.row_at(byte);
        self.chars[r] + self.text[self.bytes[r]..byte].chars().count()
    }

    /// The needle nearest row `k`'s start, as (character, byte): the last one before it and the
    /// first one from it are the only candidates (two searches, however often it is there).
    fn nearest(&self, needle: &str, k: usize) -> Option<(usize, usize)> {
        let (at_b, at_c) = (self.bytes.get(k).copied().unwrap_or(0), self.chars.get(k).copied().unwrap_or(0));
        let end = self.text.floor_char_boundary((at_b + needle.len() - 1).min(self.text.len()));
        let before = self.text[..end].rfind(needle).map(|b| (self.char_at(b), b));
        let after = self.text[at_b..].find(needle).map(|b| (self.char_at(at_b + b), at_b + b));
        match (before, after) {
            (None, None) => None,
            (None, Some(a)) => Some(a),
            (Some(b), Some(a)) if a.0 - at_c < at_c - b.0 => Some(a),
            (Some(b), _) => Some(b),
        }
    }
}

/// Where `text` is in rows (line, row text): the place nearest `hint` when it is there more
/// than once; None when it is not there. Rows are joined without breaks, so wrapping does not
/// matter. The whole print wins wherever it is; else the part found nearest `hint`, as a
/// selection in rendered Markdown lacks what the source has (a link's address, a table's bars).
pub fn find<S: AsRef<str>>(rows: &[(i64, S)], text: &str, hint: i64) -> Option<Found> {
    let whole: Vec<char> = loose(text).chars().take(PRINT).collect();
    if whole.is_empty() {
        return rows.iter().any(|(n, _)| *n == hint).then_some(Found { n: hint, start: 0, size: 0 });
    }
    let j = Joined::of(rows);
    let k = hint_row(rows, hint);
    let at = j.chars.get(k).copied().unwrap_or(0);
    let found = |b: usize, start, size| Some(Found { n: rows[j.row_at(b)].0, start, size });
    if let Some((_, b)) = j.nearest(&whole.iter().collect::<String>(), k) {
        return found(b, 0, whole.len());
    }
    let mut best: Option<((usize, usize), usize, usize)> = None;
    for size in PARTS.into_iter().filter(|&s| s < whole.len()) {
        let mut starts: Vec<usize> = (0..=whole.len() - size).step_by(size).collect();
        if !starts.contains(&(whole.len() - size)) {
            starts.push(whole.len() - size);
        }
        for start in starts {
            let part: String = whole[start..start + size].iter().collect();
            if let Some(i) = j.nearest(&part, k)
                && best.is_none_or(|(b, _, _)| i.0.abs_diff(at) < b.0.abs_diff(at))
            {
                best = Some((i, start, size));
            }
        }
        if best.is_some() {
            break;
        }
    }
    best.and_then(|((_, b), start, size)| found(b, start, size))
}

/// The row of line `hint` (the last row when it is past them).
fn hint_row<S>(rows: &[(i64, S)], hint: i64) -> usize {
    rows.iter().position(|(n, _)| *n >= hint).unwrap_or(rows.len().saturating_sub(1))
}

/// The line where each of `texts` (text, its line's hint: none is the end) begins in rows, its
/// whole print nearest the hint; None when it is not there or too short to tell (fewer than
/// FEWEST letters and digits). Rows are joined once for all of them (the sent history's strip,
/// AC-90).
pub fn find_each<S: AsRef<str>>(rows: &[(i64, S)], texts: &[(String, Option<i64>)]) -> Vec<Option<i64>> {
    const FEWEST: usize = 4;
    if rows.is_empty() {
        return vec![None; texts.len()];
    }
    let j = Joined::of(rows);
    texts.iter().map(|(text, hint)| {
        let whole: String = loose(text).chars().take(PRINT).collect();
        if whole.chars().count() < FEWEST {
            return None;
        }
        let (_, b) = j.nearest(&whole, hint.map_or(rows.len() - 1, |h| hint_row(rows, h)))?;
        Some(rows[j.row_at(b)].0)
    }).collect()
}

/// Lines first..end (end exclusive) read first: NEAR either side of a bookmark's line.
pub fn near(first: i64, end: i64, around: i64) -> (i64, i64) {
    (first.max(around - NEAR), end.min(around + NEAR))
}

/// Whether the whole history is to be read after the lines near the bookmark's: its whole
/// print was not found there.
pub fn needs_whole(found: Option<&Found>, text: &str) -> bool {
    found.is_none_or(|f| f.size < loose(text).chars().take(PRINT).count())
}

/// {t: located, id, sid, n, start, size}, or {.., n: null, why: GONE} when it was not found.
pub fn located(id: &str, sid: &str, found: Option<&Found>) -> Value {
    match found {
        Some(f) => json!({"t": "located", "id": id, "sid": sid, "n": f.n, "start": f.start, "size": f.size}),
        None => json!({"t": "located", "id": id, "sid": sid, "n": null, "why": GONE}),
    }
}

/// {t: located, id, n: null, why}: DELETED, ELSEWHERE or "The history could not be read: …".
pub fn unlocated(id: &str, why: &str) -> Value {
    json!({"t": "located", "id": id, "n": null, "why": why})
}

