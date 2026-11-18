// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A board section's rail (terminum-kanban.md AC-203): its name turned on a strip at its left, the fold key of Agent
// Pool and Routines; folded, the section is one strip: its name, a line of what it holds, its add key. Remembered by
// this browser, only when it differs from the section's default.
import { pointerKey, read, write } from "./listparts.js";

/** `section`: a `.railed` with its `.railkey`; `body`: what folds; `key`: storage; `open`: the default; `add()`: its
 *  add key's press, the tile's and the folded strip's. */
export function rail(section, body, { key, open, add }) {
  // a pointer's press leaves no focus ring, as the board's keys (docs/UI-GUIDELINES.md "Focus")
  const fold = pointerKey(section.querySelector(".railkey")), sum = section.querySelector(".railsum"), plus = pointerKey(section.querySelector(".railadd"));
  let opened = (read(key) === true) !== open;
  function draw() {
    section.classList.toggle("folded", !opened);
    fold.setAttribute("aria-expanded", String(opened));
    body.hidden = !opened;
    plus.hidden = opened;
  }
  fold.addEventListener("click", () => {
    opened = !opened;
    write(key, opened !== open || null);
    draw();
  });
  plus.addEventListener("click", add);
  draw();
  return {
    /** The folded strip's line: what the section holds. */
    summary(text) { if (sum.textContent !== text) sum.textContent = text; },
  };
}
