// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-222 project preferences. The unchanged parts of the server pack travel with a revisioned save.
import { el } from "./listparts.js";
import { TYPES } from "./taskattrs.js";
import { button } from "./kanbanutil.js";
import { detectors } from "./supervisordetectors.js";
import { actions, check, editor, feedback, fieldError, grid, hint, literal, dateValue, number, part, row, section, select, stack, text, tick, timeOf } from "./supervisorfields.js";

const LEVELS = [["observer", "Observer"], ["advisor", "Advisor"], ["operator", "Operator"]];
const TIERS = [["small", "Small"], ["medium", "Medium"], ["strong", "Strong"], ["independent", "Independent"]];
const INDICATORS = [["involvement", "Features per owner hour"], ["stability", "Stability"], ["lead_time", "Lead time"],
  ["releases", "Release cadence"], ["backlog", "Backlog"], ["cognitive_load", "Cognitive load"], ["operations", "Operational and strategic work"]];
const BUDGETS = [
  ["judgment_percent", "Judgment, % of weekly spend", { max: 100 }],
  ["fixes_percent", "Fixes, % of weekly spend", { max: 100 }],
  ["judgment_floor_tokens", "Judgment floor, tokens", { step: 1 }],
  ["investigation_reserve_percent", "Investigation reserve, %", { max: 100 }],
  ["judgment_ceiling", "Judgment ceiling, tokens per week", { step: 1 }, true],
  ["fixes_ceiling", "Fixes ceiling, USD per week", {}, true],
  ["daily_token_ceiling", "Daily ceiling, tokens", { step: 1 }, true],
  ["round_token_ceiling", "Round ceiling, tokens", { step: 1 }, true],
];
const lines = (value) => value.split("\n").map((line) => line.trim()).filter(Boolean);

function allocation(edit, draft) {
  const inputs = TYPES.map((kind) => number(draft.charter.allocation[kind] === undefined
    ? null : Math.round(draft.charter.allocation[kind] * 1e12) / 1e10, { max: 100 }));
  const before = inputs.map((node) => node.value);
  edit.raw((out) => {
    if (inputs.every((node, i) => node.value === before[i] && node.validity.valid)) return;
    const shares = {};
    for (const [i, node] of inputs.entries()) {
      if (!node.validity.valid) fieldError(node, `${TYPES[i]} allocation`, node.validationMessage);
      if (node.value !== "") shares[TYPES[i]] = Number(node.value) / 100;
    }
    if (Object.keys(shares).length && Math.abs(Object.values(shares).reduce((sum, n) => sum + n, 0) - 1) >= 1e-9) {
      fieldError(inputs.find((node) => node.value !== "") ?? inputs[0], "Allocation target", "Shares must add up to 100%.");
    }
    out.charter.allocation = shares;
  });
  return grid(...inputs.map((node, i) => row(`${TYPES[i]}, %`, node)));
}

function outsideIndicators(edit, existing) {
  const entries = el("div", "sv-stack");
  const add = (name = "", probe = "") => {
    const title = text(name), id = literal(text(probe));
    title.placeholder = "Indicator";
    id.placeholder = "Probe ID";
    const record = stack(grid(row("Indicator", title), row("Probe ID", id)),
      actions(button("textkey", "Remove indicator", () => { record.remove(); addKey.focus(); })));
    entries.append(record);
    return title;
  };
  const addKey = button("textkey", "Add outside indicator", () => add().focus());
  for (const [name, probe] of Object.entries(existing)) add(name, probe);
  edit.raw((out) => {
    const fields = [...entries.children].map((record) => record.querySelectorAll("input"));
    if (JSON.stringify(fields.map(([title, probe]) => [title.value, probe.value])) === JSON.stringify(Object.entries(existing))) return;
    const pairs = new Map();
    for (const [title, probe] of fields) {
      const name = title.value.trim(), id = probe.value.trim();
      if (!name || !id) fieldError(!name ? title : probe, "Outside indicator", "Enter an indicator name and a probe ID.");
      if (pairs.has(name)) fieldError(title, "Outside indicator", "Use each indicator name once.");
      pairs.set(name, id);
    }
    out.charter.outside_indicators = Object.fromEntries(pairs);
  });
  return stack(entries, actions(addKey));
}

