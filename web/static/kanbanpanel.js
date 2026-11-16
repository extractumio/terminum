// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Where the columns stand (terminum-kanban.md AC-105; boardpanel.css): wider than a phone, TODO is a column of its own
// and the others share the rest of the board, one at a time under their tabs, the one chosen remembered in this
// browser; on a phone (BOARD_PHONE) one column shows at a time, a swipe away from the next, the six tabs over them.
import { el, read, write } from "./listparts.js";
import { $ } from "./kanbanutil.js";
import { BOARD_PHONE } from "./phonewin.js";

const KEPT = "board.tab", FIRST = "IN PROGRESS";

/** `columns`: kanbancols.js's, by name; `still`: Reduce Motion's query. */
export function columnPanel(columns, still) {
  const phone = matchMedia(BOARD_PHONE), board = $("board"), tabs = $("colswitch"), panel = el("div", "colpanel");
  const todo = columns.get("TODO");
  panel.append(...[...columns.values()].filter((c) => c !== todo).map((c) => c.col));
  board.replaceChildren(todo.col, panel);
  tabs.replaceChildren(...[...columns.values()].map((c) => c.tab));
  const kept = read(KEPT);
  let chosen = columns.has(kept) && kept !== "TODO" ? kept : FIRST;   // the panel's
  let swiped = "TODO";                             // a phone's column in sight

  /** The column the tabs say: a phone's in sight, else the panel's; a hidden one's (INTEGRATION turned off) is the first. */
  const shown = () => (phone.matches ? swiped : columns.get(chosen).tab.hidden ? FIRST : chosen);

  /** Its tab pressed, the panel's column shown. */
  function mark() {
    const now = shown();
    for (const [name, { tab, col }] of columns) {
      const on = name === now;
      col.classList.toggle("shown", on);
      if (tab.getAttribute("aria-selected") === String(on)) continue;
      tab.setAttribute("aria-selected", String(on));
      if (on && phone.matches) tab.scrollIntoView({ inline: "nearest", block: "nearest" });   // a phone's tabs scroll; the panel's wrap
    }
  }

  /** A tab's press: on a phone its column slides into sight, else the panel shows it. */
  function show(column) {
    if (phone.matches) {
      swiped = column;
      columns.get(column).col.scrollIntoView({ inline: "start", block: "nearest", behavior: still.matches ? "auto" : "smooth" });
    } else {
      chosen = column;
      write(KEPT, column);
    }
    mark();
  }

  // a phone's swipe: the tab follows the column in sight; a hidden column takes no place
  board.addEventListener("scroll", () => {
    if (!phone.matches) return;
    const cols = [...columns.values()].filter((c) => !c.col.hidden).map((c) => c.col);
    const now = cols[Math.round(board.scrollLeft / Math.max(1, board.clientWidth))]?.dataset.status;
    if (now && now !== swiped) { swiped = now; mark(); }
  }, { passive: true });

  /** The tabs: over the phone's columns, under its head's line; else the panel's own head. */
  function arrange() {
    if (phone.matches) $("poolline").before(tabs);
    else panel.prepend(tabs);
    mark();
  }
  phone.addEventListener("change", arrange);
  arrange();
  return { show, mark };
}
