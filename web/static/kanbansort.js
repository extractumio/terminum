// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Each column's display order (terminum-kanban.md AC-105), remembered per project in this browser.
import { el, iconButton, read, write } from "./listparts.js";

const MODES = [["recent", "history", "Recently updated first"], ["priority", "list-filter", "Priority, then recently updated"]];

/** Several projects share one preference, apart from each project's own; sorting is done by terminumd. */
export function columnSort(projects, changed) {
  const key = () => `board.sort.${projects().length === 1 ? projects()[0] : "*"}`;
  const memory = new Map();                         // a blocked browser store still permits sorting in this page
  const mode = (column) => (memory.get(key()) ?? read(key()))?.[column] === "recent" ? "recent" : "priority";
  const controls = new Map();
  function draw() {
    for (const [column, box] of controls) {
      for (const b of box.children) b.setAttribute("aria-pressed", String(b.dataset.sort === mode(column)));
    }
  }
  return {
    mode,
    draw,
    keys(column) {
      const box = el("span", "colsort");
      box.setAttribute("role", "group");
      box.setAttribute("aria-label", `Sort ${column}`);
      for (const [value, icon, label] of MODES) {
        const b = iconButton("icon", icon, label, label, () => {
          if (mode(column) === value) return;
          const saved = Object.fromEntries([...controls.keys()].map((c) => [c, mode(c)]));
          const next = { ...saved, [column]: value };
          memory.set(key(), next);
          write(key(), next);
          draw();
          changed();
        });
        b.dataset.sort = value;
        box.append(b);
      }
      controls.set(column, box);
      return box;
    },
  };
}
