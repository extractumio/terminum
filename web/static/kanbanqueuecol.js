// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The INTEGRATION column (terminum-integration.md AC-193): its head holds Pause (Resume) and Batches…, no sort keys
// (its order is the queue's); on top, the batch in work as one framed group, its head the integration task's line,
// "I_API-30 · batch 7 · checking · 12 min" (a press on the words shows the step's window, on the ID its task), then
// its cards, each "in batch 7"; under it the queue in its order, each card its place or what it waits for; a paused
// queue or one waiting for another project says so. A returned task's card in IN PROGRESS says "returned: …".
// terminumd says every state in words; the board's draw lays the cards, this arranges them after.
import { showIntegration } from "./kanbanintegration.js";
import { el } from "./listparts.js";
import { api, button, say } from "./kanbanutil.js";
import { batchesWindow } from "./kanbanbatches.js";

const minutesSince = (at) => Math.max(0, Math.floor((Date.now() / 1000 - at) / 60));

export function integrationColumn({ openTask, openSession }) {
  const batches = batchesWindow({ openTask });
  let parts = [], asked = 0, keys = null, chosenNow = [];

  /** Pause and Batches… in the column's head, once; its sort keys hidden. */
  function headKeys(part) {
    if (keys) return keys;
    const head = part.col.querySelector(".colhead");
    const pause = button("textkey colkey", "Pause", async () => {
      const paused = !parts.every((p) => p.paused);
      try {
        for (const p of parts) await api(`/api/integration/project/${encodeURIComponent(p.project)}/pause`, { method: "POST", body: { paused } });
        refresh(part);
      } catch (e) { say(e.message); }
    });
    pause.title = "Pause: no new batch starts; the one in work goes on";
    const list = button("textkey colkey", "Batches…", () => (parts.length ? batches.open(parts[0].project) : say("No project with integration on is shown.")));
    list.title = "The project's past batches and their results";
    head.querySelector(".colsort")?.setAttribute("hidden", "");
    head.querySelector(".count").before(pause, list);
    keys = { pause, list };
    return keys;
  }

  const cardIn = (root, id) => root.querySelector(`.card[data-id="${CSS.escape(id)}"]`);
  const mark = (c, text) => c?.querySelector(".chead")?.append(el("span", "qmark", text));

  /** The column's cards laid as the queue stands: notes, the batch's group, the queue in order, any other card after. */
  function arrange(part) {
    const list = part.list;
    list.querySelectorAll(".batchgroup").forEach((g) => g.replaceWith(...g.querySelectorAll(".card")));
    list.querySelectorAll(".qmark, .colnote").forEach((x) => x.remove());
    const top = [];
    for (const p of parts) {
      const note = p.paused ? p.paused_by : p.waits;
      if (note) top.push(el("p", "colnote", note));
      if (p.batch) {
        const b = p.batch, group = el("div", "batchgroup"), head = el("div", "batchhead");
        const id = b.task ? button("linklike", b.task, () => openTask(b.task)) : el("span", null, "batch");
        const words = button("linklike batchwords", `batch ${b.number} · ${b.words} · ${minutesSince(b.since)} min`, () => (b.window ? openSession(b.window) : b.task && openTask(b.task)));
        if (b.window) words.title = "Show its window";
        head.append(id, el("span", "sep", " · "), words);
        group.append(head);
        const own = b.task && cardIn(list, b.task);
        if (own) group.append(own);
        for (const t of b.tasks) {
          const c = cardIn(list, t);
          if (c) { mark(c, `in batch ${b.number}`); group.append(c); }
        }
        top.push(group);
      }
      for (const q of p.queue) {
        const c = cardIn(list, q.id);
        if (!c) continue;
        mark(c, q.waits_for ? `waits for ${q.waits_for}` : q.verifying ? "verifying" : q.place);
        top.push(c);
      }
    }
    list.prepend(...top);
    const working = document.querySelector('.col[data-status="IN PROGRESS"]');
    working?.querySelectorAll(".qmark").forEach((x) => x.remove());
    for (const p of parts) for (const [id, words] of Object.entries(p.returned ?? {})) if (working) mark(cardIn(working, id), words);
  }

  /** terminumd's parts for the projects shown, then the column arranged by them; the newest asked is the one shown. */
  async function refresh(part) {
    const mine = ++asked;
    try {
      const got = await api(`/api/integration/column?projects=${encodeURIComponent(chosenNow.join(","))}`);
      if (mine !== asked) return;
      parts = got.projects;
      headKeys(part).pause.textContent = parts.length && parts.every((p) => p.paused) ? "Resume" : "Pause";
      arrange(part);
    } catch (e) { if (mine === asked) say(e.message); }
  }

  /** After the board drew its columns: the column shown or hidden (AC-193), and arranged when it stands. */
  function draw(part, meta, chosen, n) {
    showIntegration(part, meta, chosen, n);
    chosenNow = chosen;
    if (!part.col.hidden) refresh(part);
  }
  return { draw };
}
