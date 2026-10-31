// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A bookmark's text in the page (AC-57): matched by its letters and digits only, as the bridge
// matches it (bookmarks.py: the same cases in bridge/tests/loose_cases.json), so a wrapped
// row, a re-joined line and rendered Markdown all hold it; and the marks' places on the
// terminal's edge.

const KEEP = /[\p{L}\p{N}]/u;

/** Only letters and digits, lowercase. The bridge's offsets into it count code points: slice
 *  it with part(). */
export function loose(text) {
  return (text.replace(/[\ufe0e\ufe0f]/g, "").toLowerCase().match(/[\p{L}\p{N}]/gu) ?? []).join("");
}

/** loose(text)'s code points start..start+size, as the bridge counts them. */
export function part(text, start, size) { return [...loose(text)].slice(start, start + size).join(""); }

// strong, struck or emphasised words: the closing mark followed by a space, a stop or the end
const EMPHASIS = new RegExp(String.raw`(^|[\s(])(\*\*|__|~~|[*_])([^\s*_~]|[^\s*_~].*?[^\s*_~])\2(?=[\s),;:!?]|\.(?:\s|$)|$)`, "gm");

/** `text` as one line of words for a list's row (AC-33): every line break and run of space one
 *  space, the terminal's frame lines (box drawing, ⎿) gone and, from an agent's Markdown, its marks
 *  (fences, a heading's, a quote's and a list's marks, emphasis, code, a table's bars, a link's
 *  address); `max` characters at most (the row cuts it with …). */
export function oneLine(text, markdown = false, max = 240) {
  let t = (text ?? "").slice(0, max * 8);                                  // a long record's start is enough
  if (markdown) t = t
    .replace(/^[ \t]*(```|~~~).*$/gm, " ")                                  // a fence's lines
    .replace(/^[ \t]*\|?[ \t:]*-{3,}[-| \t:]*$/gm, " ")                    // a table's rule, a horizontal rule
    .replace(/^[ \t]*(?:(?:#{1,6}|[-*+]|\d+[.)]|\[[ xX]\])[ \t]+|>[ \t]?)+/gm, "")   // heading, list, task, quote marks
    .replace(/^[ \t]*\|(.*)\|[ \t]*$/gm, (_, row) => row.replace(/[ \t]*\|[ \t]*/g, " "))   // a table's row: its cells
    .replace(/!?\[([^\]\n]*)\]\([^)\n]*\)/g, "$1")                         // a link or a picture: its words
    .replace(/`([^`\n]+)`/g, "$1")                                         // code
    .replace(EMPHASIS, "$1$3")                                               // strong, struck, emphasis around words
    .replace(/\\([\\`*_{}[\]()#+\-.!>|~])/g, "$1");                          // escapes
  t = t.replace(/[─-╿⎿]/g, " ").replace(/\s+/g, " ").trim();
  const cps = [...t];
  return cps.length > max ? cps.slice(0, max).join("") : t;
}

/** A record's title on one line: its note without Markdown marks, else its text's start. The bar,
 *  the lists, the rail's tips and the navigator all show this. */
export const titleOf = (item, max = 120) => oneLine(item.note, true, 200) || oneLine(item.text, !!item.by, max);

/** The text nodes under `root` (an element or a shadow root's part), in order. */
export function textNodes(root) {
  const out = [];
  if (!root) return out;
  const w = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  while (w.nextNode()) out.push(w.currentNode);
  return out;
}

/** Where `needle` (loose already) starts and ends in the text of `nodes`:
 *  [[node, offset], [node, offset after]], or null. */
export function spanOf(nodes, needle) {
  if (!needle) return null;
  const at = [];                                   // per letter kept: [node, offset, its length]
  let text = "";
  for (const node of nodes) {
    let i = 0;
    for (const ch of node.data) {                  // by code point, as loose() reads
      for (const c of ch.toLowerCase()) if (KEEP.test(c)) { text += c; at.push([node, i, ch.length]); }
      i += ch.length;
    }
  }
  const i = text.indexOf(needle);
  if (i < 0) return null;
  const k = [...text.slice(0, i)].length;          // at counts code points, indexOf UTF-16 units
  const [endNode, endOff, len] = at[k + [...needle].length - 1] ?? [];
  return endNode ? [[at[k][0], at[k][1]], [endNode, endOff + len]] : null;
}

/** A Range over needle in nodes, or null. */
export function rangeOf(nodes, needle) {
  const span = spanOf(nodes, needle);
  if (!span) return null;
  const r = document.createRange();
  r.setStart(...span[0]);
  r.setEnd(...span[1]);
  return r;
}

/** A Range over the skip+1-th occurrence of query (as typed, case aside) in the text of
 *  nodes, or null (a search, AC-58). */
export function rangeOfText(nodes, query, skip = 0) {
  if (!query) return null;
  const text = nodes.map((node) => node.data).join("");
  const low = text.toLowerCase(), q = query.toLowerCase();
  let i = -1;
  for (let k = 0; k <= skip; k++) { i = low.indexOf(q, i + 1); if (i < 0) return null; }
  const r = document.createRange();
  let folded = 0, started = false;
  for (const node of nodes) {
    let offset = 0;
    for (const ch of node.data) {
      // One original character can lowercase into several UTF-16 units (İ → i + dot).
      folded += ch.toLowerCase().length;
      if (!started && folded > i) { r.setStart(node, offset); started = true; }
      offset += ch.length;
      if (folded >= i + q.length) { r.setEnd(node, offset); return r; }
    }
  }
  return null;
}

/** Where the marks stand: a column on the terminal's right edge, left of a scroll bar
 *  (clientWidth ends there), over its height; "left", as in the wide layout the terminal's
 *  column, not #main, places it. {css, h}, or null while the terminal is not shown. */
export function railBox(term) {
  if (term.hidden || !term.clientHeight) return null;
  const h = term.clientHeight - 16;
  return { h, css: `top:${term.offsetTop + 8}px;height:${h}px;left:${term.offsetLeft + term.clientLeft + term.clientWidth - 18}px` };
}

/** The marks on the terminal's edge: a bookmark's line placed between the oldest line the
 *  pane can load and its last one, over `height` px (the rail is the whole loadable history:
 *  marks move up as it grows); one whose line left the history (tmux's limit) is not drawn;
 *  marks closer than `gap` px are one. items: [{n}]; returns [{top, items}], top to bottom. */
export function railSlots(items, oldest, end, height, gap = 8) {
  const span = Math.max(1, end - oldest);
  const placed = items.filter((it) => it.n >= oldest && it.n <= end).map((it) => ({ it, top: Math.round(((it.n - oldest) / span) * height) }))
    .sort((a, b) => a.top - b.top);
  const out = [];
  for (const p of placed) {
    const last = out.at(-1);
    if (last && p.top - last.top < gap) last.items.push(p.it);
    else out.push({ top: p.top, items: [p.it] });
  }
  return out;
}
