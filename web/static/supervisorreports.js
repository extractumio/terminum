// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-240, AC-242, AC-243: the project's decisions, scheduled review and code digest.
import { oneAtATime } from "./dialogs.js";
import { api, button, renderInto, showError, stamp } from "./kanbanutil.js";
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
    const preview = el("div", "markdown");
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
  const error = el("p", "merr");
  error.hidden = true;
  error.setAttribute("role", "alert");
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
  const report = el("div", "markdown");
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

export function reportCards(data, path) {
  const setup = data.review_setup_error && el("p", "merr", data.review_setup_error);
  if (setup) setup.setAttribute("role", "alert");
  return [decisionList(data.decisions ?? []), setup, weekly(data.review, path), exceptionDigest(data.digest)].filter(Boolean);
}
