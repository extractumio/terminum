// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-223: a single project's independent indicator verdicts and typed evidence.
import { boardFor } from "./sections.js";
import { api, loadMeta } from "./msgpage.js";
import { $, button, showError, stamp } from "./kanbanutil.js";
import { el } from "./listparts.js";
import { boardUrl, taskUrl } from "./tasklink.js";
import { incidentHistory } from "./supervisorincidents.js";
import { actions, part, stack } from "./supervisorfields.js";

const project = new URLSearchParams(location.search).get("project");
const path = `/api/task-projects/${encodeURIComponent(project)}/supervisor`;
const more = $("sv-more"), refresh = $("sv-refresh"), error = $("sv-error");
const numbers = new Intl.NumberFormat(undefined, { maximumSignificantDigits: 6 });
let next = null;
const note = (words) => el("p", "msay", words);

function folding(title, contents) {
  const panel = part("details", "sv-panel", el("summary", "sv-touch", title));
  const reveal = () => {
    if (!panel.open) return;
    panel.append(contents());
    panel.removeEventListener("toggle", reveal);
  };
  panel.addEventListener("toggle", reveal);
  return panel;
}

function indicator({ name, verdict, guard, metrics }) {
  return folding(`${name} — ${verdict}`, () => {
    const entries = metrics.map((m) => {
      const state = m.missing ? `Not measured yet (${m.missing})` : m.verdict ?? "Context";
      const result = el("dd", null, state);
      const values = [
        m.before != null && `Prior ${numbers.format(m.before)}`,
        m.current != null && `Latest ${numbers.format(m.current)}`,
        !m.missing && `Weekly noise ${numbers.format(m.noise)}`,
      ].filter(Boolean);
      if (values.length) result.append(note(values.join(" · ")));
      return part("div", "sv-reading", el("dt", null, m.name), result);
    });
    return stack(note(guard), part("dl", "sv-readings", ...entries));
  });
}

function reference(id) {
  if (!/^[A-Za-z0-9_-]+$/.test(id)) return document.createTextNode(id);
  const link = el("a", "textkey", id);
  link.href = taskUrl(id);
  return link;
}

function evidence({ table, id }) {
  const label = table.replaceAll("_", " ");
  if (table === "tasks") return part("li", null, "Task ", reference(id));
  if (table === "task_links") return part("li", null, "Task link ", reference(id.task), " to ", reference(id.other));
  const detail = table === "supervisor_heartbeat" ? id.last_ok ? `Last successful check ${stamp(id.last_ok)}` : "No successful check recorded" : String(id);
  return el("li", null, `${label} — ${detail}`);
}

function signal(record) {
  const { detector, metric, subject_id, subject_kind, state, level, count, response, last_seen, measure } = record;
  return folding(`Level ${level} · ${metric} · ${state}`, () => {
    const investigate = button("textkey", "Investigate");
    investigate.disabled = true;
    investigate.setAttribute("aria-describedby", "sv-investigate-note");
    const readings = Object.entries({ Measured: measure.value, Baseline: measure.baseline, Threshold: measure.threshold })
      .map(([label, value]) => part("div", "sv-reading", el("dt", null, label), el("dd", null, numbers.format(value))));
    return stack(
      note(`${detector} / ${subject_kind} ${subject_id}`),
      note(`Last seen ${stamp(last_seen)} · ${count} ${count === 1 ? "occurrence" : "occurrences"} · ${response}`),
      part("dl", "sv-readings", ...readings),
      note(`Measurement window ${stamp(measure.window_start)} – ${stamp(measure.window_end)}`),
      el("h3", null, "Evidence"), record.evidence.length ? part("ul", "sv-evidence", ...record.evidence.map(evidence)) : note("No evidence rows retained."),
      actions(investigate));
  });
}

function addSignals(data) {
  $("sv-signals").append(...data.signals.map(signal));
  next = data.next;
  more.hidden = !next;
}

async function readScreen() {
  refresh.disabled = more.disabled = true;
  showError(error, "");
  showError($("sv-more-error"), "");
  try {
    const data = await api(`${path}/screen`);
    $("sv-title").textContent = `${data.project_name} · Supervisor`;
    $("sv-score").replaceChildren(...(data.indicators.length ? data.indicators.map(indicator) : [note("All scoreboard indicators are off.")]));
    $("sv-signals").replaceChildren();
    $("sv-investigate-note").hidden = !data.signals.length;
    if (!data.signals.length) $("sv-signals").append(note("No signals recorded for this project."));
    addSignals(data);
    $("sv-content").hidden = false;
  } catch (e) { showError(error, e.message); }
  finally { refresh.disabled = more.disabled = false; }
}

more.addEventListener("click", async () => {
  refresh.disabled = more.disabled = true;
  const feedback = $("sv-more-error"), list = $("sv-signals"), start = list.children.length;
  showError(feedback, "");
  try {
    addSignals(await api(`${path}/signals?${new URLSearchParams({ after: next })}`));
    list.children[start]?.querySelector("summary")?.focus();
  } catch (e) { showError(feedback, e.message); }
  finally { refresh.disabled = more.disabled = false; }
});
refresh.addEventListener("click", readScreen);

loadMeta().catch((e) => showError($("sv-theme-error"), e.message));
if (project) {
  incidentHistory(project, new URLSearchParams(location.search).get("incident"), refresh);
  boardFor(project);
  $("sv-board").href = boardUrl(project);
  refresh.hidden = false;
  readScreen();
} else showError(error, "Open a project’s Supervisor tab to see its scoreboard and signals.");
