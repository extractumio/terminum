// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-240, AC-242, AC-243: the project's decisions, scheduled review and code digest.
import { oneAtATime } from "./dialogs.js";
import { alertLine, api, button, renderInto, showError, stamp } from "./kanbanutil.js";
import { el } from "./listparts.js";
import { taskUrl } from "./tasklink.js";
import { actions, part, stack } from "./supervisorfields.js";

const VERDICTS = [["useful", "Useful"], ["not_useful", "Not useful"], ["wrong", "Wrong"]];
const number = new Intl.NumberFormat(undefined, { maximumSignificantDigits: 6 });

function taskLink(task, words) {
  const link = el("a", "textkey", words);
  link.href = taskUrl(task);
  return link;
}

function decisionList(decisions) {
  if (!decisions.length) return null;
  const items = decisions.map(({ task, problem, question }) => {
    const preview = el("div", "markdown wrap");
    renderInto(preview, question, { comment: true });
    const title = task ? taskLink(task, task) : el("h3", null, `Register problem ${problem}`);
    return part("li", "sv-stack", title, preview);
  });
  return part("details", "sv-panel", el("summary", "sv-touch", `Decisions waiting (${items.length})`), part("ol", "sv-stack sv-evidence", ...items));
}

function verdictItem(review, item, path) {
  const keys = actions();
  const subject = el("p", null, `${item.metric} · ${item.detector}`);
  keys.setAttribute("role", "group");
  keys.setAttribute("aria-label", `Verdict for ${item.metric}`);
  const error = alertLine();
  const paint = (verdict) => [...keys.children].forEach((key) => key.setAttribute("aria-pressed", String(key.value === verdict)));
  const press = oneAtATime(keys, async (verdict) => {
    showError(error, "");
    try {
      const saved = await api(`${path}/verdicts`, {
        method: "POST", body: { task: review.batch_task ?? review.task, fingerprint: item.fingerprint, verdict },
      });
      paint(saved.verdict);
    } catch (e) { showError(error, e.message); }
  });
  for (const [value, words] of VERDICTS) {
    const key = button("textkey", words, () => press(value));
    key.value = value;
    keys.append(key);
  }
  paint(item.verdict);
  return part("li", "sv-stack", subject, keys, error);
}

function weekly(review, path) {
  if (!review) return el("p", "msay", "No weekly review published.");
  const report = el("div", "markdown wrap");
  renderInto(report, review.result, { comment: true });
  const batch = review.items.length
    ? part("ol", "sv-stack sv-evidence", ...review.items.map((item) => verdictItem(review, item, path)))
    : el("p", "msay", "No items to rate in this review.");
  return part("details", "sv-panel", el("summary", "sv-touch", `Weekly review · ${stamp(review.at)}`),
    stack(actions(taskLink(review.task, `Open ${review.task} in Routines`)), report, batch));
}

function exceptionDigest(digest) {
  if (!digest) return null;
  const items = digest.entries.map(({ fingerprint, metric, measure, action }) => {
    const link = el("a", "textkey", "Findings");
    const query = new URLSearchParams(location.search);
    query.set("signal", fingerprint);
    link.href = `/supervisor/?${query}#signal-${fingerprint}`;
    return part("li", null, `${metric}: ${number.format(measure.value)} vs ${number.format(measure.baseline)} baseline. ${action}. `, link);
  });
  return part("details", "sv-panel", el("summary", "sv-touch", `Exception digest · ${stamp(digest.at)}`),
    stack(part("ul", "sv-evidence", ...items),
      el("p", "msay", digest.last_ok == null ? "Collector has no successful run recorded." : `Collector last ran ${stamp(digest.last_ok)}.`)));
}

function configDiff(change) {
  const fields = new Set([...Object.keys(change.previous ?? {}), ...Object.keys(change.next ?? {})]);
  const display = (value) => value == null ? "Default" : typeof value === "object" ? JSON.stringify(value) : String(value);
  return [...fields].filter((key) => JSON.stringify(change.previous?.[key]) !== JSON.stringify(change.next?.[key]))
    .map((key) => `${key.replaceAll("_", " ")} · ${display(change.previous?.[key])} → ${display(change.next?.[key])}`);
}

function configuration(data, path, refresh) {
  const changes = data.config_changes ?? [], sunset = data.sunset ?? [];
  if (!changes.length && !sunset.length) return null;
  const items = changes.map((change) => {
    const status = change.reverted ? "Reverted" : change.applied ? "Applied" : change.decision ?? "Awaiting a decision";
    const error = alertLine(), keys = actions();
    const press = oneAtATime(keys, async (operation) => {
      showError(error, "");
      try {
        await api(`${path}/config/${change.id}/${operation}`, { method: "POST", body: {} });
        await refresh();
      } catch (e) { showError(error, e.message); }
    });
    if (change.task) keys.append(taskLink(change.task, `Open ${change.task}`));
    if (change.applied && !change.reverted) keys.append(button("textkey", "Revert change", () => press("revert")));
    if (change.promote) keys.append(button("textkey", "Promote to Live", () => press("promote")));
    return part("li", "sv-stack", el("h3", null, `Change ${change.id} · ${change.detector}`),
      el("p", "msay", `${status} · ${change.class} · ${stamp(change.at)}`),
      ...configDiff(change).map((line) => el("p", "wrap", line)), el("p", "wrap", change.replay), keys, error);
  });
  for (const { detector, removal } of sunset) {
    items.push(el("li", null, `${detector} · ${removal ? "Proposed for removal" : "Weekly review only"}`));
  }
  const panel = part("details", "sv-panel", el("summary", "sv-touch", "Configuration changes"),
    part("ul", "sv-stack sv-evidence", ...items));
  panel.id = "sv-config-changes";
  panel.open = document.getElementById(panel.id)?.open ?? false;
  return panel;
}

export function reportCards(data, path, refresh) {
  const setup = data.review_setup_error && el("p", "merr", data.review_setup_error);
  if (setup) setup.setAttribute("role", "alert");
  return [decisionList(data.decisions ?? []), setup, weekly(data.review, path), configuration(data, path, refresh), exceptionDigest(data.digest)].filter(Boolean);
}
