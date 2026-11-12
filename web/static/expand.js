// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A box to read or type in, shown over the whole window (terminum.md AC-184): a key in its corner covers the
// window with it; the same key or Esc gives it back to its dialog. The field never moves, so what was typed,
// its undo, its keys and its draft stay as they were (expand.css). The board and the session page load it with
// taskboxes.js, the schedules' page with scheduled.js.
import { el, iconButton } from "./listparts.js";
import { icon } from "./icons.js";

const BACK = "Back to its window (Esc)", FULL = "Full window";
let shown = null;                                // the box over the window: one at a time (a redraw may drop it)

/** Wraps `field`, in the page or not yet, in a box with the key; answers the box. */
export function expandable(field) {
  const box = el("div", "xbox");
  field.replaceWith(box);
  box.append(field);
  addKey(box);
  return box;
}

/** Gives the key to each `.xbox` in the page that has none (the boxes the markup writes). */
export function expandKeys() {
  for (const box of document.querySelectorAll(".xbox")) if (!box.lastElementChild?.classList.contains("xkey")) addKey(box);
}

function addKey(box) {
  for (const part of box.children) if (part.tabIndex < 0) part.tabIndex = 0;   // a preview or a pre scrolls by the keys too
  const key = iconButton("icon xkey", "maximize-2", FULL, FULL, () => full(box, shown !== box));
  key.setAttribute("aria-pressed", "false");
  box.append(key);
}

/** Shows `box` over the window or back in its place; the part it shows takes the keys. */
function full(box, on) {
  if (on && shown) full(shown, false);
  shown = on ? box : null;
  box.classList.toggle("full", on);
  const key = box.lastElementChild;
  key.title = on ? BACK : FULL;
  key.setAttribute("aria-pressed", String(on));
  key.replaceChildren(icon(on ? "minimize-2" : "maximize-2", 14));
  [...box.children].find((c) => c !== key && !c.hidden)?.focus({ preventScroll: true });
}

/** Whether nothing stands over the box shown (a question opened after Save would): its middle is its own. */
function onTop() {
  const r = shown.getBoundingClientRect();
  return shown.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
}

// Esc gives the box back before anything else hears it (a keydown cancelled closes no dialog), unless a
// window over it has the keys
addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || !shown?.isConnected || e.isComposing || !onTop()) return;   // gone with a redraw: nothing to give back
  e.preventDefault();
  e.stopImmediatePropagation();
  full(shown, false);
}, true);
// a dialog closed another way (its own keys, a script) takes its box back with it; its close is heard a moment
// later, when it may be open again
addEventListener("close", (e) => { if (shown && !e.target.open && e.target.contains?.(shown)) full(shown, false); }, true);

expandKeys();
