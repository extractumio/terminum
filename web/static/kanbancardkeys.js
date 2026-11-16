// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The integration queue's keys where the user works on the board (terminum-integration.md AC-193), each as terminumd
// lists it on the card (its `queue.keys`): a card's menu, by its ⋯ key or a right click, holds Integrate, Take out and a
// deciding batch's keys (Return… with a line of why, Check again, Return all, Publish again, Close…); a card that may be
// integrated dropped on INTEGRATION (its column or its tab) is queued, one that may be taken out dropped on IN PROGRESS
// is taken out; other columns take no drop (terminum-kanban.md AC-107). Refused, a toast says terminumd's words.
import { iconButton } from "./listparts.js";
import { $, say } from "./kanbanutil.js";
import { batchKey, pressBatch, pressQueue } from "./kanbanbatch.js";
import { dropLit } from "./droplit.js";
import { menuBox } from "./menubox.js";

const QUEUE = { integrate: "Integrate", "take-out": "Take out" };
const MARKS = { integrate: "arrow-right-left", "take-out": "undo-2", return: "undo-2", "return-all": "undo-2", "check-again": "history", "publish-again": "history", close: "x" };
/** What a drop on each column presses, by the key a card must hold. */
const DROPS = { INTEGRATION: "integrate", "IN PROGRESS": "take-out" };
const keysOf = (t) => t.queue?.keys ?? [];

/** `dragging()`: the tasks of a drag; `reload()`: the columns read again. */
export function cardKeys({ dragging, reload }) {
  const menu = menuBox($("cardmenu"), MARKS);
  /** What terminumd says it did, or its refusal, in a toast; the columns read again either way. */
  const told = async (go) => {
    try { say(await go()); } catch (e) { say(e.message); }
    reload();
  };

  /** The menu's items for `t`, as [label, action, press]. */
  function items(t) {
    const q = t.queue;
    // a batch's key said as its window says it; Return…'s refusal stays in its dialog, any other goes to a toast
    const send = async (key, body) => { say(await pressBatch(q.batch_id, key, body)); reload(); }, fail = (e) => { say(e.message); reload(); };
    return keysOf(t).filter((key) => !(key === "return" && q.head)).map((key) => {
      const [label, press] = key in QUEUE ? [QUEUE[key], () => told(() => pressQueue(t.id, key))] : batchKey({ number: q.batch }, key, t.id, send, fail);
      return [label, key, press];
    });
  }

  function open(t, x, y) {
    menu.fill(`${t.id} · ${t.title}`, ...items(t).map(([label, action, press]) => menu.button(label, action, () => { menu.hide(); press(); })));
    menu.open(x, y);
  }

  /** On card `c` of `t`: its ⋯ key and its right click, where terminumd lists a key for it. */
  function onCard(c, t, head) {
    if (!keysOf(t).length) return;
    head.append(iconButton("icon cmenu", "ellipsis", `${t.id}'s menu`, "Integrate, Take out and the batch's keys", (b) => {
      const r = b.getBoundingClientRect();
      open(t, r.left, r.bottom);
    }));
    c.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      const r = c.getBoundingClientRect(), keys = !e.clientX && !e.clientY;
      open(t, keys ? r.left + 24 : e.clientX, keys ? r.top + 24 : e.clientY);
    });
  }

  /** The dragged tasks a drop on `column` takes: every one holding its key, else none. */
  function taken(column) {
    const tasks = dragging(), key = DROPS[column];
    return tasks?.length && tasks.every((t) => keysOf(t).includes(key)) ? tasks : null;
  }
  /** INTEGRATION and IN PROGRESS, by their column and their tab, light under a drag they take; the tasks go one at a
   *  time, each said: a refused one moves nothing. */
  function drops(columns) {
    for (const [column, key] of Object.entries(DROPS)) {
      const { col, tab } = columns.get(column);
      for (const zone of [col, tab]) {
        const lit = dropLit((on) => zone.classList.toggle("over", on));   // gone once the drag ends, however it ends
        zone.addEventListener("dragover", (e) => {
          if (!taken(column)) return;
          e.preventDefault();
          lit(true);
        });
        zone.addEventListener("dragleave", (e) => { if (!zone.contains(e.relatedTarget)) lit(false); });
        zone.addEventListener("drop", async (e) => {
          const tasks = taken(column);
          if (!tasks) return;
          e.preventDefault();
          lit(false);
          const said = [];
          for (const t of tasks) said.push(await pressQueue(t.id, key).catch((no) => no.message));
          say(said.join(" · "));
          reload();
        });
      }
    }
  }
  return { onCard, drops };
}
