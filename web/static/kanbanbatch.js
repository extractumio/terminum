// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A batch's integration task's window (terminum-integration.md AC-194): its state in words and, when a human decides
// (a red round, a batch not published, a failed After), the keys: Check again, Return… a task with why, Return all,
// Publish again, Close…. terminumd says what it cannot do, here.
import { askConfirm, askName } from "./dialogs.js";
import { el } from "./listparts.js";
import { api, button, row, showError } from "./kanbanutil.js";
import { sessionUrl } from "./tasklink.js";

const WORDS = { "check-again": "Check again", "publish-again": "Publish again", "return-all": "Return all", close: "Close…" };

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
      await api(`/api/integration/task/${encodeURIComponent(t.id)}/${queued ? "out" : "queue"}`, { method: "POST", body: {} });
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
  const send = async (b, key, body) => {
    await api(`/api/integration/batch/${b.batch}/${key}`, { method: "POST", body: body ?? {} });
    showError(error, "");
    refresh();
  };
  const press = (b, key, body) => async () => { try { await send(b, key, body); } catch (e) { showError(error, e.message); } };
  /** A line of why, asked in the page's own dialog: refused, terminumd's words stay in it. */
  const why = (b, key, text, yes, body) => () => askName(text, "", yes, 600, (words) => send(b, key, { ...body, why: words }));
  api(`/api/integration/task/${encodeURIComponent(t.id)}`).then((b) => {
    line.textContent = `Batch ${b.number} · ${b.state} · round ${b.round}${b.why ? ` · ${b.why}` : ""} · ${b.tasks.map((x) => `${x.id} ${x.status}`).join(", ")}`;
    // its step's window, shown as a session is (AC-193)
    if (b.window) keys.append(button("textkey", "Show its window", () => { location.href = sessionUrl(b.window); }));
    for (const key of b.keys) {
      if (key === "return") {
        for (const x of b.tasks) keys.append(button("textkey", `Return ${x.id}…`, why(b, "return", `Why does ${x.id} go back? Its agent reads it.`, "Return", { task: x.id })));
      } else if (key === "return-all") {
        keys.append(button("textkey", WORDS[key], async () => { if (await askConfirm(`Return batch ${b.number}'s ${b.tasks.length} tasks to their agents?`, "Return all")) press(b, key)(); }));
      } else if (key === "close") {
        keys.append(button("textkey", WORDS[key], why(b, key, `Close batch ${b.number}: why?`, "Close")));
      } else {
        keys.append(button("textkey", WORDS[key], press(b, key)));
      }
    }
  }).catch((e) => showError(error, e.message));
  return wrap;
}
