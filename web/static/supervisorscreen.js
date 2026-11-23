// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-223: a single project's independent indicator verdicts and typed evidence.
import { oneAtATime } from "./dialogs.js";
import { boardFor } from "./sections.js";
import { api, loadMeta } from "./msgpage.js";
import { $, alertLine, button, showError, stamp } from "./kanbanutil.js";
import { el } from "./listparts.js";
import { boardUrl, taskUrl } from "./tasklink.js";
import { drawRuleOutcomes } from "./supervisoroutcomes.js";
import { drawAgentRates, drawReflections } from "./supervisorreflection.js";
import { reportCards } from "./supervisorreports.js";
import { incidentHistory } from "./supervisorincidents.js";
import { onIncidents } from "./incidents.js";
import { actions, part, stack, text } from "./supervisorfields.js";

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
  if (table === "supervisor_line") return el("li", null, `Channel ${id.channel}: ${id.failure}`);
  const detail = table === "supervisor_heartbeat" ? id.last_ok ? `Last successful check ${stamp(id.last_ok)}` : "No successful check recorded" : String(id);
  return el("li", null, `${label} — ${detail}`);
}

function signal(record, enabled) {
  const { detector, metric, subject_id, subject_kind, state, level, count, response, last_seen, measure } = record;
  const panel = folding(`Level ${level} · ${metric} · ${state}`, () => {
    const feedback = note(record.investigation ?? "");
    feedback.setAttribute("role", "status");
    const error = el("p", "incident-error");
    error.setAttribute("role", "alert");
    error.hidden = true;
    const controls = actions();
    const investigate = button("textkey", "Investigate now", oneAtATime(controls, async () => {
      showError(error, "");
      try {
        const saved = await api(`${path}/signals/${encodeURIComponent(record.fingerprint)}/investigate`, { method: "POST" });
        feedback.textContent = saved.message;
      } catch (e) { showError(error, e.message); }
    }));
    controls.append(investigate);
    investigate.disabled = !enabled;
    if (!enabled) investigate.setAttribute("aria-describedby", "sv-investigate-note");
    const readings = Object.entries({ Measured: measure.value, Baseline: measure.baseline, Threshold: measure.threshold })
      .map(([label, value]) => part("div", "sv-reading", el("dt", null, label), el("dd", null, numbers.format(value))));
    return stack(
      note(`${detector} / ${subject_kind} ${subject_id}`),
      note(`Last seen ${stamp(last_seen)} · ${count} ${count === 1 ? "occurrence" : "occurrences"} · ${response}`),
      part("dl", "sv-readings", ...readings),
      note(`Measurement window ${stamp(measure.window_start)} – ${stamp(measure.window_end)}`),
      el("h3", null, "Evidence"), record.evidence.length ? part("ul", "sv-evidence", ...record.evidence.map(evidence)) : note("No evidence rows retained."),
      controls, feedback, error);
  });
  panel.id = `signal-${record.fingerprint}`;
  if (location.hash === `#${panel.id}`) panel.open = true;
  return panel;
}

const currency = new Intl.NumberFormat(undefined, { style: "currency", currency: "USD" });
const percent = new Intl.NumberFormat(undefined, { style: "percent", maximumFractionDigits: 1 });
const shown = (...nodes) => stack(...nodes.filter(Boolean));
const problemState = (state) => state === "accepted" ? "Accepted as is" : state.replaceAll("_", " ");

function spending({ judgment, fixes }, eligible) {
  return [
    folding("Judgment · last 7 days", () => stack(
      note(`${numbers.format(judgment.tokens)} of ${numbers.format(judgment.limit_tokens)} tokens used · ${numbers.format(judgment.held_tokens)} reserved`),
      note(`Last 24 hours: ${numbers.format(judgment.day_tokens)} of ${numbers.format(judgment.day_limit_tokens)} tokens`),
      note(`Per round: up to ${numbers.format(judgment.round_limit_tokens)} tokens`),
      note(judgment.investigation_left === 0 || judgment.round_limit_tokens === 0
        ? "Judgment limit reached. Code-only work continues."
        : `${numbers.format(judgment.routine_left)} tokens left for routine work · ${numbers.format(judgment.investigation_left)} for level 2 and 3 investigations`))),
    folding("Fixes · last 7 days", () => shown(
      note(`${currency.format(fixes.dollars)} of ${currency.format(fixes.limit_dollars)} used · ${currency.format(fixes.held_dollars)} reserved`),
      note(`Estimated cost per run: ${currency.format(fixes.estimated_dollars_per_run)}`),
      fixes.unpriced_runs ? note(`Actual cost unavailable for ${fixes.unpriced_runs} ${fixes.unpriced_runs === 1 ? "run" : "runs"}.`) : null,
      fixes.exception_dollars ? note(`Charter exceptions: ${currency.format(fixes.exception_dollars)}`) : null,
      note(eligible ? "Supervisor tasks are eligible for Auto-Dispatcher." : "Auto-Dispatcher is set to exclude Supervisor tasks."))),
  ];
}

function rulingControls(p) {
  const input = text("", "area");
  input.required = true;
  input.maxLength = 1000;
  const feedback = alertLine();
  const controls = actions();
  const form = part("form", "sv-stack", part("label", "sv-stack", "Reason to accept this problem as is", input), controls, feedback);
  const accept = el("button", "textkey", "Accept as is");
  controls.append(accept);
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (!input.value.trim()) return showError(feedback, "Enter a reason for this ruling.");
    accept.disabled = true;
    showError(feedback, "");
    try {
      await api(`${path}/problems/${p.id}/ruling`, { method: "POST", body: { reason: input.value } });
      await readScreen();
      const panel = document.querySelector(`[data-problem="${p.id}"]`);
      if (panel) { panel.open = true; panel.querySelector("summary").focus(); }
    } catch (e) { showError(feedback, e.message); }
    finally { accept.disabled = false; }
  });
  return form;
}

