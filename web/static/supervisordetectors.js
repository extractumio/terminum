// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { el } from "./listparts.js";
import { fieldError, grid, hint, dateValue, number, row, section, select, text, timeOf } from "./supervisorfields.js";

const MODES = [["", "Project default"], ["off", "Off"], ["shadow", "Shadow"], ["live", "Live"], ["digest", "Digest"]];
const THRESHOLD = { absolute: "Absolute", baseline: "From project history", change: "Change" };

function thresholdFields(edit, title, defaults, saved, change) {
  const kind = Object.keys(defaults)[0];
  const choices = ["", ...(kind === "baseline" ? ["baseline", "absolute"] : [kind])];
  const type = select(choices.map((k) => [k, THRESHOLD[k] ?? "Project default"]), Object.keys(saved ?? {})[0] ?? "");
  const fields = el("div", "fieldgrid");
  let inputs = [];
  function draw() {
    const value = saved?.[type.value] ?? defaults[type.value];
    if (type.value === "baseline") {
      inputs = [number(value?.percentile, { min: 1, max: 100, step: 1 }), number(value?.floor)];
      fields.replaceChildren(row("Percentile", inputs[0]), row("Floor", inputs[1]));
    } else if (type.value) {
      inputs = [number(type.value === "change" ? value?.by : value)];
      fields.replaceChildren(row(type.value === "change" ? "Change by" : "Limit", inputs[0]));
    } else {
      inputs = [];
      fields.replaceChildren();
    }
  }
  draw();
  const initial = [type.value, ...inputs.map((f) => f.value)];
  type.addEventListener("change", draw);
  edit.raw((out) => {
    if (JSON.stringify(initial) === JSON.stringify([type.value, ...inputs.map((f) => f.value)])) return;
    for (const f of inputs) {
      if (f.value === "" || !f.validity.valid) fieldError(f, title, f.validationMessage || "Enter a threshold.");
    }
    const [first, second] = inputs.map((f) => Number(f.value));
    change(out, type.value === "baseline" ? { baseline: { percentile: first, floor: second } }
      : type.value === "change" ? { change: { by: first } }
      : type.value === "absolute" ? { absolute: first } : null);
  });
  return [row("Threshold", type), fields];
}

function thresholdText(threshold) {
  if ("absolute" in threshold) return `absolute ${threshold.absolute}`;
  if ("change" in threshold) return `change by ${threshold.change.by}`;
  return `percentile ${threshold.baseline.percentile}, floor ${threshold.baseline.floor}`;
}

export function detectors(edit, draft, defaults) {
  const rows = Object.entries(defaults).map(([name, d]) => {
    const saved = draft.detectors[name] ?? {};
    const set = (out, key, value) => { (out.detectors[name] ??= {})[key] = value; };
    const mode = select(MODES, saved.mode ?? "");
    const days = number(saved.window_days, { min: 1, max: 365, step: 1, placeholder: d.window_days });
    const reason = text(saved.mute?.reason);
    const until = text(dateValue(saved.mute?.until, "datetime-local"), "datetime-local");
    until.step = "1";
    const initial = [reason.value, until.value];
    edit.raw((out) => {
      if (reason.value === initial[0] && until.value === initial[1]) return;
      if (reason.value || until.value) {
        if (!reason.value.trim()) fieldError(reason, name, "A detector mute needs a reason.");
        if (!(timeOf(until) > 0)) fieldError(until, name, "A detector mute needs an end time.");
        set(out, "mute", { reason: reason.value, until: timeOf(until) });
      } else set(out, "mute", null);
    });
    const status = hint("");
    status.dataset.detectorStatus = name;
    return section(name, hint(`Default mode ${d.mode}, threshold ${thresholdText(d.threshold)}.`), status,
      grid(edit.field("Mode", mode, (v, out) => set(out, "mode", v || null)),
        edit.field("Window, days", days, (v, out) => set(out, "window_days", v ? Number(v) : null))),
      ...thresholdFields(edit, name, d.threshold, saved.threshold, (out, v) => set(out, "threshold", v)),
      grid(row("Mute reason", reason), row("Muted until", until)));
  });
  const unavailable = el("div", "sv-stack");
  unavailable.id = "supervisorunavailable";
  return section("Detectors and mutes", hint("Shadow records findings for review. Live applies the detector’s response rules."),
    ...rows, unavailable);
}

export function detectorStatus(root, scan) {
  const rules = scan?.detectors ?? {};
  for (const node of root.querySelectorAll("[data-detector-status]")) {
    const rule = rules[node.dataset.detectorStatus];
    const calibrated = Object.entries(rule?.observed_thresholds ?? {}).map(([metric, range]) =>
      `${metric} ${range.min === range.max ? range.min : `${range.min}–${range.max}`}`);
    node.textContent = rule ? [
      rule.can_run ? `Effective mode ${rule.mode}.` : "Unavailable.",
      ...rule.unavailable, calibrated.length ? `Calibrated limits: ${calibrated.join(", ")}.` : "",
    ].filter(Boolean).join(" ") : "";
    node.hidden = !rule;
  }
  const additional = root.querySelector("#supervisorunavailable");
  const configured = new Set([...root.querySelectorAll("[data-detector-status]")].map((n) => n.dataset.detectorStatus));
  additional?.replaceChildren(...Object.entries(rules).filter(([name]) => !configured.has(name)).map(([name, rule]) =>
    hint(`${name} — ${rule.can_run ? "Built-in watchdog." : rule.unavailable.join(" ")}`)));
}
