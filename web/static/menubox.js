// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A modal menu in a <dialog role=menu> (projects.css): its items under a head saying what it is for, with × to close
// it; at the press on a computer, a sheet on a phone (fabpane.js). The page is shaded and takes no press while it is
// open; a press on the shade closes it and does nothing else; Esc closes it, the focus going back; ↑ ↓ walk its
// items. The session list's row menu (rowmenu.js) and a card's menu on the board (kanbancardkeys.js) are its.
import { inside, placeModal } from "./fabpane.js";
import { floats } from "./fold.js";
import { icon } from "./icons.js";
import { pointerKey } from "./listparts.js";

/** `box`: the dialog; `marks`: each item's icon by its action (icons.js). */
export function menuBox(box, marks = {}) {
  let openedAt = -Infinity, back = null;           // when a finger opened it; what had the focus
  const hide = () => {
    if (!box.open) return;
    box.close();
    if (back?.isConnected && box.contains(document.activeElement)) back.focus({ preventScroll: true });
  };
  const button = (label, action, run, enabled = true, mark = marks[action]) => {
    const b = document.createElement("button");
    b.type = "button"; b.textContent = label; b.dataset.action = action; b.disabled = !enabled;
    b.setAttribute("role", "menuitem");
    if (mark) b.prepend(icon(mark, 16));
    // the lift of the finger that opened it is no press of what is under it now
    b.onclick = (e) => { e.stopPropagation(); if (performance.now() - openedAt > 350) run(); };
    pointerKey(b);                                 // the input keeps the keys (a menu opened with them, open)
    return b;
  };
  /** The menu's items under a head: what it is for, and × to close it. */
  function fill(title, ...items) {
    const head = document.createElement("div");
    head.className = "mhead";
    const t = document.createElement("span");
    t.textContent = title;
    const x = button("×", "x", hide);
    x.title = "Close this menu";
    x.setAttribute("aria-label", "Close this menu");
    x.removeAttribute("role");
    head.append(t, x);
    box.replaceChildren(head, ...items);
  }
  /** keepKeys: the field with the keys keeps them (the on-screen keyboard stays up, the sheet with it):
   *  opened without showModal (it takes the focus and makes the page inert), over a sheet's shade. */
  function open(x, y, byFinger = false, keepKeys = false) {
    const keep = keepKeys && floats() && !box.open;
    if (!box.open) { back = document.activeElement; if (keep) box.setAttribute("open", ""); else box.showModal(); }
    openedAt = byFinger ? performance.now() : -Infinity;
    const w = box.offsetWidth, h = box.offsetHeight;
    if (!placeModal(box, keep ? hide : null, !keep)) {   // a computer's: at the pointer
      box.style.left = `${inside(x, w, innerWidth)}px`;
      box.style.top = `${inside(y, h, innerHeight)}px`;
    }
    // never on a destructive first item (Close Session): Enter after opening never asks to close it
    if (!keep) box.querySelector("button:not(:disabled):not([data-action=x]):not([data-action=close])")?.focus({ preventScroll: true });
  }
  box.addEventListener("keydown", (e) => {          // ↑ ↓ through its items, as a menu
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = [...box.querySelectorAll("button:not(:disabled)")], i = items.indexOf(document.activeElement);
    items[(i + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length]?.focus();
  });
  // a press on the shade (the dialog itself, outside its box) closes it, and reaches nothing
  // under it; the lift of the finger that opened it does not
  box.addEventListener("click", (e) => {
    if (e.target !== box) return;
    const r = box.getBoundingClientRect(), within = e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
    if (!within && performance.now() - openedAt > 350) hide();
  });
  box.addEventListener("cancel", (e) => { e.preventDefault(); hide(); });   // Esc: the focus goes back too
  return { box, hide, button, fill, open };
}
