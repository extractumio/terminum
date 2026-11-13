// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// An agent's queue (terminum-dispatcher.md AC-169): the tasks that wait for its room, in the order they start.
// On its rectangle a pale dot for each and the key that opens its window at the queue; in its window the list:
// a row is dragged to another place (reorder.js: a mouse at once, a finger after a rest) or moved by its arrows,
// and taken out by its cross. A row is a press: Run on several tasks waits as one. terminumd keeps the order and says
// what it refuses; every page hears.
import { el, iconButton, pointerKey } from "./listparts.js";
import { bindReorder } from "./reorder.js";
import { api } from "./kanbanutil.js";

const MOST_DOTS = 6;                               // more waiting than this: the count says the rest
/** A queued press by its tasks: its first, and the others one Run took with it. */
const names = (t) => [t.id, ...(t.also ?? [])].join(", ");

/** The key on an agent's rectangle: a pale dot for each task of `queue`, and how many; a press opens the queue. */
export function queueKey(a, queue, open) {
  const key = pointerKey(el("button", "agqueue"));
  key.type = "button";
  key.title = `Queue: ${queue.map(names).join("; ")}. Press to change its order or take a task out`;
  key.setAttribute("aria-label", `${a.name}'s queue: ${queue.length} waiting`);
  key.append(...queue.slice(0, MOST_DOTS).map(() => el("span", "agwait")), el("small", "agcount", `${queue.length} queued`));
  key.addEventListener("click", (e) => { e.stopPropagation(); open(); });   // the card's own press opens no second window
  return key;
}

/** The queue in an agent's window, in `box`. `projectName(id)`; `say(words)`: what terminumd refused;
 *  `refresh(id)`: the queue from a new board read, undefined on failure. Returns
 *  `show(agent, queue)`, with no agent nothing, and `at()`: the queue scrolled into view. terminumd tells every page
 *  of a change, this one too: the board's next read shows it. */
export function queueList(box, { projectName, say, refresh }) {
  const head = el("h3", "agqueuehead", "Queue"), list = el("ol", "agqueuelist"), none = el("p", "hint");
  list.setAttribute("aria-label", "Queue");
  box.append(head, list, none);
  let state = null, drawn = "";
  const states = new Map();                         // outstanding writes keep their agent when its window changes
  const forget = (mine) => { if (mine && mine !== state && !mine.pending && !mine.error) states.delete(mine.id); };

  /** The list as `queue`, drawn anew. */
  function redraw() {
    drawn = "";
    draw();
  }
  /** A change to terminumd, shown at once as `next`, so a second press counts from it and the board's read finds the
   *  list as it stands; refused, the list goes back and says why. */
  function send(path, body, next) {
    const mine = state;
    mine.error = "";
    say("");
    mine.queue = next;
    mine.pending++;
    redraw();
    const report = (words) => { mine.error = words; if (state === mine) say(words); };
    // Each intent is shown at once, but the server receives them in click order. A failed earlier write must not
    // roll back a later intent; once the tail drains, a new board read reconciles successes and refusals alike.
    mine.tail = mine.tail.then(async () => {
      try { await api(path, { method: "POST", body }); } catch (e) { report(e.message); }
      try {
        if (mine.pending === 1) {
          const queue = await refresh(mine.id);
          if (queue === undefined) throw new Error("The queue could not be refreshed. Look at it again.");
          if (mine.pending === 1) mine.queue = queue; // an intent made during the read remains optimistic
        }
      } catch (e) { report([mine.error, e.message].filter(Boolean).join(" ")); }
      finally { mine.pending--; if (state === mine) redraw(); else forget(mine); }
    });
    return mine.tail;
  }
  const order = (ids) => send(`/api/task-agents/${state.id}/queue`, { tasks: ids }, ids.map((id) => state.queue.find((t) => t.id === id)).filter(Boolean)); // a held drag may name a task that left
  function move(id, by) {
    const now = state.queue.map((t) => t.id), at = now.indexOf(id);
    if (at < 0 || at + by < 0 || at + by >= now.length) return;
    now.splice(at + by, 0, ...now.splice(at, 1));
    return order(now);
  }

  function row(t, i, all) {
    const li = el("li", "agqueued"), item = el("span", "item"), acts = el("span", "acts");
    li.dataset.drag = "row";
    item.dataset.id = t.id;
    const grip = el("span", "qgrip");
    grip.setAttribute("aria-hidden", "true");
    item.append(grip, el("span", "qplace", `${i + 1}`), el("b", null, names(t)), el("span", "qtitle", t.title), el("small", null, projectName(t.project) ?? ""));
    const up = iconButton("icon", "arrow-up", `Move ${t.id} up`, "Start it sooner", () => move(t.id, -1));
    const down = iconButton("icon", "arrow-down", `Move ${t.id} down`, "Start it later", () => move(t.id, 1));
    up.disabled = i === 0;
    down.disabled = i === all.length - 1;
    acts.append(up, down, iconButton("icon", "x", `Take ${t.id} out of the queue`, "Take it out of the queue: nothing starts, the task stays where it is",
      () => send(`/api/tasks/${t.id}/leave-queue`, undefined, state.queue.filter((o) => o.id !== t.id))));
    li.append(item, acts);
    return li;
  }

  const drag = bindReorder(list, (_kind, _parent, moved) => order(moved));
  // a row under a press is not drawn anew: a board's read between the press and its first move would take the row
  // from under the pointer, and the drag or the key's click with it; what came meanwhile is drawn once it lets go
  let pressed = false;
  list.addEventListener("pointerdown", () => { pressed = true; });
  for (const end of ["pointerup", "pointercancel"]) window.addEventListener(end, () => { pressed = false; setTimeout(draw); }); // the click comes before a redraw

  function show(a, queue, opening = false) {
    const previous = state;
    box.hidden = !a;
    if (!a) { state = null; forget(previous); return; }
    let mine = states.get(a.id);
    if (!mine) { mine = { id: a.id, queue, pending: 0, tail: Promise.resolve(), error: "" }; states.set(a.id, mine); }
    if (mine !== state || opening) say(mine.error);
    state = mine;
    forget(previous);
    if (!mine.pending) mine.queue = queue;
    draw();
  }
  function draw() {
    if (!state) return;
    const queue = state.queue, now = JSON.stringify([state.id, queue]);
    if (now === drawn || pressed || drag.dragging()) return;
    drawn = now;
    list.replaceChildren(...queue.map(row));
    none.textContent = queue.length ? (queue.length > 1 ? "The first starts when a slot frees. Drag a task, or use its arrows, to change the order." : "It starts when a slot frees.")
      : "No task waits for this agent. A task run while it is at its limit can be added here.";
  }
  return { show, at: () => head.scrollIntoView({ block: "nearest" }) };
}
