// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Rows of a pane from a `capture-pane -p -e -J` capture (AC-02, AS-03): every joined line is
//! cut back into rows of the pane's width the way tmux wraps, and a row is encoded in the
//! page's line format `{"r": runs, "e": hard_eol}` of iterm-enhancer's `screen.py`
//! (`enc_line`, `enc_cells`), which `web/static/render.js` draws.
//!
//! A cell is one character plus the zero-width characters after it (combining marks, a ZWJ
//! and the character it joins, variation selectors); its width comes from `unicode-width`,
//! except that a VS16 makes a narrow character wide, as tmux does by default
//! (`variation-selector-always-wide`).

use super::sgr::{self, Color, Style};
use serde_json::Value;
use std::sync::Arc;
use unicode_width::UnicodeWidthChar;

/// The cursor flag of a run.
pub const CURSOR: u16 = 256;
/// A blank cell with any of these flags still draws something: underline, strikethrough,
/// inverse, cursor.
const VISIBLE_WHEN_BLANK: u16 = sgr::UNDERLINE | sgr::STRIKE | sgr::INVERSE | CURSOR;
const ZWJ: char = '\u{200D}';
const VS16: char = '\u{FE0F}';

/// One column of a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// The base character (a space for the right half of a wide character).
    pub ch: char,
    /// The zero-width characters drawn with `ch`.
    pub extra: String,
    /// 1, 2 for the left half of a wide character, 0 for its right half.
    pub width: u8,
    pub style: Style,
    /// The OSC 8 hyperlink's address the cell is in (AC-35).
    pub link: Option<Arc<str>>,
}

impl Cell {
    fn is_plain_blank(&self) -> bool {
        self.width == 1 && self.ch == ' ' && self.extra.is_empty()
    }

    /// The cell's text in the page format: "" for the right half of a wide character.
    fn text(&self) -> String {
        if self.width == 0 {
            return String::new();
        }
        let mut s = String::with_capacity(self.ch.len_utf8() + self.extra.len());
        s.push(self.ch);
        s.push_str(&self.extra);
        s
    }
}

/// One row of the pane: its cells and whether a joined line ends here (`hard_eol`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub hard: bool,
}

/// The rows of a capture cut at `width` cells; every line of the capture (split on `\n`, a
/// final `\n` ends the last line) gives at least one row, its last one hard.
pub fn capture_rows(text: &str, width: usize) -> Vec<Row> {
    let width = width.max(1);
    let mut style = Style::default();
    let mut link = None;
    let mut rows = Vec::new();
    let mut cells = Vec::with_capacity(width);
    for line in sgr::lines(text) {
        line_cells(line, &mut style, &mut link, &mut cells);
        cut(&mut cells, width, &mut rows);
    }
    rows
}

/// Exactly `height` rows of a screen capture: missing rows are empty hard rows, rows past
/// `height` are dropped.
pub fn screen_rows(text: &str, width: usize, height: usize) -> Vec<Row> {
    let mut rows = capture_rows(text, width);
    rows.truncate(height);
    rows.resize(height, Row { cells: Vec::new(), hard: true });
    rows
}

/// The cells of one line, one per character with its zero-width followers, wide ones not
/// yet split into halves.
fn line_cells(line: &str, style: &mut Style, link: &mut Option<Arc<str>>, out: &mut Vec<Cell>) {
    let mut join = false; // the previous character was a ZWJ: this one belongs to its cell
    sgr::parse_line(line, style, link, |c, st, ln| {
        let c = if c.is_control() { ' ' } else { c };
        let w = c.width().unwrap_or(1).min(2) as u8;
        if let Some(last) = out.last_mut().filter(|_| join || w == 0) {
            last.extra.push(c);
            if c == VS16 && last.width == 1 {
                last.width = 2;
            }
            join = c == ZWJ;
            return;
        }
        join = c == ZWJ;
        out.push(Cell { ch: c, extra: String::new(), width: w.max(1), style: st, link: ln.cloned() });
    });
}

/// Cuts one line's cells into rows of `width` cells as tmux wraps: a wide character that
/// would straddle the right edge starts the next row (that row ends one cell short).
fn cut(cells: &mut Vec<Cell>, width: usize, rows: &mut Vec<Row>) {
    let mut row = Vec::with_capacity(width.min(cells.len() + cells.len() / 2));
    let mut col = 0;
    for cell in cells.drain(..) {
        let w = cell.width as usize;
        if col + w > width && col > 0 {
            rows.push(Row { cells: std::mem::take(&mut row), hard: false });
            col = 0;
        }
        col += w;
        let (style, link) = (cell.style, cell.link.clone());
        row.push(cell);
        if w == 2 {
            row.push(Cell { ch: ' ', extra: String::new(), width: 0, style, link });
        }
    }
    rows.push(Row { cells: row, hard: true });
}

