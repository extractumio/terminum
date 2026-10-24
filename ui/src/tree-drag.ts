// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Rows dragged out of the tree (AC-36, AC-52): files' and folders' paths with their host, which the
// page's terminal puts at its input (web/static/upload.js); anywhere else plain text, one path a line.

/** The drag's own type: {host, paths}, host null for this Mac. */
export const PATHS_TYPE = "application/x-terminum-paths";

/** Lets `el`'s rows (`.row[data-p]`) be dragged: `paths(p)` the paths a drag of row `p` carries,
 *  `host()` the host they are on. */
export function bindDrag(el: HTMLElement, paths: (p: string) => string[], host: () => string | null) {
  el.addEventListener("dragstart", (e) => {
    const row = (e.target as Element).closest?.<HTMLElement>(".row[data-p]");
    if (!row || !e.dataTransfer) return;
    const list = paths(row.dataset.p!);
    e.dataTransfer.setData(PATHS_TYPE, JSON.stringify({ host: host(), paths: list }));
    e.dataTransfer.setData("text/plain", list.join("\n"));
    e.dataTransfer.effectAllowed = "copy";
  });
}
