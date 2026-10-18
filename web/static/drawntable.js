// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Tables a program drew with rules (AC-31): Claude Code's and rich's ┌─┬─┐, tabulate's grids,
// mysql's +---+. Pure: outparse.js finds them, widgetui.js shows their cells as an HTML table,
// whose cells wrap at any width where the drawn rules break apart. A line that does not fit
// the grid makes it no table: it stays text. A box table whose top rule is not there (scrolled
// away, or cut off where it was copied) starts at its head row when a middle rule follows it.

// rules: a left end, runs of a horizontal line between junctions, a right end
const RULE = {
  top: ["┌┏╔╭┍┎╒╓", "┬┳╦┯┰╤╥", "┐┓╗╮┑┒╕╖"],
  mid: ["├┣╠┝┞┟┠┡┢╞╟", "┼╋╬┿╀╁╂╃╄╅╆╇╈╉╊╪╫", "┤┫╣┥┦┧┨┩┪╡╢"],
  end: ["└┗╚╰┕┖╘╙", "┴┻╩┷┸╧╨", "┘┛╝╯┙┚╛╜"],
};
const LINE = "─━═┄┅┈┉╌╍";
const rule = ([l, j, r]) => new RegExp(`^[${l}][${LINE}]+(?:[${j}][${LINE}]+)*[${r}]$`);
const BOX = { top: rule(RULE.top), mid: rule(RULE.mid), end: rule(RULE.end), bar: /[│┃║]/ };
const ASCII = /^\+[-=]+(?:\+[-=]+)*\+$/;     // mysql, prettytable, tabulate's "grid": every rule alike
// before a table's top rule: indentation, and a coding agent's message or tool mark
const LEAD = /^\s*(?:[⏺●⎿]\s*)?/;
const NUMBER = /^[-+]?(?:\d[\d,]*(?:\.\d+)?|\.\d+)\s*(?:%|[kKMGT]?i?B|[kKMGT]|ms|s)?$/;

const JUNCTION = /[┬┳╦┯┰╤╥┼╋╬┿╀╁╂╃╄╅╆╇╈╉╊╪╫┴┻╩┷┸╧╨]/g;
// the columns a rule draws
const width = (t) => (t[0] === "+" ? t.split("+").length - 2 : (t.match(JUNCTION) ?? []).length + 1);

// a box row: a bar at each end, n columns between
const boxRow = (t) => /^[│┃║].*[│┃║]$/.test(t);
const columns = (t) => t.split(BOX.bar).length - 2;
/** Whether line `text` is a head row with no top rule: a box row the middle rule `next` follows. */
const headRow = (text, next) => {
  const t = text.replace(LEAD, "").trimEnd(), r = next?.trim() ?? "";
  return boxRow(t) && BOX.mid.test(r) && columns(t) === width(r) && width(r) >= 2;
};

/** Whether a line starts a drawn table of two columns or more: its top rule, or a head row
 *  (without a top rule) when `next`, the line after it, is a middle rule. */
export const tableTop = (text, next) => {
  const t = text.replace(LEAD, "").trimEnd();
  return ((BOX.top.test(t) || ASCII.test(t)) && width(t) >= 2) || headRow(text, next);
};

/** The table whose top rule is line i of `lines` ([{text}]): {to (its bottom rule's line), head
 *  (cells, or null), rows (cells), align ("right" for a column of numbers, else "")}; null when
 *  its bottom rule is not printed yet; undefined when it is no table. A row's cell wrapped over
 *  lines is one cell again: between rules each part is a row, and without rules between the
 *  rows a line whose first cell is empty goes on the row above (box tables only: mysql and its
 *  kind never wrap a cell, and an empty first value is a row of its own). */
export function drawnTable(lines, i, max) {
  const headless = headRow(lines[i].text, lines[i + 1]?.text);
  if (!headless && !tableTop(lines[i].text)) return undefined;
  const top = headless ? lines[i + 1].text.trim() : lines[i].text.replace(LEAD, "").trimEnd();
  const ascii = top[0] === "+", n = width(top);
  const row = (t) => (ascii ? /^\|.*\|$/.test(t) : boxRow(t));
  // an ASCII table's rules look alike: the one no row follows is its last
  const isRule = (t, kind, next) => width(t) === n
    && (ascii ? ASCII.test(t) && (kind === "end") !== row(next ?? "") : BOX[kind].test(t));
  const parts = [[]];
  for (let j = headless ? i : i + 1; j < lines.length && j - i < max; j++) {
    const t = (j === i ? lines[j].text.replace(LEAD, "") : lines[j].text).trim(), next = lines[j + 1]?.text.trim();
    if (isRule(t, "end", next) && parts.at(-1).length) return grid(parts, j, ascii);
    if (isRule(t, "mid", next)) { if (!parts.at(-1).length) return undefined; parts.push([]); continue; }
    const cells = ascii ? t.split("|") : t.split(BOX.bar);
    if (!row(t) || cells.length !== n + 2) return undefined;
    parts.at(-1).push(cells.slice(1, -1).map((c) => c.trim()));
  }
  return lines.length - i < max ? null : undefined;
}

// The parts between rules as a head and rows: the first part is the head when a rule follows it.
function grid(parts, to, ascii) {
  const join = (part) => part[0].map((_, c) => part.map((cells) => cells[c]).filter(Boolean).join(" "));
  const head = parts.length > 1 ? join(parts.shift()) : null;
  let rows;
  if (parts.length > 1) rows = parts.map(join);
  else {
    rows = [];
    for (const cells of parts[0]) {
      if (!ascii && rows.length && !cells[0]) rows.at(-1).forEach((v, c) => { if (cells[c]) rows.at(-1)[c] = v ? `${v} ${cells[c]}` : cells[c]; });
      else rows.push([...cells]);
    }
  }
  const align = (head ?? rows[0]).map((_, c) => {
    const vals = rows.map((r) => r[c]).filter((v) => v && !/^[-–—]$/.test(v));   // "-": no value
    return vals.length && vals.every((v) => NUMBER.test(v)) ? "right" : "";
  });
  return { to, head, rows, align };
}

/** The table as Markdown (what Copy gives: it pastes into a document, an issue or a chat). */
export function tableMarkdown({ head, rows, align }) {
  const line = (cells) => `| ${cells.map((c) => c.replace(/\\/g, "\\\\").replace(/\|/g, "\\|")).join(" | ")} |`;
  const first = head ?? rows[0].map(() => "");
  return [line(first), `| ${align.map((a) => (a ? "--:" : "---")).join(" | ")} |`, ...rows.map(line)].join("\n");
}