fn color(c: Color) -> Value {
    match c {
        Color::Default => Value::Null,
        Color::Palette(n) => Value::from(n),
        Color::Rgb(r, g, b) => Value::String(format!("#{r:02x}{g:02x}{b:02x}")),
    }
}

/// Plain text when every cell is one character below U+2000 and none is wide; else the
/// cells as a list so the page pins each one to the grid.
fn cells_value(cells: &[Cell]) -> Value {
    if cells.iter().all(|c| c.width == 1 && c.extra.is_empty() && c.ch < '\u{2000}') {
        Value::String(cells.iter().map(|c| c.ch).collect())
    } else {
        Value::Array(cells.iter().map(|c| Value::String(c.text())).collect())
    }
}

fn run(text: Value, fg: Value, bg: Value, flags: u16, link: Option<usize>) -> Value {
    let mut r = vec![text, fg, bg, Value::from(flags)];
    r.extend(link.map(Value::from));
    Value::Array(r)
}

/// A row as `{"r": runs, "e": hard_eol}`; a run is `[text-or-cells, fg, bg, flags]`. When cells
/// are an OSC 8 hyperlink's (AC-35) the row also has `"h"`, its addresses each once, and such a run
/// a fifth element, the index of its address there (a long address over many colors is sent once).
/// Runs group cells of equal style and link; a hard row's trailing blank cells are dropped unless they draw
/// something (a row tmux wrapped keeps them: they are the line's own spaces, which the page needs
/// when it joins the rows again, AC-04); the cursor cell gets its own run with the cursor flag,
/// padded out past the text.
pub fn encode(row: &Row, cursor: Option<usize>) -> Value {
    let cells = &row.cells;
    let key = |i: usize| {
        let st = cells[i].style;
        (st.fg, st.bg, st.flags | if cursor == Some(i) { CURSOR } else { 0 }, cells[i].link.as_ref())
    };
    let mut n = cells.len();
    while n > 0 && row.hard {
        let (_, bg, flags, link) = key(n - 1);
        if !(cells[n - 1].is_plain_blank() && bg == Color::Default && flags & VISIBLE_WHEN_BLANK == 0 && link.is_none()) {
            break;
        }
        n -= 1;
    }
    let mut runs = Vec::new();
    let mut links: Vec<&Arc<str>> = Vec::new();
    let mut i = 0;
    while i < n {
        let k = key(i);
        let mut j = i + 1;
        while j < n && key(j) == k {
            j += 1;
        }
        let link = k.3.map(|l| {
            links.iter().position(|&h| h == l).unwrap_or_else(|| {
                links.push(l);
                links.len() - 1
            })
        });
        runs.push(run(cells_value(&cells[i..j]), color(k.0), color(k.1), k.2, link));
        i = j;
    }
    if let Some(x) = cursor.filter(|&x| x >= cells.len()) {
        if x > n {
            runs.push(run(Value::String(" ".repeat(x - n)), Value::Null, Value::Null, 0, None));
        }
        runs.push(run(Value::String(" ".into()), Value::Null, Value::Null, CURSOR, None));
    }
    let mut line = serde_json::Map::with_capacity(3);
    line.insert("r".into(), Value::Array(runs));
    line.insert("e".into(), Value::Bool(row.hard));
    if !links.is_empty() {
        line.insert("h".into(), links.iter().map(|l| Value::String(l.to_string())).collect());
    }
    Value::Object(line)
}

/// [`encode`] as compact JSON: the row's identity when frames are diffed.
pub fn encode_str(row: &Row, cursor: Option<usize>) -> String {
    encode(row, cursor).to_string()
}

/// The row's plain text, trailing spaces trimmed (agent state, folds, search, bookmarks).
pub fn row_text(row: &Row) -> String {
    let mut s = String::with_capacity(row.cells.len());
    for c in row.cells.iter().filter(|c| c.width != 0) {
        s.push(c.ch);
        s.push_str(&c.extra);
    }
    s.truncate(s.trim_end_matches(' ').len());
    s
}

#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;
