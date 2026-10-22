// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A finger on the keyboard field, which lies over the cursor's row or a coding agent's input box
// (kbdplace.js; AC-03, AC-40). A short tap moves the cursor like a tap on the terminal; two quick
// taps select the word under them (the keyboard goes down, the selection keeps iOS's handles); a
// finger that rests a moment, however long, and then slides moves the cursor as in a text field:
// in an agent's input box onto the character under the finger, over its lines too; at a shell's
// prompt along the line, a character a step. Held still half a second and lifted, it opens the
// page's menu (Cut, Clear, Copy, Paste, Undo, Redo: undo.js). A quick swipe across is → to the
// right and the program's Undo to the left; one that moves at once otherwise scrolls the
// terminal. The finger's times are the events' own (when it touched), not when a busy page got
// to them.
import { termRange } from "./seltext.js";

export const MENU_AFTER = 500;   // ms a finger held still on the input (or a selection) before its lift opens the menu
const SLIDE_AFTER = 250;   // ms a finger rests on the input before sliding moves the cursor
export const TAP = 350;    // ms: a tap at most
const DOUBLE = 300;        // ms between two taps of a double tap, at most
// a swipe: quick, long and across, not from a screen edge (Safari's back and forward)
const SWIPE = { ms: 300, px: 48, edge: 24 };

/** Whether viewport point (x, y) lies on the selection `sel`. */
export function inSelection(sel, x, y) {
  return [...sel.getRangeAt(0).getClientRects()].some((r) => x >= r.left && x <= r.right && y >= r.top && y <= r.bottom);
}

/** Selects the word under (x, y) in the terminal, below the keyboard field `kbd` (which takes no
 *  pointer for the look), and puts the keyboard away: iOS keeps no page selection while a field
 *  has the focus. Returns the selection's top, or null. */
export function selectWordAt(kbd, x, y) {
  const was = kbd.style.pointerEvents;
  kbd.style.pointerEvents = "none";
  const at = document.caretRangeFromPoint?.(x, y);
  kbd.style.pointerEvents = was;
  kbd.blur();
  if (!at) return null;
  const sel = getSelection();
  sel.collapse(at.startContainer, at.startOffset);
  sel.modify("move", "backward", "word");
  sel.modify("extend", "forward", "word");
  return sel.rangeCount && !sel.isCollapsed ? sel.getRangeAt(0).getBoundingClientRect().top : null;
}

/** What a finger's lift was: "tap", "menu", "swipe-left", "swipe-right", "hold" (still, too long for
 *  a tap, too short for the menu: nothing) or "" (a slide or a scroll, already done).
 *  down: {x, y, at, slide, scroll, across}; up: {x, y, at}; width: the screen's. Pure. */
export function liftOf(down, up, width) {
  const dx = up.x - down.x, dy = up.y - down.y, ms = up.at - down.at, far = Math.hypot(dx, dy);
  if (down.slide) return "";
  if (ms < SWIPE.ms && Math.abs(dx) >= SWIPE.px && Math.abs(dx) >= 2 * Math.abs(dy)
      && down.x >= SWIPE.edge && down.x <= width - SWIPE.edge) return dx > 0 ? "swipe-right" : "swipe-left";
  if (down.scroll !== undefined || down.across || far >= 10) return "";
  if (ms >= MENU_AFTER) return "menu";
  return ms < TAP ? "tap" : "hold";
}

/** kbd: the keyboard field; term: the terminal; options: Input's (holdMenu, inBox, cellWidth, tapMove,
 *  tapLink, appCursor, undo); type(text, path): to the pane; refill(): the field's kept
 *  characters back; touched(): a finger was on the field now. */
export function kbdTouch({ kbd, term, options, type, refill, touched }) {
  let down = null, lastTap = null;
  const arrow = (dir) => type(options.appCursor() ? `\x1bO${dir}` : `\x1b[${dir}`, "slide");
  kbd.addEventListener("touchstart", (e) => {
    const t = e.touches[0];
    touched();
    down = t && e.touches.length === 1 ? { x: t.clientX, y: t.clientY, at: e.timeStamp, slide: false, moved: 0 } : null;
  }, { passive: true });
  kbd.addEventListener("touchcancel", () => { down = null; });
  kbd.addEventListener("touchmove", (e) => {
    const t = e.touches[0];
    if (!down || !t) return;
    const dx = t.clientX - down.x, dy = t.clientY - down.y;
    if (down.scroll !== undefined) {                                 // a swipe over the field scrolls the terminal
      e.preventDefault();
      term.scrollTop += down.scroll - t.clientY;
      down.scroll = t.clientY;
      return;
    }
    if (down.across) { e.preventDefault(); return; }               // across at once: a swipe or nothing, decided at the lift
    if (!down.slide) {
      const rest = e.timeStamp - down.at;
      if (rest < SLIDE_AFTER) {                              // moved at once: up or down scrolls; across is a swipe, still
        if (Math.hypot(dx, dy) > 10) { if (Math.abs(dx) > Math.abs(dy)) down.across = true; else down.scroll = t.clientY; }
        return;
      }
      down.box = !!options.inBox?.();                          // the finger draws the cursor over the box's lines
      if (Math.hypot(dx, dy) < 4 || (!down.box && Math.abs(dx) < Math.abs(dy))) return;
      down.slide = true;
    }
    e.preventDefault();                                             // the slide is ours, not iOS's loupe
    const w = options.cellWidth?.() || 8;
    if (down.box) {                                                 // to the character under the finger, once it reaches another
      if (down.to && Math.abs(t.clientX - down.to.x) < w / 2 && Math.abs(t.clientY - down.to.y) < w) return;
      down.to = { x: t.clientX, y: t.clientY };
      options.tapMove?.(t.clientX, t.clientY);
      return;
    }
    const cells = Math.trunc(dx / w);
    for (; down.moved < cells; down.moved++) arrow("C");
    for (; down.moved > cells; down.moved--) arrow("D");
  }, { passive: false });
  kbd.addEventListener("touchend", (e) => {
    touched();
    refill();                                                       // the pane's cursor goes where the finger says
    const t = e.changedTouches[0], d = down;
    down = null;
    if (!d || !t) return;
    const up = { x: t.clientX, y: t.clientY, at: e.timeStamp }, what = liftOf(d, up, innerWidth);
    if (what === "hold") { lastTap = null; return; }
    if (what !== "tap") {
      e.preventDefault();                                           // its tap would move the cursor
      lastTap = null;
      if (what === "menu") options.holdMenu?.(up.x, up.y);
      else if (what === "swipe-right") arrow("C");
      else if (what === "swipe-left") options.undo?.();
      return;
    }
    if (lastTap && up.at - lastTap.at < DOUBLE && Math.hypot(up.x - lastTap.x, up.y - lastTap.y) < 24) {
      e.preventDefault();                                           // no zoom, no second move
      lastTap = null;
      selectWordAt(kbd, up.x, up.y);
      return;
    }
    lastTap = up;
    if (!options.tapLink?.(up.x, up.y)) options.tapMove?.(up.x, up.y);   // an address or a path on the row still opens
  });
  // a selection in the terminal takes the touches the field would (its handles, a finger held on it)
  document.addEventListener("selectionchange", () => {
    kbd.classList.toggle("yield", !!termRange(term) && document.activeElement !== kbd);
  });
}
