// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// An agent's queue (terminum-dispatcher.md AC-169): the tasks that wait for its room, in the order they start.
// On its rectangle a pale dot for each and the key that opens its window at the queue; in its window the list:
// a row is dragged to another place (reorder.js: a mouse at once, a finger after a rest) or moved by its arrows,
// and taken out by its cross. terminumd keeps the order and says what it refuses; every page hears.
import { el, iconButton } from "./listparts.js";
import { bindReorder } from "./reorder.js";
import { api } from "./kanbanutil.js";

const MOST_DOTS = 6;                               // more waiting than this: the count says the rest

/** The key on an agent's rectangle: a pale dot for each task of `queue`, and how many; a press opens the queue. */
export function queueKey(a, queue, open) {
  const key = el("button", "agqueue");
  key.type = "button";
  const names = queue.map((t) => t.id).join(", ");
  key.title = `Queue: ${names}. Press to change its order or take a task out`;
  key.setAttribute("aria-label", `${a.name}'s queue: ${queue.length} waiting`);
  for (const t of queue.slice(0, MOST_DOTS)) key.append(el("span", "agwait"));
  key.append(el("small", "agcount", `${queue.length} queued`));
  key.addEventListener("click", (e) => { e.stopPropagation(); open(); });
  return key;
}

/** The queue in an agent's window, in `box`. `projectName(id)`; `say(words)`: what terminumd refused. Returns
 *  `show(agent, queue)`, with no agent nothing, and `at()`: the queue scrolled into view. terminumd tells every page
 *  of a change, this one too: the board's next read shows it. */
export function queueList(box, { projectName, say }) {
  const head = el("h3", "agqueuehead", "Queue"), list = el("ol", "agqueuelist"), none = el("p", "hint");
  list.setAttribute("aria-label", "Queue");
  box.append(head, list, none);
  let agent = null, last = [], drawn = "";

  /** A change to terminumd; refused, the list goes back to what terminumd has. */
  async function send(path, body) {
    say("");
    try { await api(path, { method: "POST", body }); } catch (e) {
      say(e.message);
      drawn = "";
      show(agent, last);
    }
  }
  const order = (ids) => send(`/api/task-agents/${agent.id}/queue`, { tasks: ids });
  const ids = () => [...list.children].map((li) => li.querySelector(".item").dataset.id);
  function move(id, by) {
    const now = ids(), at = now.indexOf(id);
    now.splice(at + by, 0, ...now.splice(at, 1));
    return order(now);
  }

  function row(t, i, all) {
    const li = el("li", "agqueued"), item = el("span", "item"), acts = el("span", "acts");
    li.dataset.drag = "row";
    item.dataset.id = t.id;
    const grip = el("span", "qgrip");
    grip.setAttribute("aria-hidden", "true");
    item.append(grip, el("span", "qplace", `${i + 1}`), el("b", null, t.id), el("span", "qtitle", t.title), el("small", null, projectName(t.project) ?? ""));
    const up = iconButton("icon", "arrow-up", `Move ${t.id} up`, "Start it sooner", () => move(t.id, -1));
    const down = iconButton("icon", "arrow-down", `Move ${t.id} down`, "Start it later", () => move(t.id, 1));
    up.disabled = i === 0;
    down.disabled = i === all.length - 1;
    acts.append(up, down, iconButton("icon", "x", `Take ${t.id} out of the queue`, "Take it out of the queue: nothing starts, the task stays where it is",
      () => send(`/api/tasks/${t.id}/give-up`)));
    li.append(item, acts);
    return li;
  }

  const drag = bindReorder(list, (_kind, _parent, moved) => order(moved));

  function show(a, queue) {
    agent = a;
    last = queue;
    box.hidden = !a;
    const now = JSON.stringify([a?.id, queue]);
    if (!a || now === drawn || drag.dragging()) return;
    drawn = now;
    list.replaceChildren(...queue.map(row));
    none.textContent = queue.length ? (queue.length > 1 ? "The first starts when a slot frees. Drag a task, or use its arrows, to change the order." : "It starts when a slot frees.")
      : "No task waits for this agent. A task run while it is at its limit can be added here.";
  }
  return { show, at: () => head.scrollIntoView({ block: "nearest" }) };
}
