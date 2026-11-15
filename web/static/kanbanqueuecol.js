// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The INTEGRATION column's head (terminum-integration.md AC-193): Pause (Resume) and Batches… for the shown projects
// that integrate, and what holds their queue, paused by whom or waiting for what, in terminumd's words. Its cards are
// the board's, in the queue's order, each saying what terminumd says of it (kanban.js's card).
import { showIntegration } from "./kanbanintegration.js";
import { el } from "./listparts.js";
import { api, button, say } from "./kanbanutil.js";
import { batchesWindow } from "./kanbanbatches.js";

export function integrationColumn({ openTask, reload }) {
  const batches = batchesWindow({ openTask });
  let shown = [];
  const box = el("span", "queuekeys"), note = el("span", "colnote");
  const allPaused = () => shown.length > 0 && shown.every((p) => p.queue?.paused);
  const pause = button("textkey", "Pause", async () => {
    const paused = !allPaused();
    const failed = [];
    for (const p of shown) {
      try { await api(`/api/integration/project/${encodeURIComponent(p.id)}/pause`, { method: "POST", body: { paused } }); }
      catch (e) { failed.push(`${p.name}: ${e.message}`); }
    }
    if (failed.length) say(failed.join(" · "));
    reload();   // what holds is shown either way
  });
  const list = button("textkey", "Batches…", () => (shown.length ? batches.open(shown[0].id) : say("No project with integration on is shown.")));
  list.title = "The project's past batches and their results";
  box.append(pause, list, note);

  /** The keys and the note, from the shown projects' queues as the board's meta says them. */
  function draw(part, meta, chosen, n) {
    showIntegration(part, meta, chosen, n);
    shown = meta.projects.filter((p) => p.integration && (!chosen.length || chosen.includes(p.id)));
    const paused = allPaused();
    pause.textContent = paused ? "Resume" : "Pause";
    pause.title = paused ? "Resume: batches start again" : "No new batch starts; the one in work goes on";
    note.textContent = shown.map((p) => p.queue?.note).filter(Boolean).join(" · ");
    note.hidden = !note.textContent;
  }
  return { keys: () => box, draw };
}
