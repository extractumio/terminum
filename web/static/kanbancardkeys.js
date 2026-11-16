// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The integration queue's keys where the user works on the board (terminum-integration.md AC-193): a card's menu, by
// its ⋯ key or a right click, holds Integrate on an IN PROGRESS task of a project that integrates, Take out on a queued
// one, and on a deciding batch's cards its keys (Return… with a line of why, Check again, Return all, Publish again);
// an IN PROGRESS card dropped on INTEGRATION (its column or its tab) is queued, a queued one dropped on IN PROGRESS is
// taken out; other columns take no drop (terminum-kanban.md AC-107). Refused, a toast says terminumd's words.
import { askConfirm, askName } from "./dialogs.js";
import { inside, placeModal } from "./fabpane.js";
import { icon } from "./icons.js";
import { el, iconButton, pointerKey } from "./listparts.js";
import { $, api, say } from "./kanbanutil.js";
import { BATCH_WORDS, pressBatch } from "./kanbanbatch.js";

const integrates = (meta, t) => meta.projects.some((p) => p.id === t.project && p.integration);
/** Integrate: an IN PROGRESS task of a project that integrates, never a batch's own ("I_", AC-187). */
const canQueue = (meta, t) => t.status === "IN PROGRESS" && !t.id.startsWith("I_") && integrates(meta, t);
/** Take out: a queued task, never a batch's integration task: it ends with its batch. */
const canTakeOut = (t) => t.status === "INTEGRATION" && !t.queue?.head;

const queue = async (t) => (await api(`/api/integration/task/${encodeURIComponent(t.id)}/queue`, { method: "POST", body: {} }))?.said;
const takeOut = (t) => api(`/api/integration/task/${encodeURIComponent(t.id)}/out`, { method: "POST", body: {} }).then(() => `${t.id} is IN PROGRESS again.`);

/** `getMeta()`: the board's meta; `dragging()`: the tasks of a drag; `reload()`: the columns read again. */
export function cardKeys({ getMeta, dragging, reload }) {
  const box = $("cardmenu");
  let back = null;
  const hide = () => {
    if (!box.open) return;
    box.close();
    if (back?.isConnected) back.focus({ preventScroll: true });
  };
  /** One key's press: what terminumd says, kept or refused, in a toast; the columns read again. */
  const run = (go) => async () => {
    hide();
    try {
      const said = await go();
      if (said) say(said);
    } catch (e) { say(e.message); }
    reload();
  };

  /** The menu's items for `t`, as [label, icon, press]. */
  function items(t) {
    const out = [], q = t.queue, keys = q?.batch_id ? q.keys ?? [] : [];
    if (canQueue(getMeta(), t)) out.push(["Integrate", "arrow-right-left", run(() => queue(t))]);
    if (canTakeOut(t)) out.push(["Take out", "undo-2", run(() => takeOut(t))]);
    const send = (key, body) => pressBatch(q.batch_id, key, body);
    for (const key of keys) {
      if (key === "return" && !q.head) {
        out.push([`Return ${t.id}…`, "undo-2", () => { hide(); askName(`Why does ${t.id} go back? Its agent reads it.`, "", "Return", 600, async (why) => { say(await send(key, { task: t.id, why })); reload(); }); }]);
      } else if (key === "return-all") {
        out.push([BATCH_WORDS[key], "undo-2", async () => { hide(); if (await askConfirm(`Return batch ${q.batch}'s tasks to their agents?`, "Return all")) run(() => send(key))(); }]);
      } else if (key === "check-again" || key === "publish-again") {
        out.push([BATCH_WORDS[key], "history", run(() => send(key))]);
      }
    }
    return out;
  }

  function open(t, x, y) {
    const list = items(t);
    if (!list.length) return;
    const head = el("div", "mhead"), close = el("button", null, "×");
    close.type = "button";
    close.title = "Close this menu";
    close.setAttribute("aria-label", "Close this menu");
    close.addEventListener("click", hide);
    head.append(el("span", null, `${t.id} · ${t.title}`), close);
    box.replaceChildren(head, ...list.map(([label, mark, press]) => {
      const b = pointerKey(el("button", null, label));
      b.type = "button";
      b.setAttribute("role", "menuitem");
      b.prepend(icon(mark, 16));
      b.addEventListener("click", (e) => { e.stopPropagation(); press(); });
      return b;
    }));
    back = document.activeElement;
    box.showModal();
    if (!placeModal(box, hide)) {                  // a computer's: at the press; a phone's: a sheet
      box.style.left = `${inside(x, box.offsetWidth, innerWidth)}px`;
      box.style.top = `${inside(y, box.offsetHeight, innerHeight)}px`;
    }
    box.querySelector("button[role=menuitem]")?.focus({ preventScroll: true });
  }
  box.addEventListener("click", (e) => {           // a press on the shade closes it and reaches nothing under it
    if (e.target !== box) return;
    const r = box.getBoundingClientRect();
    if (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom) hide();
  });
  box.addEventListener("cancel", (e) => { e.preventDefault(); hide(); });
  box.addEventListener("keydown", (e) => {         // ↑ ↓ through its items, as a menu
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const all = [...box.querySelectorAll("button[role=menuitem]")], i = all.indexOf(document.activeElement);
    all[(i + (e.key === "ArrowDown" ? 1 : all.length - 1)) % all.length]?.focus();
  });

  /** On card `c` of `t`: its ⋯ key when it has items, and its right click. */
  function onCard(c, t, head) {
    if (!items(t).length) return;
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

  /** What a drop of the dragged tasks on `column` does, or null where it takes none. */
  function dropOn(column) {
    const tasks = dragging();
    if (!tasks?.length) return null;
    if (column === "INTEGRATION" && tasks.every((t) => canQueue(getMeta(), t))) return () => tasks.map((t) => () => queue(t));
    if (column === "IN PROGRESS" && tasks.every(canTakeOut)) return () => tasks.map((t) => () => takeOut(t));
    return null;
  }
  /** INTEGRATION and IN PROGRESS, by their column and their tab, light under a drag they take; one at a time, each
   *  said: a refused one moves nothing. */
  function drops(columns) {
    for (const column of ["INTEGRATION", "IN PROGRESS"]) {
      const { col, tab } = columns.get(column);
      for (const zone of [col, tab]) {
        zone.addEventListener("dragover", (e) => {
          if (!dropOn(column)) return;
          e.preventDefault();
          zone.classList.add("over");
        });
        zone.addEventListener("dragleave", (e) => { if (!zone.contains(e.relatedTarget)) zone.classList.remove("over"); });
        zone.addEventListener("drop", async (e) => {
          const presses = dropOn(column);
          if (!presses) return;
          e.preventDefault();
          zone.classList.remove("over");
          const said = [];
          for (const press of presses()) {
            try { said.push(await press()); } catch (no) { said.push(no.message); }
          }
          say(said.filter(Boolean).join(" · "));
          reload();
        });
      }
    }
    document.addEventListener("dragend", () => document.querySelectorAll(".col.over, #colswitch .over").forEach((z) => z.classList.remove("over")));
  }
  return { onCard, drops };
}
