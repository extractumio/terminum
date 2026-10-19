// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Python's `str` rules the folds were written with (fold.py's port, AC-32): code points,
//! Unicode whitespace; and a page line's text.

use serde_json::Value;

/// Python's `str.isspace()`: Unicode whitespace and the separators U+001C..U+001F.
pub fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.strip()`.
pub fn py_strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// Python's `str.rstrip()`.
pub fn py_rstrip(s: &str) -> &str {
    s.trim_end_matches(is_py_space)
}

/// Python's `\w` on `str`: a letter, a digit or "_".
pub fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Python's `\d` (decimal digits): ASCII and the common other scripts' digits.
pub fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
        || matches!(c as u32, 0x0660..=0x0669 | 0x06F0..=0x06F9 | 0x0966..=0x096F | 0x09E6..=0x09EF | 0x0E50..=0x0E59 | 0xFF10..=0xFF19)
}

pub fn chars(s: &str) -> usize {
    s.chars().count()
}

/// A line as the page gets it ({"r": runs, "e": hard}): its text.
pub fn text_of(line: &Value) -> String {
    let mut out = String::new();
    for run in line["r"].as_array().into_iter().flatten() {
        match &run[0] {
            Value::String(t) => out.push_str(t),
            Value::Array(cells) => cells.iter().filter_map(Value::as_str).for_each(|c| out.push_str(c)),
            _ => {}
        }
    }
    out
}
