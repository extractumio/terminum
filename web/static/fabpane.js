// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A floating button's pane (the sent history's, the reminders'): modal, the page shaded behind it
// (#panescrim, controls.css): it takes the keys while open; Esc, its ×, its button or a press
// outside it close it, and the keys go back where they were. placeBeside also places the
// bookmarks' pane and find's field: left of their button, the first row centered on it, or under
// it where the view is too narrow (then `data-under` is set: the buttons below make way).

import { floats } from "./fold.js";   // a phone or a narrow window

const GAP = 14;                // px between a button and its pane

/** On a phone or a narrow window `el` is a centered modal sheet (sheets.css, AC-18): its inline
 *  place is cleared and `data-sheet` set, true returned; elsewhere false and the caller places it.
 *  With `close`, a shade (#sheetscrim) lies under it until it is hidden (a <dialog>: closed): a press
 *  there takes the whole press (nothing under it gets it) and calls `close`; the sheet takes the
 *  focus unless `focus` is false (the input keeps the keys, rowmenu.js). A modal <dialog> has its ::backdrop. */
export function placeModal(el, close, focus = true) {
  if (!floats()) { delete el.dataset.sheet; return false; }
  if ("sheet" in el.dataset && sheets.some((x) => x.el === el)) return true;   // open as one: placed again each frame, nothing to do
  for (const p of ["top", "left", "right", "bottom"]) el.style.removeProperty(p);
  delete el.dataset.under;
  el.dataset.sheet = "";
  if (close && !el.hidden) shadeUnder(el, close, focus);
  return true;
}
const LINGER = 400;            // ms the closed shade still takes the presses (the click after a tap)
const sheets = [];             // the open sheets with a shade, the top one last: {el, close}
function shade() {
  let s = document.getElementById("sheetscrim");
  if (s) return s;
  s = document.createElement("div");
  s.id = "sheetscrim";
  // the press is the shade's alone: no outside-press handler, no focus change; it closes the top
  // sheet on the finger's lift and the shade, clear, stays a moment for the click a phone sends after it
  for (const t of ["pointerdown", "pointerup", "mousedown", "mouseup", "touchstart", "touchend", "click"]) s.addEventListener(t, (e) => {
    e.stopPropagation();
    if (t === "mousedown" || (t === "pointerdown" && e.pointerType === "mouse")) e.preventDefault();
    if (t === "pointerup" && !s.style.opacity && sheets.length) { s.style.opacity = "0"; sheets.at(-1).close(); }
  });
  return s;
}
/** The shade under the top sheet, or gone (after LINGER when a press on it closed the last). */
function restack() {
  const s = shade(), top = sheets.at(-1);
  clearTimeout(s.later);
  if (top) {                                       // the sheet's stacking context, just under it
    if (s.nextElementSibling !== top.el) top.el.before(s);
    s.hidden = false; s.style.opacity = "";
    return;
  }
  if (s.style.opacity) s.later = setTimeout(() => { s.hidden = true; }, LINGER); else s.hidden = true;
}
let escapes = false;
const gone = (el) => el.hidden || !("sheet" in el.dataset) || (el.tagName === "DIALOG" && !el.open);
function shadeUnder(el, close, focus) {
  if (!escapes) {                                  // Esc closes the top sheet (a <dialog> over it closes itself first)
    escapes = true;
    document.addEventListener("keydown", (e) => {
      if (e.key !== "Escape" || !sheets.length || document.querySelector("dialog:modal")) return;   // a non-modal one (rowmenu.js) is a sheet here
      e.preventDefault();
      e.stopPropagation();
      sheets.at(-1).close();
    }, true);
  }
  if (!sheets.some((x) => x.el === el)) {
    sheets.push({ el, close });
    const watch = new MutationObserver(() => {
      if (!gone(el)) return;
      watch.disconnect();
      sheets.splice(sheets.findIndex((x) => x.el === el), 1);
      restack();
    });
    watch.observe(el, { attributes: true, attributeFilter: ["hidden", "data-sheet", "open"] });
  }
  restack();
  if (focus && !el.contains(document.activeElement)) el.querySelector("button:not(:disabled), input, textarea, [tabindex]")?.focus({ preventScroll: true });
}

