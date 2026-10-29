// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Rows dragged out of the tree (AC-36, AC-52): files' and folders' paths with their host, which the
// page's terminal puts at its input (web/static/upload.js); anywhere else plain text, one path a line.
// A finger held on the tree opens its context menu (AC-52).

const HOLD = 500;            // ms a finger rests before the menu opens (the page's session list too)

/** The drag's own type: {host, paths}, host null for this Mac. */
export const PATHS_TYPE = "application/x-terminum-paths";

/** Lets `el`'s rows (`.row[data-p]`) be dragged: `paths(p)` the paths a drag of row `p` carries,
 *  `host()` the host they are on. */
export function bindDrag(el: HTMLElement, paths: (p: string) => string[], host: () => string | null) {
  // a finger's press is a hold (its menu, bindHold), never a drag: on a phone Files and the terminal are not side by side
  let finger = false;
  el.addEventListener("pointerdown", (e) => { finger = e.pointerType === "touch"; });
  el.addEventListener("dragstart", (e) => {
    if (finger) return void e.preventDefault();
    const row = (e.target as Element).closest?.<HTMLElement>(".row[data-p]");
    if (!row || !e.dataTransfer) return;
    const list = paths(row.dataset.p!);
    e.dataTransfer.setData(PATHS_TYPE, JSON.stringify({ host: host(), paths: list }));
    e.dataTransfer.setData("text/plain", list.join("\n"));
    e.dataTransfer.effectAllowed = "copy";
  });
}

/** A finger resting on `el` HOLD ms (not moving) opens its context menu there, as a right click
 *  does: iOS Safari sends no contextmenu. The click its lift makes is dropped; a browser's own
 *  contextmenu meanwhile (Android) makes it no hold. */
export function bindHold(el: HTMLElement) {
  let hold: { x: number; y: number; t: number } | null = null, held = false;
  const cancel = () => { if (hold) clearTimeout(hold.t); hold = null; };
  el.addEventListener("pointerdown", (e) => {
    if (e.pointerType !== "touch" || (e.target as Element).closest?.("input, textarea")) return;   // a name being typed: its own hold
    const { clientX: x, clientY: y, target } = e;
    const p = (target as Element).closest?.<HTMLElement>(".row[data-p]")?.dataset.p;
    held = false;
    cancel();
    hold = { x, y, t: window.setTimeout(() => {
      hold = null;
      held = true;
      // its row, which the tree may have drawn again meanwhile (a listing came)
      const row = (target as Node).isConnected ? target : p ? [...el.querySelectorAll<HTMLElement>(".row[data-p]")].find((r) => r.dataset.p === p) : null;
      (row ?? el).dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: x, clientY: y }));
    }, HOLD) };
  });
  el.addEventListener("pointermove", (e) => { if (hold && Math.hypot(e.clientX - hold.x, e.clientY - hold.y) > 10) cancel(); });
  for (const type of ["pointerup", "pointercancel", "contextmenu"]) el.addEventListener(type, cancel);
  el.addEventListener("click", (e) => { if (held) { held = false; e.preventDefault(); e.stopImmediatePropagation(); } }, true);
}