function problem(p) {
  const panel = folding(`Impact ${p.impact} · ${p.title} · ${problemState(p.state)}`, () => shown(
    note(p.indicator ? `${percent.format(p.share)} of ${p.indicator.replaceAll("_", " ")} · rank ${numbers.format(p.rank)} · ${p.lens}` : p.lens),
    el("h3", null, "Hypothesis"), note(p.hypothesis),
    el("h3", null, "What would disprove it"), note(p.disproof),
    el("h3", null, "Observation window"),
    part("ul", "sv-evidence", ...p.yardstick.map(y => el("li", null,
      `${y.detector} · ${y.fingerprint} · threshold ${numbers.format(y.threshold)} for ${y.weeks} weeks${y.clear_at == null ? "" : ` · clear since ${stamp(y.clear_at)}`}`))),
    p.fix_options.length ? stack(el("h3", null, "Fix options"),
      part("ul", "sv-evidence", ...p.fix_options.map(f => el("li", null, `${f.layer} · ${f.action}`)))) : null,
    p.tasks.length ? stack(el("h3", null, "Fix tasks"),
      part("ul", "sv-evidence", ...p.tasks.map(id => part("li", null, reference(id))))) : null,
    p.exception ? note("Verified charter breach. Its fix is exempt from the spending limit.") : null,
    p.action_ready === false && p.state !== "resolved" && p.state !== "accepted" ? note("Proposed fixes need a checked finding.") : null,
    p.new_evidence ? note(p.new_evidence) : null,
    p.ruling ? note(`Owner ruling · ${p.ruling}`) : null,
    p.state === "accepted" || p.state === "resolved" ? null : rulingControls(p)));
  panel.dataset.problem = String(p.id);
  return panel;
}

function heldAction(hold) {
  const error = alertLine();
  const keys = actions();
  const release = button("textkey", hold.undo, oneAtATime(keys, async () => {
    showError(error, "");
    try {
      await api(`${path}/holds/${hold.id}/release`, { method: "POST" });
      await readHolds();
      $("sv-holds-title").focus();
    } catch (e) { showError(error, e.message); }
  }));
  const signal = el("a", "textkey", `Signal · ${hold.detector}`);
  signal.href = `?project=${encodeURIComponent(project)}&signal=${encodeURIComponent(hold.signal)}#signal-${hold.signal}`;
  keys.append(signal, release);
  return part("article", "incident-card",
    el("h3", "incident-title", hold.task ? `${hold.label} · ${hold.task}` : hold.label),
    note(hold.reason), keys, error);
}

function drawHolds(holds) {
  $("sv-holds").replaceChildren(...(holds.length ? holds.map(heldAction) : [note("No holds in effect.")]));
}

let holdsRead, holdsAgain = false;
function readHolds() {
  if (holdsRead) { holdsAgain = true; return holdsRead; }
  const error = $("sv-holds-error");
  holdsRead = (async () => {
    do {
      holdsAgain = false;
      showError(error, "");
      try { drawHolds(await api(`${path}/holds`)); }
      catch (e) { showError(error, e.message); }
    } while (holdsAgain);
  })().finally(() => { holdsRead = null; });
  return holdsRead;
}

function addSignals(data) {
  $("sv-signals").append(...data.signals.map((record) => signal(record, data.investigations_enabled)));
  next = data.next;
  more.hidden = !next;
}

async function readScreen() {
  refresh.disabled = more.disabled = true;
  showError(error, "");
  showError($("sv-more-error"), "");
  try {
    const target = new URLSearchParams(location.search).get("signal");
    const data = await api(`${path}/screen${target ? `?${new URLSearchParams({ signal: target })}` : ""}`);
    $("sv-title").textContent = `${data.project_name} · Supervisor`;
    drawHolds(data.holds ?? []);
    drawRuleOutcomes(data, path, readScreen);
    drawReflections(data.reflections ?? [], path);
    drawAgentRates(data.agent_rates ?? []);
    $("sv-reports").replaceChildren(...reportCards(data, path, readScreen));
    if (data.spending) $("sv-spending").replaceChildren(...spending(data.spending, data.marked_tasks_eligible));
    const problems = data.problems ?? [];
    $("sv-problems").replaceChildren(...(problems.length ? problems.map(problem) : [note("No problems in the register.")]));
    $("sv-score").replaceChildren(...(data.indicators.length ? data.indicators.map(indicator) : [note("All scoreboard indicators are off.")]));
    $("sv-signals").replaceChildren();
    $("sv-investigate-note").hidden = !data.signals.length || data.investigations_enabled;
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
  let incidentHolds = "";
  onIncidents((incidents) => {
    const signature = incidents.filter((i) => i.project === project).flatMap((i) => (i.holds ?? []).map((h) => h.id)).join(",");
    if (signature !== incidentHolds) { incidentHolds = signature; readHolds(); }
  });
  document.addEventListener("supervisorverdict", (event) => { if (event.detail === project) readScreen(); });
  document.addEventListener("supervisorholdchange", (event) => { if (event.detail === project) readHolds(); });
  boardFor(project);
  $("sv-board").href = boardUrl(project);
  refresh.hidden = false;
  readScreen();
} else showError(error, "Open a project’s Supervisor tab to see its scoreboard and signals.");
