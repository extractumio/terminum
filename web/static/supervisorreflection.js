// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { $, api, showError, stamp } from "./kanbanutil.js";
import { el, textKey } from "./listparts.js";
import { taskUrl } from "./tasklink.js";
import { part, stack } from "./supervisorfields.js";

const triggerNames = {
  breaker_opened: "Breaker opened",
  human_override: "Owner override",
  fix_recurred: "Fixed problem recurred",
  miss: "Missed evidence",
  alert_wrong: "Alert marked wrong",
  budget_exceeded: "Budget exceeded",
  decision_rate: "Decision rate outside its band",
  incident_postmortem: "Incident ended",
};

const number = new Intl.NumberFormat(undefined, { maximumSignificantDigits: 3 });
const percent = new Intl.NumberFormat(undefined, { style: "percent", maximumFractionDigits: 1 });
const dollars = new Intl.NumberFormat(undefined, { style: "currency", currency: "USD", maximumSignificantDigits: 3 });

function reading(label, value) {
  return part("div", "sv-reading", el("dt", null, label), el("dd", null, value));
}

function panel(label, content) {
  return part("details", "sv-panel", el("summary", "sv-touch", label), content);
}

function summary(metric, format) {
  const sample = `n = ${metric.sample}`;
  if (metric.value == null) return `${metric.missing} · ${sample}`;
  const [min, max] = metric.interval_95;
  const interval = max == null ? "95% interval needs another price" : `95% interval ${format.format(min)}–${format.format(max)}`;
  return [format.format(metric.value), sample, interval, metric.missing, !metric.can_gate && "Advisory only"]
    .filter(Boolean).join(" · ");
}

export function drawAgentRates(rates) {
  const grouped = new Map();
  for (const row of rates) {
    if (!grouped.has(row.agent)) grouped.set(row.agent, []);
    grouped.get(row.agent).push(row);
  }
  const agents = Array.from(grouped.values(), (rows) => panel(rows[0].agent_name,
    stack(...rows.map((row) => part("section", "sv-stack",
      el("h4", null, row.task_type),
      part("dl", "sv-readings",
        reading("Failed runs", summary(row.failure, percent)),
        reading("Returned runs", summary(row.return, percent)),
        reading("Questions per task", summary(row.questions, number)),
        reading("Cost per run", summary(row.cost, dollars)),
      ))))));
  $("sv-agent-rates").replaceChildren(...(agents.length ? agents : [el("p", "msay", "No pool agents.")]));
}

function timeline(path, id) {
  const contents = stack(), control = panel("Case timeline", contents);
  const error = el("p", "merr");
  error.setAttribute("role", "alert");
  error.hidden = true;
  contents.append(error);
  const read = async () => {
    if (!control.open) return;
    control.removeEventListener("toggle", read);
    const status = el("p", "msay", "Loading the case timeline…");
    status.setAttribute("role", "status");
    contents.append(status);
    try {
      const { case: saved } = await api(`${path}/rounds/${encodeURIComponent(id)}`);
      contents.append(part("ol", null, ...saved.timeline.map((row) =>
        part("li", null, `${stamp(row.at)} · ${row.kind.replaceAll("_", " ")} · ${row.ref}`,
          el("p", "msay", JSON.stringify(row.data))))));
    } catch (e) {
      showError(error, e.message);
      control.addEventListener("toggle", read);
    } finally { status.remove(); }
  };
  control.addEventListener("toggle", read);
  return control;
}

function reflection(entry, path) {
  const heading = `${entry.triggers.map((key) => triggerNames[key] ?? key).join(" · ")} · ${stamp(entry.at)}`;
  const result = entry.result;
  const content = stack();
  if (!result) {
    content.append(el("p", "msay", entry.failure ?? entry.waiting ?? (entry.case ? "In progress." : "Waiting for an independent agent.")));
  } else {
    content.append(part("dl", "sv-readings", reading("Missed evidence", result.missed),
      reading("Noise", result.noise), reading("Wrong decisions", result.wrong)));
    for (const proposal of result.proposals) {
      content.append(part("dl", "sv-readings", reading(`Proposed change · settings revision ${result.settings_revision}`, proposal.change),
        reading("Replay proposal", proposal.replay), reading("Timeline events", proposal.evidence.join(", "))));
    }
  }
  if (entry.routing_card) {
    const link = el("a", "textkey", `Routing decision ${entry.routing_card}`);
    link.href = taskUrl(entry.routing_card);
    textKey(link);
    content.append(link);
  }
  if (entry.case) content.append(timeline(path, entry.case));
  return panel(heading, content);
}

export function drawReflections(entries, path) {
  $("sv-reflections").replaceChildren(...(entries.length ? entries.map((entry) => reflection(entry, path)) : [el("p", "msay", "No reflections recorded.")]));
}
