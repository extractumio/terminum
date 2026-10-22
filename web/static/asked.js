// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Quotes asked about (AC-37): the text the user asked the agent about (Ask's Add to Chat or Send
// to Agent) stays marked in the terminal with a light blue wavy line down its left, beside its
// rows (as a reminder's quote is: reminders.css), where it was selected. terminumd keeps them per
// pane (`asked`, `askedmarks`), so every page of the pane shows them. Each is found by its text,
// spaces counted as one, at the occurrence nearest the line its selection began on, among the
// rows within a screen of the visible ones (linkmarks.js); its line stands in the terminal's
// left gutter (as a bookmark brought into view: markjump.js), the rows' DOM left as it is.
import { nearRows } from "./linkmarks.js";
import { continues, joinedText } from "./rowtext.js";

const NEAR_LINE = 30;               // lines from where it was selected an occurrence may stand
const MARKS = /[︎️]/;               // the text-style marks render.js adds: not the program's text

/** `text` with every run of white space as one space and the marks left out, and where each of
 *  its characters came from in `text`. */
export function squeeze(text) {
  let out = "", space = false;
  const at = [];
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (MARKS.test(c)) continue;
    if (/\s/.test(c)) { space = out.length > 0; continue; }
    if (space) { out += " "; at.push(i); space = false; }
    out += c;
    at.push(i);
  }
  return { out, at };
}

/** view: the TermView; send(msg) to terminumd; sid(): the shown pane. {seen(m), asked(n, text,
 *  line), redraw()}. */
export function askedMarks({ term, boxes, view, send, sid }) {
  let items = [], timer = 0;
  const lines = [];                 // the drawn lines, kept for the next look
  const later = (ms = 120) => { clearTimeout(timer); timer = setTimeout(mark, ms); };
  term.addEventListener("scroll", () => later(), { passive: true });
  addEventListener("resize", () => later());

  function mark() {
    clearTimeout(timer);
    if (!items.length) return draw([]);
    // the near rows' text, paragraph by paragraph, and the text node each part of it is in
    let text = "";
    const pieces = [];
    let para = [];
    const take = () => {
      const p = joinedText(para);
      for (const piece of p.pieces) pieces.push({ node: piece.node, at: text.length + piece.at });
      text += p.text + "\n";
      para = [];
    };
    for (const row of nearRows(term, boxes)) { para.push(row); if (!continues(row)) take(); }
    if (para.length) take();
    const { out, at } = squeeze(text);
    const point = (offset, end) => {         // a text offset as a node and an offset in it
      let a = 0, b = pieces.length;
      while (a < b) { const m = (a + b) >> 1; if (end ? pieces[m].at < offset : pieces[m].at <= offset) a = m + 1; else b = m; }
      const p = pieces[a - 1];
      return p ? { node: p.node, offset: Math.min(offset - p.at, p.node.length) } : null;
    };
    const ranges = [];
    for (const item of items) {
      const want = squeeze(String(item.text ?? "")).out;
      if (!want) continue;
      let best = null;
      for (let i = out.indexOf(want); i >= 0; i = out.indexOf(want, i + 1)) {
        const a = point(at[i], false), line = a && view.lineOf(a.node.parentElement?.closest(".ln"));
        const far = line === null || line === undefined ? Infinity : Math.abs(line - item.line);
        if (far <= NEAR_LINE && (!best || far < best.far)) best = { i, a, far };
      }
      const b = best && point(at[best.i + want.length - 1] + 1, true);
      if (!b) continue;
      const r = document.createRange();
      r.setStart(best.a.node, best.a.offset);
      r.setEnd(b.node, b.offset);
      ranges.push(r);
    }
    draw(ranges);
  }

  /** A wavy line down the left of each range's rows, in the terminal's gutter (it scrolls with them). */
  function draw(ranges) {
    const t = term.getBoundingClientRect();
    ranges.forEach((r, k) => {
      const rects = [...r.getClientRects()].filter((x) => x.height);
      const line = (lines[k] ??= Object.assign(document.createElement("div"), { className: "askline" }));
      if (!line.isConnected) term.append(line);
      if (!rects.length) return void (line.hidden = true);
      const top = Math.min(...rects.map((x) => x.top)), bottom = Math.max(...rects.map((x) => x.bottom));
      line.style.cssText = `top:${Math.round(top - t.top + term.scrollTop)}px;height:${Math.round(bottom - top)}px;left:${term.scrollLeft + 6}px`;
      line.hidden = false;
    });
    for (const line of lines.slice(ranges.length)) line.hidden = true;
  }

  return {
    /** terminumd's messages: the shown pane's quotes; another pane's page kept one; a pane shown anew. */
    seen(m) {
      if (m.t === "askedmarks" && m.sid === sid()) { items = Array.isArray(m.items) ? m.items : []; later(0); }
      if (m.t === "askednew" && m.key === sid()) send({ t: "askedmarks" });
      if (m.t === "hist" && m.mode === "reset") { items = []; later(0); send({ t: "askedmarks" }); }
    },
    /** A quote just asked about (Ask has its number): kept by terminumd, which answers with them all. */
    asked(n, text, line) { if (Number.isInteger(line)) send({ t: "asked", n, text, line }); },
    redraw: () => later(60),
  };
}
