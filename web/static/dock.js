// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Docked Files' width (AC-70): the band between the terminal and Files is a separator. Dragging it
// (mouse or touch) or ← → sets --side-w; the terminal follows at once and tmux once the drag rests
// (sizing.js watches the terminal's size and sends it SETTLE after the last change); a double click
// puts back the default. Kept in this browser for every session.

const $ = (id) => document.getElementById(id);
export const SIDE = { def: 380, min: 260, step: 16, maxShare: 0.6, termCols: 45 };

/** The width Files may take: at least SIDE.min, at most maxShare of the column beside the list,
 *  and never so wide the terminal (now `termPx` wide beside a Files `sidePx` wide) holds fewer than
 *  termCols columns of `cellPx`. Pure, for the tests. */
export function clampSide(px, { mainPx, sidePx, termPx, cellPx }) {
  const byTerm = termPx > 0 ? sidePx + termPx - SIDE.termCols * cellPx : Infinity;   // a hidden terminal: no bound
  const max = Math.max(SIDE.min, Math.min(mainPx * SIDE.maxShare, byTerm));
  return Math.round(Math.min(max, Math.max(SIDE.min, px)));
}

/** store: the page's per-browser store; cellWidth(): one column's px at the chosen text size;
 *  onResize(): the terminal's width changed. */
export function dockGrip({ store, cellWidth, onResize }) {
  const grip = $("dockgrip"), root = document.documentElement;
  const side = () => parseFloat(getComputedStyle(root).getPropertyValue("--side-w")) || SIDE.def;
  function termPx() {   // the terminal's text width: its box less its padding
    const t = $("term"), cs = getComputedStyle(t);
    return t.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
  }
  let want = store.get("sideWidth", null) ?? SIDE.def;   // the width chosen; a narrow window may hold less
  grip.setAttribute("aria-valuemin", String(SIDE.min));
  function set(px, save) {
    // clamped only when laid out: before (signing in, Files closed) the chosen width waits
    const mainPx = $("main").clientWidth, shown = document.body.classList.contains("docked") && mainPx > 0 && $("term").clientWidth > 0;
    const was = side(), at = shown && { mainPx, sidePx: was, termPx: termPx(), cellPx: cellWidth() || 8 };
    const w = shown ? clampSide(px, at) : Math.round(Math.max(SIDE.min, px));
    if (save) store.set("sideWidth", want = w);
    grip.setAttribute("aria-valuenow", String(w));
    if (w === was) return;
    root.style.setProperty("--side-w", `${w}px`);
    if (shown) grip.setAttribute("aria-valuemax", String(clampSide(Infinity, at)));
    if (shown) onResize();                         // a Files not docked leaves the terminal as it is
  }
  set(want, false);
  grip.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    grip.setPointerCapture(e.pointerId);
    const right = $("main").getBoundingClientRect().right, off = side() - (right - e.clientX);   // no jump where it is held
    document.body.classList.add("docksizing");   // the iframes let the pointer through (dock.css)
    const at = (ev) => right - ev.clientX + off;
    let frame = 0, last = null;                    // one width a frame, however many moves
    const move = (ev) => { last = ev; if (!frame) frame = requestAnimationFrame(() => { frame = 0; set(at(last), false); }); };
    const end = (ev) => {
      cancelAnimationFrame(frame);
      if (ev.type === "pointerup") set(at(ev), true);
      document.body.classList.remove("docksizing");
      for (const [t, f] of [["pointermove", move], ["pointerup", end], ["pointercancel", end]]) grip.removeEventListener(t, f);
    };
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", end);
    grip.addEventListener("pointercancel", end);
  });
  grip.addEventListener("keydown", (e) => {
    const step = { ArrowLeft: SIDE.step, ArrowRight: -SIDE.step }[e.key];   // left widens Files
    if (step) { e.preventDefault(); set(side() + step, true); }
  });
  grip.addEventListener("dblclick", () => set(SIDE.def, true));
  // another window width or Files docked again: the chosen width, within that width's bounds
  let docked = document.body.classList.contains("docked"), mainW = 0;
  new ResizeObserver(() => { if ($("main").clientWidth !== mainW) { mainW = $("main").clientWidth; set(want, false); } }).observe($("main"));
  new MutationObserver(() => {
    if (docked === document.body.classList.contains("docked")) return;
    docked = !docked;
    if (docked) set(want, false);
  }).observe(document.body, { attributeFilter: ["class"] });
}
