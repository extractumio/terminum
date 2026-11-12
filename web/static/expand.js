// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A box to read or type in, shown over the whole window (terminum.md AC-184): a key in its corner covers the
// window with it; the same key or Esc gives it back to its dialog. The field never moves, so what was typed,
// its undo, its keys and its draft stay as they were (expand.css). The board and the session page load it with
// taskboxes.js, the schedules' page with scheduled.js.
import { icon } from "./icons.js";

const WORDS = { true: "Back to its window (Esc)", false: "Full window" };

/** Wraps `field`, in the page or not yet, in a box with the key; answers the box. */
export function expandable(field) {
  const box = document.createElement("div");
  box.className = "xbox";
  field.replaceWith(box);
  box.append(field);
  addKey(box);
  return box;
}

/** Gives the key to each `.xbox` under `root` that has none (the boxes the markup writes). */
export function expandKeys(root = document) {
  for (const box of root.querySelectorAll(".xbox:not(:has(> .xkey))")) addKey(box);
}

function addKey(box) {
  const key = document.createElement("button");
  key.type = "button";
  key.className = "icon xkey";
  key.addEventListener("click", () => full(box, !box.classList.contains("full"), true));
  for (const shown of box.children) if (shown.tabIndex < 0) shown.tabIndex = 0;   // a preview scrolls by the keys too
  box.append(key);
  drawKey(key, false);
}

function drawKey(key, on) {
  key.replaceChildren(icon(on ? "minimize-2" : "maximize-2", 14));
  key.title = WORDS[on];
  key.setAttribute("aria-label", WORDS[on]);
  key.setAttribute("aria-pressed", String(on));
}

/** Shows `box` over the window or back in its place; `focus`: the field it shows takes the keys. */
function full(box, on, focus) {
  box.classList.toggle("full", on);
  const key = box.querySelector(":scope > .xkey");
  drawKey(key, on);
  if (focus) [...box.children].find((c) => c !== key && !c.hidden)?.focus({ preventScroll: true });
}

/** Whether nothing stands over `box` (a question opened after Save would): its middle is its own. */
function onTop(box) {
  const r = box.getBoundingClientRect();
  return box.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
}

// Esc gives the box back before anything else hears it (a keydown cancelled closes no dialog), unless a
// window over it has the keys
addEventListener("keydown", (e) => {
  const box = e.key === "Escape" && !e.isComposing && document.querySelector(".xbox.full");
  if (!box || !onTop(box)) return;
  e.preventDefault();
  e.stopImmediatePropagation();
  full(box, false, true);
}, true);
// a dialog closed another way (its own keys, a script) takes its box back with it
addEventListener("close", (e) => { for (const box of e.target.querySelectorAll?.(".xbox.full") ?? []) full(box, false); }, true);

expandKeys();
