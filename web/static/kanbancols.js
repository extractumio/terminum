// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The board's columns, made once (terminum-kanban.md AC-105): each its head (its name, its keys, its count), its list
// of cards and its tab on a phone; a draw fills them. INTEGRATION stands hidden until a shown project integrates, its
// head holding the queue's keys instead of sort keys: its order is the queue's (terminum-integration.md AC-193).
import { el, iconButton } from "./listparts.js";
import { COLUMNS, button } from "./kanbanutil.js";

export function boardColumns({ sorting, showColumn, newTask, queueKeys }) {
  return new Map(COLUMNS.map((column) => {
    const col = el("section", "col"), head = el("h2", "colhead"), count = el("span", "count"), list = el("div", "cards");
    const queue = column === "INTEGRATION";
    col.dataset.status = column;
    col.hidden = queue;   // until a shown project integrates
    const keys = queue ? queueKeys() : sorting.keys(column);
    head.append(el("span", "colname", column), keys, count);
    if (column === "TODO") keys.before(iconButton("icon new", "plus", "New task", "A new task in TODO", newTask));   // AC-106
    col.append(head, list);
    const tab = button(null, column, () => showColumn(column));
    tab.dataset.status = column;
    tab.setAttribute("role", "tab");
    tab.hidden = queue;
    return [column, { col, count, list, tab, drawn: "" }];
  }));
}
