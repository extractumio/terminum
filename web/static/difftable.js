// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A Markdown table in a coding agent's diff (AC-31): Claude Code shows an edit of a Markdown file
// as numbered rows, "  7 -| Version | 0.11.1 |", "  7 +| Version | 0.12.0 |", "  8  | Date | … |".
// Two of its table rows in a row or more are drawn as a table: removed rows struck, added ones
// marked, the header from the row above a "|---|" when the diff shows it. Pure: lines are
// outparse.js's logical lines ({text}).

const ROW = /^\s*\d+\s*([+-])?\s*(\|.*\|)\s*$/;
const SEP = /^\|(\s*:?-{3,}:?\s*\|)+$/;

const cellsOf = (t) => t.slice(1, -1).split(/(?<!\\)\|/).map((c) => c.trim().replace(/\\\|/g, "|"));

/** The table of diff rows from line i: {to, head, rows, align, marks ("+", "-" or "" a row)}, or
 *  undefined when fewer than two of its rows are a table's. */
export function diffTable(lines, i, max = 2000) {
  const got = [];
  for (let j = i; j < lines.length && j - i < max; j++) {
    const m = ROW.exec(lines[j].text);
    if (!m) break;
    got.push({ mark: m[1] ?? "", text: m[2].trim() });
  }
  const sep = got.findIndex((r) => SEP.test(r.text));
  const body = got.filter((r, k) => k !== sep && !(sep > 0 && k === sep - 1));
  if (body.length < 2 && !(sep > 0 && body.length)) return undefined;
  const head = sep > 0 ? cellsOf(got[sep - 1].text) : null;
  const rows = body.map((r) => cellsOf(r.text)), n = Math.max(head?.length ?? 0, ...rows.map((r) => r.length));
  const pad = (r) => [...r, ...Array(n - r.length).fill("")];
  return { to: i + got.length - 1, head: head && pad(head), rows: rows.map(pad), align: Array(n).fill(false), marks: body.map((r) => r.mark) };
}
