// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Search of the shown pane's history (AC-34): the text the user typed, case aside, in every
//! line the page could load (the history it may load and the screen), not only the part
//! loaded. Rows tmux wrapped are one line; a match is told by the row it starts on and how many
//! matches on that row come before it, as the page finds it among the row's text. A port of
//! iterm-enhancer's search.py; reading the rows from tmux is the stream's.

use super::pystr::{py_rstrip, py_strip};
use serde_json::{Value, json};

/// Matches told at most (the count says when there are more).
pub const MAX_HITS: usize = 2000;
/// Characters of a search.
pub const MAX_QUERY: usize = 200;

#[cfg(test)]
/// Rows numbered from `first` out of lines in the page format ({"r", "e"}): (n, text, hard end).
pub fn rows_of(first: i64, lines: &[Value]) -> Vec<(i64, String, bool)> {
    lines.iter().enumerate().map(|(k, l)| (first + k as i64, super::pystr::text_of(l), l["e"].as_bool().unwrap_or(false))).collect()
}

/// (row, matches before it on that row) for each match of `query` (case aside) in rows (n,
/// text, hard end), at most MAX_HITS, and the whole count. A row's spaces at its end are left
/// out, as the page draws it; rows tmux wrapped are one line. Matches may overlap, as
/// Python's `str.find(q, i + 1)` found them; past the cap the count goes on without overlaps
/// (`str.count`).
pub fn find<S: AsRef<str>>(rows: &[(i64, S, bool)], query: &str) -> (Vec<(i64, usize)>, usize) {
    let mut scan = Scan { q: query.to_lowercase(), hits: Vec::new(), total: 0, per_row: Vec::new() };
    if scan.q.is_empty() {
        return (Vec::new(), 0);
    }
    let (mut parts, mut starts, mut size) = (String::new(), Vec::new(), 0usize);
    for (n, text, eol) in rows {
        let t = py_rstrip(text.as_ref());
        starts.push((*n, size));
        parts.push_str(t);
        size += t.chars().map(|c| c.to_lowercase().count()).sum::<usize>();
        if *eol {
            scan.line(&parts, &starts);
            (parts, size) = (String::new(), 0);
            starts.clear();
        }
    }
    if !starts.is_empty() {
        scan.line(&parts, &starts);
    }
    (scan.hits, scan.total)
}

struct Scan {
    q: String,
    hits: Vec<(i64, usize)>,
    total: usize,
    per_row: Vec<(i64, usize)>, // matches told per row so far
}

