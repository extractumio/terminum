// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A batch's integration task's window (terminum-integration.md AC-194): its state in words and, when a human decides
// (a red round, a batch not published, a failed After), the keys: Check again, Return… a task with why, Return all,
// Publish again, Close…. terminumd says what it cannot do, here.
import { askConfirm, askName } from "./dialogs.js";
import { el } from "./listparts.js";
import { api, button, showError } from "./kanbanutil.js";

const WORDS = { "check-again": "Check again", "publish-again": "Publish again", "return-all": "Return all", close: "Close…" };

/** The batch's part of `t`'s window, filled once terminumd answers; none for a task of no batch. */
export function batchPart(t, refresh) {
  if (!t.id.startsWith("I_")) return null;
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