function charter(edit, saved) {
  const fields = [
    ["mission", "Mission"], ["users", "Users, one per line", lines], ["risk_appetite", "Risk appetite"],
    ["priorities", "Priorities, highest first, one per line", lines], ["never", "Never list, one rule per line", lines],
  ].map(([key, label, read]) => {
    const value = saved.charter[key];
    return edit.field(label, text(Array.isArray(value) ? value.join("\n") : value, "area"),
      (v, out) => { out.charter[key] = read ? read(v) : v; });
  });
  const date = (label, key) => edit.field(label, text(dateValue(saved.charter[key], "date"), "date"),
    (v, out) => { out.charter[key] = v; }, timeOf);
  return section("Charter and allocation", ...fields, date("Review priorities on", "priority_review_at"),
    el("h3", null, "Allocation target"),
    hint("Set the intended share of work by task type. Leave all shares empty to measure allocation without a target."),
    allocation(edit, saved), date("Review allocation on", "allocation_review_at"));
}

export function preferences(view, agents, { request, run }) {
  const saved = view.settings, edit = editor();
  const level = edit.field("Level", select(LEVELS, saved.level), (v, out) => { out.level = v; });
  const choices = [["", "None"], ...agents.map((a) => [a.id, a.name])];
  const tiers = TIERS.map(([key, title]) => edit.field(`${title} tier`, select(choices, saved.tiers[key] ?? ""),
    (v, out) => { out.tiers[key] = v || null; }));
  const budgets = BUDGETS.map(([key, label, bounds, optional]) => {
    const field = number(saved.budgets[key], { ...bounds, placeholder: optional ? "No ceiling" : undefined });
    field.required = !optional;
    return edit.field(label, field, (v, out) => { out.budgets[key] = v ? Number(v) : null; });
  });
  const indicators = INDICATORS.map(([key, label]) => edit.field(label, check(saved.charter.indicators[key]),
    (v, out) => { out.charter.indicators[key] = v; }));
  const surfaceInputs = new Map();
  const surfaces = [
    ["t1", "T1 internal boundaries"], ["t2", "T2 contracts"], ["t3", "T3 breaking surfaces"],
    ["t3_terms", "T3 phrases in task text"], ["existing_review", "Paths with an existing review process"],
  ].map(([key, label]) => {
    const input = literal(text((saved.charter.surfaces[key] ?? []).join("\n"), "area"));
    surfaceInputs.set(key, input);
    return edit.field(label, input, (v, out) => { out.charter.surfaces[key] = lines(v); });
  });
  const probes = [["git", "Git"], ["gate_record", "Gate records"], ["code_host", "Code host"]].map(([key, label]) =>
    edit.field(label, check(saved.probes[key]), (v, out) => { out.probes[key] = v; }));
  const previewError = feedback();
  const previewResult = el("p", "msay");
  previewResult.setAttribute("role", "status");
  for (const input of surfaceInputs.values()) {
    input.addEventListener("input", () => { previewResult.textContent = ""; });
  }
  const preview = button("textkey", "Preview last month", () => run(previewError, async () => {
    const draft = () => Object.fromEntries([...surfaceInputs].map(([key, input]) => [key, lines(input.value)]));
    const candidate = draft();
    previewResult.textContent = "Reading task history…";
    try {
      const result = await request("/surfaces/preview", "POST", candidate);
      if (JSON.stringify(candidate) !== JSON.stringify(draft())) return;
      previewResult.textContent = result.empty ? "This map would require no design reviews."
        : `T2: ${result.t2} · T3: ${result.t3} · ${result.sampled} tasks sampled · ${result.without_diff} without a recorded diff${result.truncated ? " · sample limit reached" : ""}`;
    } catch (error) {
      previewResult.textContent = "";
      throw error;
    }
    return preview;
  }));
  const node = stack(level, section("Tier agents and budgets", hint("With no tier agents selected, the Supervisor runs on code alone."),
    grid(...tiers), el("h3", null, "Budgets"), hint("Budgets set the limits for judgment rounds and fixes when the Supervisor admits work."), grid(...budgets)),
  charter(edit, saved), section("Indicators", part("div", "sv-checks", ...indicators),
    hint("Outside indicators will use an approved probe to supply their measurements."), outsideIndicators(edit, saved.charter.outside_indicators)),
  section("Surface map", hint("Enter one pattern or phrase per line. An edit or removal on a T3 path requires owner sign-off. An addition on the same path requires a T2 review."),
    ...surfaces, actions(preview), previewError, previewResult),
  detectors(edit, saved, view.detector_defaults),
  section("Standard probes", hint("These settings will control which standard probes may collect data."), actions(...probes)));
  return { node, read: () => edit.read(structuredClone(saved)) };
}
