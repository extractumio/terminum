// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// On a touch screen the invisible keyboard field lies over the cursor's row or a coding agent's
// input box, so a finger held there gets the page's Paste, Undo and Redo (Input); elsewhere it is
// out of the way. While the keyboard is up it moves only when its row moves by a row or more:
// iOS scrolls the page to a focused field that moves, and the screen jumped on redraws.

const $ = (id) => document.getElementById(id);

/** touch: the touch screen's media query; mouseOn(): a full-screen program has the mouse (it gets the taps). */
export function kbdPlacer({ touch, mouseOn }) {
  return function placeKbd() {
    const k = $("kbd");
    if (!touch.matches) { if (k.style.cssText) k.style.cssText = ""; return; }   // a computer's keys need no place
    // a coding agent's input box whole (where it draws its own cursor may not be tmux's), else the cursor's row
    const cur = !mouseOn() ? $("screen").querySelector(".blk.inbox") ?? $("screen").querySelector(".cur") : null;
    const t = $("term").getBoundingClientRect(), r = cur?.getBoundingClientRect();
    if (!r || r.bottom < t.top || r.top > t.bottom) { k.style.cssText = ""; return; }
    const top = Math.round(Math.max(r.top, t.top)), bottom = Math.round(Math.min(r.bottom, t.bottom));
    const was = parseFloat(k.style.top), wasBottom = was + parseFloat(k.style.height), row = $("screen").querySelector(".ln")?.offsetHeight || 16;
    if (document.activeElement === k && k.style.width === `${t.width}px` && parseFloat(k.style.left) === t.left
      && Math.abs(top - was) < row && Math.abs(bottom - wasBottom) < row) return;
    k.style.cssText = `left:${t.left}px;top:${top}px;width:${t.width}px;height:${bottom - top}px`;
  };
}
