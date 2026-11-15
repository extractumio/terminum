// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Batches… of the INTEGRATION column (terminum-integration.md AC-193): a project's batches newest first, 100 at a
// time: number, tasks, the target's and the candidate's commits, its integration task, rounds, result, how long its
// check ran and its log's key; the head says how many of the last 20 were green. A task's ID opens its window.
import { el } from "./listparts.js";
import { $, api, button, say } from "./kanbanutil.js";

const minutes = (s) => (s == null ? "—" : `${Math.max(1, Math.round(s / 60))} min`);

export function batchesWindow({ openTask }) {
  const box = $("batcheswin"), list = $("batcheslist"), more = $("batchesmore");
  let project = null, oldest = null;
  $("batchesclose").onclick = () => box.close();

  const ids = (tasks) => tasks.flatMap((id, i) => [i ? ", " : "", button("linklike", id, () => openTask(id))]);

  function row(b) {
    const li = el("li", "batchrow");
    const head = el("div", "batchline");
    head.append(el("strong", null, `Batch ${b.number}`), " · ", el("span", `batchresult ${b.result.startsWith("green") ? "good" : "bad"}`, b.result));
    if (b.task) head.append(" · ", button("linklike", b.task, () => openTask(b.task)));
    const tasks = el("div", "batchtasks");
    tasks.append(...ids(b.tasks));
    li.append(head, tasks, el("small", null, `${b.base} → ${b.candidate ?? "—"} · ${b.rounds === 1 ? "1 round" : `${b.rounds} rounds`} · check ${minutes(b.checked_seconds)} · ${b.log}`));
    return li;
  }

  async function read(before) {
    try {
      const q = before ? `?before=${before}` : "";
      const got = await api(`/api/integration/project/${encodeURIComponent(project)}/batches${q}`);
      const rows = got.batches.map(row);
      if (before) list.append(...rows); else list.replaceChildren(...rows);
      oldest = got.batches.at(-1)?.number ?? oldest;
      more.hidden = !got.more;
      if (!before) {
        $("batchesmeta").textContent = got.of ? `green ${got.green} of the last ${got.of}` : "";
        $("batchesnone").hidden = got.batches.length > 0;
      }
    } catch (e) { say(e.message); }
  }
  more.onclick = () => read(oldest);

  /** Opens for the project `id`. */
  function open(id) {
    project = id;
    oldest = null;
    list.replaceChildren();
    if (!box.open) box.showModal();
    read(null);
  }
  return { open };
}
