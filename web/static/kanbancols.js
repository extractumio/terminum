// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The board's columns, made once (terminum-kanban.md AC-105): each its head (its name, its keys, its count), its list
// of cards and its tab (kanbanpanel.js), whose count of WAIT FOR HUMAN's or FAILED's tasks pulses while it holds any;
// a draw fills them. INTEGRATION stands hidden until a shown project integrates, its
// head holding the queue's keys instead of sort keys: its order is the queue's (terminum-integration.md AC-193).
import { icon } from "./icons.js";
import { el } from "./listparts.js";
import { COLUMNS, button } from "./kanbanutil.js";

const ALARMS = { "WAIT FOR HUMAN": "waiting", FAILED: "failed" };   // their counts in the head counters' hues (askfold.css)

/** TODO's + New task, a text key as tall as the head's icon keys (AC-106). */
function newKey(newTask) {
  const b = button("textkey new", "", (e) => { e.stopPropagation(); newTask(); });
  b.id = "newtask";
  b.title = "A new task in TODO";
  b.setAttribute("aria-label", "New task");
  b.append(icon("plus", 14), el("span", null, "New task"));
  return b;
}

export function boardColumns({ sorting, showColumn, newTask, queueKeys }) {
  return new Map(COLUMNS.map((column) => {
    const col = el("section", "col"), head = el("h2", "colhead"), count = el("span", "count"), list = el("div", "cards");
    const queue = column === "INTEGRATION";
    col.dataset.status = column;
    col.hidden = queue;   // until a shown project integrates
    const keys = queue ? queueKeys() : sorting.keys(column);
    head.append(el("span", "colname", column), keys, count);
    if (column === "TODO") keys.before(newKey(newTask));
    col.append(head, list);
    const tab = button(null, "", () => showColumn(column)), tabCount = el("span", "tabn");
    tab.append(el("span", "tabname", column), tabCount);
    tab.dataset.status = column;
    tab.setAttribute("role", "tab");
    tab.hidden = queue;
    const drawCount = (counts) => {
      const n = counts[column] ?? 0, queued = column === "WAIT FOR HUMAN" ? (counts.queued_waiting ?? 0) : 0;
      const label = `${n - queued}${queued ? ` · ${queued} queued` : ""}`;
      count.textContent = label;
      tabCount.textContent = label;
      const alarm = column in ALARMS && n - queued > 0;   // tasks that wait for the human (AC-117)
      tab.classList.toggle("alarm", alarm);
      if (ALARMS[column]) tabCount.classList.toggle(ALARMS[column], alarm);
    };
    return [column, { col, count, list, tab, drawCount, drawn: "" }];
  }));
}
