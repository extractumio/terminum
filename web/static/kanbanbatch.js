// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A batch's integration task's window (terminum-integration.md AC-194): its state in words and, when a human decides
// (a red round, a batch not published, a failed After), the keys: Check again, Return… a task with why, Return all,
// Publish again, Close…. terminumd says what it cannot do, here.
import { askConfirm, askName } from "./dialogs.js";
import { el } from "./listparts.js";
import { api, button, row, showError } from "./kanbanutil.js";
import { sessionUrl } from "./tasklink.js";

const BATCH_WORDS = { "check-again": "Check again", "publish-again": "Publish again", "return-all": "Return all", close: "Close…" };
/** A human's key on batch `id` (its store ID); answers terminumd's words for what it did, or throws its refusal. */
export const pressBatch = async (id, key, body) => (await api(`/api/integration/batch/${id}/${key}`, { method: "POST", body: body ?? {} })).said;

/** A human's Integrate ("integrate") or Take out ("take-out") of task `id` (AC-193); answers what terminumd did. */
export async function pressQueue(id, key) {
  const out = key === "take-out", r = await api(`/api/integration/task/${encodeURIComponent(id)}/${out ? "out" : "queue"}`, { method: "POST", body: {} });
  return out ? `${id} is IN PROGRESS again.` : r.said;
}

/** Batch `b`'s ({number, count?}) key `key` as [label, press], Return… for task `task`, in the integration task's window
 *  and on its group's cards alike. `send(key, body)` presses it: a line of why is asked in the page's own dialog, where
 *  a refusal's words stay; any other refusal goes to `fail(e)`. */
export function batchKey(b, key, task, send, fail) {
  const ask = (text, yes, body) => () => askName(text, "", yes, 600, (why) => send(key, { ...body, why }));
  const press = () => send(key).catch(fail);
  if (key === "return") return [`Return ${task}…`, ask(`Why does ${task} go back? Its agent reads it.`, "Return", { task })];
  if (key === "close") return [BATCH_WORDS[key], ask(`Close batch ${b.number}: why?`, "Close")];
  if (key === "return-all") return [BATCH_WORDS[key], async () => { if (await askConfirm(`Return batch ${b.number}'s ${b.count ? `${b.count} ` : ""}tasks to their agents?`, "Return all")) press(); }];
  return [BATCH_WORDS[key], press];
}

/** A task of a project whose integration is on: Integrate while it is in work, Take out while it is queued (AC-193);
 *  terminumd says what it refuses, here. */
function queueKeys(t, refresh, meta) {
  const on = meta?.projects?.some((p) => p.id === t.project && p.integration);
  if (!on || !["IN PROGRESS", "INTEGRATION"].includes(t.status)) return null;
  const wrap = el("section", "tasksec queuekeys"), error = el("p", "error");
  error.setAttribute("role", "alert");
  error.hidden = true;
  const queued = t.status === "INTEGRATION";
  const key = button("textkey", queued ? "Take out" : "Integrate", async () => {
    try {
      await pressQueue(t.id, queued ? "take-out" : "integrate");
      showError(error, "");
      refresh();
    } catch (e) { showError(error, e.message); }
  });
  key.title = queued ? "Take it out of the integration queue: it is IN PROGRESS again" : "Queue the commit its session's folder stands at for integration";
  wrap.append(row(key), error);
  return wrap;
}

/** The batch's part of `t`'s window, filled once terminumd answers; for a task of no batch, its queue's keys. */
export function batchPart(t, refresh, meta) {
  if (!t.id.startsWith("I_")) return queueKeys(t, refresh, meta);
  const wrap = el("section", "tasksec batchpart"), line = el("p", "batchstate", "Reading the batch…"), keys = el("div", "actions batchkeys"), error = el("p", "error");
  error.setAttribute("role", "alert");
  error.hidden = true;
  wrap.append(line, keys, error);
  api(`/api/integration/task/${encodeURIComponent(t.id)}`).then((b) => {
    line.textContent = `Batch ${b.number} · ${b.state} · round ${b.round}${b.why ? ` · ${b.why}` : ""} · ${b.tasks.map((x) => `${x.id} ${x.status}`).join(", ")}`;
    // its step's window, shown as a session is (AC-193)
    if (b.window) keys.append(button("textkey", "Show its window", () => { location.href = sessionUrl(b.window); }));
    const send = async (key, body) => {
      await pressBatch(b.batch, key, body);
      showError(error, "");
      refresh();
    };
    const fail = (e) => showError(error, e.message), shown = { number: b.number, count: b.tasks.length };
    for (const key of b.keys) {
      for (const x of key === "return" ? b.tasks : [null]) keys.append(button("textkey", ...batchKey(shown, key, x?.id, send, fail)));
    }
  }).catch((e) => showError(error, e.message));
  return wrap;
}
