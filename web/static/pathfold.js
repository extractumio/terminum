// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Long file paths in a row of output shown short (AC-35): its start and its end, "[...]" between;
// a click on "[...]" shows it whole, and so it stays while the page is open. The row's text stays
// as it is (the middle is hidden by style, the marker's "[...]" is generated), so copying, a
// click on the path, its underline and the cursor's maths see the whole path. Shown whole where
// text is edited or laid out as a grid (an agent's input box and footer, a full-screen program:
// style.css).
import { tokens } from "./links.js";
import { joinedText } from "./rowtext.js";

const HEAD = 36, TAIL = 40;     // characters kept at each end
const LONGER = HEAD + TAIL + 8; // a path shorter than this is never folded

const opened = new Set();       // paths shown whole

/** Folds the long paths of row `el` (render.js lineRec) that lie within it; none in the
 *  cursor's row (the cursor's maths read its text as it is drawn). */
export function foldPaths(el) {
  if (el.querySelector(".cur")) return;
  const { text, pieces } = joinedText([el]);
  for (const tok of tokens(text).reverse()) {   // from the end: earlier offsets stay where they were
    if (tok.kind !== "path" || tok.value.length < LONGER || opened.has(tok.value)) continue;
    hide(pieces, tok.start + HEAD, tok.start + tok.value.length - TAIL, tok.value);
  }
}

// Wraps the text from offset a to b in hidden spans, the marker before the first.
function hide(pieces, a, b, value) {
  let marker = null;
  for (const { node, at } of pieces) {
    const from = Math.max(a, at) - at, to = Math.min(b, at + node.length) - at;
    if (from >= to) continue;
    let part = node;
    if (to < part.length) part.splitText(to);
    if (from > 0) part = part.splitText(from);
    const mid = document.createElement("span");
    mid.className = "pmid";
    mid.dataset.fold = value;
    part.replaceWith(mid);
    mid.append(part);
    if (!marker) {
      marker = document.createElement("span");
      marker.className = "pfold";
      marker.dataset.fold = value;
      marker.dataset.wg = "";     // the terminal's own clicks and taps leave it alone (widgetui.js inWidget)
      marker.setAttribute("role", "button");
      marker.tabIndex = 0;          // the keys open it too (bindFolds)
      marker.title = `Show the whole path: ${value}`;
      mid.before(marker);
    }
  }
}

/** Shows path `value` whole, everywhere in `root` it is folded, for as long as the page is open. */
function unfold(root, value) {
  opened.add(value);
  for (const el of root.querySelectorAll(".pfold, .pmid")) if (el.dataset.fold === value) el.classList.add("open");
}

/** A click on "[...]" in `term` shows its path whole, everywhere it is folded. */
export function bindFolds(term) {
  term.addEventListener("click", (e) => {
    const m = e.target.closest?.(".pfold");
    if (!m) return;
    e.preventDefault();
    unfold(term, m.dataset.fold);
  });
  term.addEventListener("keydown", (e) => {
    const m = e.target.closest?.(".pfold");
    if (!m || (e.key !== "Enter" && e.key !== " ")) return;
    e.preventDefault();
    e.stopPropagation();
    unfold(term, m.dataset.fold);
  });
}

/** A found text (find.js) that runs into a folded path's hidden middle: that path shown whole. */
export function unfoldAt(range) {
  const term = document.getElementById("term");
  for (const mid of term?.querySelectorAll(".pmid:not(.open)") ?? []) if (range.intersectsNode(mid)) unfold(term, mid.dataset.fold);
}
