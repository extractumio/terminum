// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A task that needs a human (terminum-kanban.md AC-117): a line as a waiting agent's, on both pages,
// and how many on Kanban in the page's head: those waiting for a human in yellow, those failed in red. terminumd says which lines are up and keeps which a human
// closed or saw, so every browser shows the same; a line being closed stays hidden while the close goes.
import { icon } from "./icons.js";
import { el, iconButton, plural } from "./listparts.js";
import { kanbanLinks } from "./sections.js";
import { api } from "./kanbanutil.js";

/** Lines in `box`; `open(id)` shows a task (its window then says it was seen); `say(text)` tells a refused close. */
export function taskLines(box, open, say) {
  let last = [];
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
    c.append(go, iconButton("x", "x", "Close this line", "Close this line", () => shut(l.id, l.key)));
    return c;
  }

  /** Told to terminumd, which tells every page: the lines of `id` hidden here meanwhile, back on a refusal. `key`:
   *  the line closed; none: the line the task has now is seen (its window opened). */
  function shut(id, key) {
    const hid = last.filter((l) => l.id === id && (!key || l.key === key) && !closing.has(l.key)).map((l) => l.key);
    if (key && !hid.length) return;
    for (const k of hid) closing.add(k);
    draw();
    api(`/api/tasks/${encodeURIComponent(id)}/seen`, { method: "POST", body: key ? { key } : {} }).catch((e) => {
      if (!hid.length) return;                    // nothing shown went: the window says its own failures
      for (const k of hid) closing.delete(k);
      draw();
      say(`${id}'s line: ${e.message}`);
    });
  }

  function draw() {
    const due = last.filter((l) => !closing.has(l.key));
    for (const c of box.querySelectorAll(".taskline")) c.remove();
    box.append(...due.map(line));
    box.hidden = !box.childElementCount;
  }

  return {
    /** terminumd's `taskmarks`: the lines due, and how many cards wait for a human and how many failed. */
    came(m) {
      last = m.lines ?? [];
      const now = new Set(last.map((l) => l.key));
      for (const k of closing) if (!now.has(k)) closing.delete(k);
      draw();
      for (const a of kanbanLinks()) {
        a.querySelector(".taskneeds")?.remove();
        const counts = [["waiting", m.waiting, (n) => `${plural(n, "task")} wait for you`], ["failed", m.failed, (n) => `${plural(n, "task")} failed`]].filter(([, n]) => n > 0);
        if (!counts.length) continue;
        const box = el("span", "taskneeds");
        for (const [kind, n, says] of counts) {
          const b = el("span", `taskneed ${kind}`, n);
          b.title = says(n);
          box.append(b);
        }
        a.append(box);
      }
    },
    /** A task's window opened: its line is seen, in every browser. */
    opened(id) { shut(id); },
  };
}
