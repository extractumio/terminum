// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A task that needs a human (terminum-kanban.md AC-117): a line as a waiting agent's, on both pages,
// and how many on Kanban in the page's head. terminumd says which lines are up and keeps which a human
// closed or saw, so every browser shows the same; a line being closed stays hidden while the close goes.
import { icon } from "./icons.js";
import { el, iconButton, plural } from "./listparts.js";
import { kanbanLinks } from "./sections.js";
import { api } from "./kanbanutil.js";

try { localStorage.removeItem("tm.taskseen"); } catch { /* where each browser kept its own closes */ }

/** Lines in `box`; `open(id)` shows a task (its window then says it was seen); `say(text)` tells a refused close. */
export function taskLines(box, open, say) {
  let last = [], heard = false, opened = new Set();   // opened: windows opened before terminumd's first lines
  const closing = new Set();       // keys sent or kept by terminumd, until its lines leave them out

  function line(l) {
    const c = el("div", "ask taskline");
    c.dataset.task = l.id;
    const go = el("button", "go"), mark = el("span", `state ${l.failed ? "failed" : "waiting"}`);
    go.type = "button";
    mark.append(icon(l.failed ? "triangle-alert" : "circle-help", 14));
    go.append(mark, el("span", "what", l.words));
    go.title = `Open ${l.id}`;
    go.addEventListener("click", () => open(l.id));
    c.append(go, iconButton("x", "x", "Close this line", "Close this line", () => shut(l)));
    return c;
  }

  /** Told to terminumd, which tells every page; refused, the line comes back with its words. */
  function shut(l) {
    if (closing.has(l.key)) return;
    closing.add(l.key);
    draw();
    api(`/api/tasks/${encodeURIComponent(l.id)}/seen`, { method: "POST", body: { key: l.key } }).catch((e) => {
      closing.delete(l.key);
      draw();
      say(`${l.id}'s line: ${e.message}`);
    });
  }

  function draw() {
    const due = last.filter((l) => !closing.has(l.key));
    for (const c of box.querySelectorAll(".taskline")) c.remove();
    box.append(...due.map(line));
    box.hidden = !box.childElementCount;
  }

  return {
    /** terminumd's `taskmarks`: the lines due, and how many tasks need a human. */
    came(m) {
      last = m.lines ?? [];
      heard = true;
      const now = new Set(last.map((l) => l.key));
      for (const k of closing) if (!now.has(k)) closing.delete(k);
      for (const l of last) if (opened.has(l.id)) shut(l);   // a window opened before the lines came
      opened = new Set();
      draw();
      for (const a of kanbanLinks()) {
        a.querySelector(".taskneed")?.remove();
        if (!m.need) continue;
        const n = el("span", "taskneed", m.need);
        n.title = `${plural(m.need, "task")} wait for you or failed`;
        a.append(n);
      }
    },
    /** A task's window opened: its line is seen, in every browser. */
    opened(id) {
      const had = last.filter((l) => l.id === id);
      if (!heard) opened.add(id);
      for (const l of had) shut(l);
    },
  };
}
