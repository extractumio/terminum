// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Editing in a terminal program from a phone (AC-52): which screen cell a tap is on, where
// the cursor is, and the arrow keys that move one to the other. The character under a point is
// also what a click on a link reads (termlinks.js).
const VS15 = "︎";     // added after symbols for their text form: not a character of the terminal's

/** Characters of `row` before (node, offset), not counting added variation selectors. */
function charsBefore(row, node, offset) {
  const walk = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
  let n = 0;
  for (let t = walk.nextNode(); t; t = walk.nextNode()) {
    const text = t === node ? t.data.slice(0, offset) : t.data;
    n += text.length - text.split(VS15).length + 1;
    if (t === node) return n;
  }
  return n;
}

/** The text position at viewport point (x, y), seen through the keyboard field (it may lie
 *  over the cursor's row, invisible, for iOS's Paste). */
export function caretAt(x, y) {
  const kbd = document.getElementById("kbd");
  const was = kbd?.style.pointerEvents;
  if (kbd) kbd.style.pointerEvents = "none";
  try {
    if (document.caretPositionFromPoint) {
      const p = document.caretPositionFromPoint(x, y);
      return p && { node: p.offsetNode, offset: p.offset };
    }
    const r = document.caretRangeFromPoint?.(x, y);       // Safari
    return r && { node: r.startContainer, offset: r.startOffset };
  } finally {
    if (kbd) kbd.style.pointerEvents = was;
  }
}

/** The character at viewport point (x, y) of the terminal, or null (no row's text there):
 *  {row, node, offset} of the caret, and `before` 1 when the character is the one before it. */
export function pointAt(x, y) {
  const at = caretAt(x, y);
  if (!at || at.node.nodeType !== Node.TEXT_NODE) return null;
  const row = at.node.parentElement?.closest(".ln");
  if (!row) return null;
  // the caret falls between characters: the one under the pointer is the one it is on, or before it
  const r = document.createRange();
  r.setStart(at.node, at.offset);
  const box = at.offset < at.node.length ? (r.setEnd(at.node, at.offset + 1), r.getBoundingClientRect()) : null;
  return { row, node: at.node, offset: at.offset, before: !box || x < box.left ? 1 : 0 };
}

/** The screen's rows, in order: a paragraph's rows are in a block of their own (term.js). */
const rowsOf = (screen) => [...screen.querySelectorAll(".ln")];

/** {row, col} of the screen cell at viewport point (x, y), or null (scrollback, or no text). */
export function cellAt(screen, x, y) {
  const p = pointAt(x, y);
  if (!p || !screen.contains(p.row)) return null;
  return { row: rowsOf(screen).indexOf(p.row), col: Math.max(0, charsBefore(p.row, p.node, p.offset) - p.before) };
}

/** {row, col} of the cursor on the screen, or null. */
export function cursorCell(screen) {
  const cur = screen.querySelector(".cur");
  const row = cur?.closest(".ln");
  const first = cur && document.createTreeWalker(cur, NodeFilter.SHOW_TEXT).nextNode();
  if (!row || !first) return null;
  return { row: rowsOf(screen).indexOf(row), col: charsBefore(row, first, 0) };
}

const FAINT = 2, CURSOR = 256;

/** The column after the last character the user typed on a row ({r: runs} as render.js gets
 *  it): faint text is a program's suggestion (Claude Code shows its next prompt so, and → takes
 *  it), and the cursor's own cell is where the next character goes, so neither counts. */
export function typedEnd(data) {
  let col = 0, end = 0;
  for (const [text, , , flags] of data?.r ?? []) {
    const cells = Array.isArray(text) ? text : [...text];
    for (const c of cells) {
      col++;
      if (c !== "" && c.trim() && !(flags & (FAINT | CURSOR))) end = col;
    }
  }
  return end;
}

const cellsOf = (text) => (Array.isArray(text) ? text : [...text]);

/** The column of the cursor's cell on a row ({r: runs}), or -1. */
export function cursorCol(data) {
  let col = 0;
  for (const [text, , , flags] of data?.r ?? []) {
    if (flags & CURSOR) return col;
    col += cellsOf(text).length;
  }
  return -1;
}

/** The row with the cursor on cell `col` instead (null: none), as the bridge draws it (screen.py):
 *  its own run, padded out past the text; a wide character's two cells stay together. */
export function withCursor(data, col) {
  const runs = [];
  let x = 0;
  for (const [text, fg, bg, flags] of data.r) {
    const cells = cellsOf(text), f = flags & ~CURSOR;
    const part = (a, b, fl) => { if (b > a) runs.push([Array.isArray(text) ? cells.slice(a, b) : cells.slice(a, b).join(""), fg, bg, fl]); };
    let i = col == null ? -1 : col - x;
    if (i >= 0 && i < cells.length) {
      if (cells[i] === "" && i > 0) i--;                                   // the right half of a wide one
      const j = cells[i + 1] === "" ? i + 2 : i + 1;
      part(0, i, f); part(i, j, f | CURSOR); part(j, cells.length, f);
    } else part(0, cells.length, f);
    x += cells.length;
  }
  if (col != null && col >= x) {
    if (col > x) runs.push([" ".repeat(col - x), null, null, 0]);
    runs.push([" ", null, null, CURSOR]);
  }
  return { ...data, r: runs };
}

/** The ← or → keys from one cell to another on its row: "" when the move is not one to make (another
 *  row: ↑ and ↓ would recall a shell's history; a coding agent's input moves by its text, boxnav.js). */
export function arrowsTo(from, to, { appCursor = false } = {}) {
  if (!from || !to || to.row !== from.row) return "";
  const dx = to.col - from.col;
  if (Math.abs(dx) > 120) return "";
  return ((appCursor ? "\x1bO" : "\x1b[") + (dx < 0 ? "D" : "C")).repeat(Math.abs(dx));
}