impl Scan {
    /// The matches in one line: its rows' text joined once (a line can be thousands of rows).
    /// starts: (n, the row's first character after lowercase expansion), matching `low`.
    fn line(&mut self, joined: &str, starts: &[(i64, usize)]) {
        let low = joined.to_lowercase();
        let (mut at_byte, mut at_char) = (0usize, 0usize); // where chars were counted up to
        let mut from = 0usize;
        while let Some(found) = low[from..].find(&self.q) {
            let i = from + found;
            if self.hits.len() >= MAX_HITS {
                // past the cap only the count goes on ("2000+")
                self.total += low[i..].matches(self.q.as_str()).count();
                return;
            }
            self.total += 1;
            at_char += low[at_byte..i].chars().count();
            at_byte = i;
            let k = starts.partition_point(|&(_, s)| s <= at_char).max(1) - 1;
            let n = starts[k].0;
            let before = match self.per_row.iter_mut().find(|(r, _)| *r == n) {
                Some((_, c)) => {
                    *c += 1;
                    *c - 1
                }
                None => {
                    self.per_row.push((n, 1));
                    0
                }
            };
            self.hits.push((n, before));
            from = i + low[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
}

/// The page's ref of a request: a string, cut to 40 characters (a number as its text).
pub fn ref_of(msg: &Value) -> String {
    match &msg["ref"] {
        Value::String(s) => s.chars().take(40).collect(),
        Value::Number(n) => n.to_string().chars().take(40).collect(),
        _ => String::new(),
    }
}

/// The query of a {t: find, ref, q} message, or why it is refused.
pub fn query_of(msg: &Value) -> Result<String, String> {
    match msg["q"].as_str() {
        Some(q) if !py_strip(q).is_empty() && q.chars().count() <= MAX_QUERY => Ok(q.to_string()),
        _ => Err(format!("Type 1 to {MAX_QUERY} characters to find.")),
    }
}

/// {t: found, ref, sid, q, hits: [[n, k]], total}.
pub fn found(r#ref: &str, sid: &str, q: &str, hits: &[(i64, usize)], total: usize) -> Value {
    json!({"t": "found", "ref": r#ref, "sid": sid, "q": q, "hits": hits, "total": total})
}

/// {t: found, ref, error}: "No session is shown.", the query refused, or
/// "The history could not be read: <why>".
pub fn refused(r#ref: &str, why: &str) -> Value {
    json!({"t": "found", "ref": r#ref, "error": why})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn lowercase_expansion_keeps_search_hits_on_their_original_wrapped_rows() {
        let rows = [(10, "İ", false), (11, "x x", false), (12, "z", true)];
        assert_eq!(find(&rows, "x"), (vec![(11, 0), (11, 1)], 2));
        assert_eq!(find(&rows, "İx"), (vec![(10, 0)], 1));
        assert_eq!(find(&rows, "xz"), (vec![(11, 0)], 1));
        assert_eq!(find(&[(1, "Ο", false), (2, "Σ", true)], "ος"), (vec![(1, 0)], 1));
    }

    #[test]
    fn matches_by_row_case_insensitive_and_across_a_wrap() {
        let rows = [(10, "Error: disk", true), (11, "all good", true), (12, "an err", false), (13, "or again, error", true)];
        assert_eq!(find(&rows, "ERROR"), (vec![(10, 0), (12, 0), (13, 0)], 3));
    }

    #[test]
    fn two_on_a_row_are_told_apart_and_the_count_goes_on_past_the_cap() {
        assert_eq!(find(&[(5, "ab ab ab", true)], "ab"), (vec![(5, 0), (5, 1), (5, 2)], 3));
        let rows: Vec<(i64, &str, bool)> = (0..MAX_HITS as i64 + 5).map(|n| (n, "x", true)).collect();
        let (hits, total) = find(&rows, "x");
        assert_eq!((hits.len(), total), (MAX_HITS, MAX_HITS + 5));
    }

    /// The same text as one wrapped line and as short lines takes about as long: a line joined
    /// again per row would take thousands of times longer. Timed against the short lines, side by
    /// side, the best of three, so a busy machine (the other tests) slows both alike.
    #[test]
    fn a_long_wrapped_line_is_quick() {
        let a = "a".repeat(200);
        let rows = |eol| -> Vec<(i64, &str, bool)> { (0..=11000).map(|n| (n, if n < 11000 { a.as_str() } else { "a" }, eol || n == 11000)).collect() };
        let (wrapped, short) = (rows(false), rows(true));
        let best = |rows: &[(i64, &str, bool)]| (0..3).map(|_| {
            let t = Instant::now();
            assert_eq!(find(rows, "a").1, 11000 * 200 + 1);
            t.elapsed()
        }).min().unwrap();
        let (hits, total) = find(&wrapped, "a");
        assert_eq!((hits.len(), total), (MAX_HITS, 11000 * 200 + 1));
        let (w, s) = (best(&wrapped), best(&short));
        assert!(w < s * 4, "wrapped {w:?}, short lines {s:?}");
    }

    #[test]
    fn overlaps_trailing_spaces_and_wide_text_count_as_python_did() {
        assert_eq!(find(&[(0, "aaa", true)], "aa"), (vec![(0, 0), (0, 1)], 2), "str.find(q, i + 1) overlaps");
        let rows = [(1, "漢字漢字  ", false), (2, "ab  ", false), (3, "cd", true)];
        assert_eq!(find(&rows, "b  c"), (vec![], 0), "a row's end spaces are not drawn");
        assert_eq!(find(&rows, "B"), (vec![(2, 0)], 1), "rows by characters, not bytes");
    }

    #[test]
    fn the_answer_and_a_refused_search() {
        let lines: Vec<Value> = ["one", "two needle", "three", "needle", "foo bar"]
            .iter()
            .map(|t| json!({"r": [[t, null, null, 0]], "e": true}))
            .collect();
        let rows = rows_of(0, &lines);
        let q = query_of(&json!({"t": "find", "ref": "r", "q": "Needle"})).unwrap();
        let (hits, total) = find(&rows, &q);
        let m = found(&ref_of(&json!({"ref": "r"})), "tmux:local:terminum:%1", &q, &hits, total);
        assert_eq!((m["hits"].clone(), m["total"].clone(), m["sid"].clone()), (json!([[1, 0], [3, 0]]), json!(2), json!("tmux:local:terminum:%1")));
        assert!(query_of(&json!({"q": "  "})).unwrap_err().contains("characters"));
        assert!(query_of(&json!({"q": "x".repeat(201)})).is_err());
        assert!(query_of(&json!({"q": 5})).is_err());
        assert_eq!(find(&rows, "foo bar").1, 1, "a cell the program skipped is a space, as the page draws it");
        assert_eq!(refused("e", "No session is shown."), json!({"t": "found", "ref": "e", "error": "No session is shown."}));
        assert_eq!(ref_of(&json!({"ref": "x".repeat(50)})).len(), 40);
    }
}