/** Places `pane` beside `fab` (nothing while it is hidden). */
export function placeBeside(fab, pane) {
  if (pane.hidden) return;
  const area = fab.offsetParent;
  if (area && fab.offsetTop > area.clientHeight / 2) {      // a button at the bottom (a phone's row): the pane above it
    pane.style.top = `${Math.max(8, fab.offsetTop - 8 - pane.offsetHeight)}px`;
    pane.style.left = `${Math.max(8, Math.min(fab.offsetLeft, area.clientWidth - pane.offsetWidth - 8))}px`;
    delete pane.dataset.under;
    return;
  }
  const left = fab.offsetLeft - GAP - pane.offsetWidth, fits = left >= 8;
  // its first row shown (a header, the find field) level with the button's middle
  const head = [...pane.children].find((c) => c.offsetHeight > 0);
  const mid = head ? head.offsetTop + head.offsetHeight / 2 : fab.offsetHeight / 2;
  pane.style.top = `${fits ? Math.max(8, Math.round(fab.offsetTop + fab.offsetHeight / 2 - mid)) : fab.offsetTop + fab.offsetHeight + 8}px`;
  pane.style.left = `${fits ? left : Math.max(8, fab.offsetLeft + fab.offsetWidth - pane.offsetWidth)}px`;
  if (fits) delete pane.dataset.under; else pane.dataset.under = "";
}

/** The shade behind an open pane, made once; a press on it calls `onPress`. */
function scrim(onPress) {
  let el = document.getElementById("panescrim");
  if (!el) {
    el = document.createElement("div");
    el.id = "panescrim";
    el.hidden = true;
    document.getElementById("main").append(el);
  }
  el.onpointerdown = (e) => { e.preventDefault(); onPress(); };
  return el;
}

/** fab, pane, close: elements; onToggle(open) after it opened or closed. {toggle(on?), open(), place()}. */
export function fabPane({ fab, pane, close, onToggle }) {
  let open = false, back = null;
  function toggle(on = !open) {
    if (on === open) return;
    open = on;
    pane.hidden = !on;
    scrim(() => toggle(false)).hidden = !on;
    fab.setAttribute("aria-expanded", String(on));
    if (on) { back = document.activeElement; pane.focus({ preventScroll: true }); }
    else if (back?.isConnected) { back.focus({ preventScroll: true }); back = null; }
    onToggle(on);
    place();
  }
  const place = () => { if (!pane.hidden && !placeModal(pane)) placeBeside(fab, pane); };
  fab.addEventListener("click", () => toggle());
  fab.addEventListener("mousedown", (e) => e.preventDefault());   // the keys stay put until the pane opens
  close.addEventListener("click", () => toggle(false));
  pane.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { e.preventDefault(); toggle(false); }
    else if (e.key === "Tab") keepTab(pane, e);
  });
  // modal: the keys never reach what lies behind it (the terminal's input) while it is open;
  // a <dialog> opened over it takes them
  document.addEventListener("focusin", (e) => {
    if (open && !pane.contains(e.target) && !e.target.closest?.("dialog[open]")) pane.focus({ preventScroll: true });
  });
  return { toggle, open: () => open, place };
}

/** Tab and Shift+Tab go round the buttons and fields of `pane`, never out of it. */
function keepTab(pane, e) {
  const all = [...pane.querySelectorAll("button, input, textarea, select, a[href], [tabindex]:not([tabindex='-1'])")]
    .filter((el) => !el.disabled && el.offsetParent);
  const at = all.indexOf(document.activeElement), last = all.length - 1;
  if (at === -1 || (e.shiftKey ? at === 0 : at === last)) {
    e.preventDefault();
    (e.shiftKey ? all[last] : all[0])?.focus({ preventScroll: true });
  }
}
