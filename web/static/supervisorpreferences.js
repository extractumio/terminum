// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-222 project preferences. The unchanged parts of the server pack travel with a revisioned save.
import { el } from "./listparts.js";
import { TYPES } from "./taskattrs.js";
import { button, fillSelect } from "./kanbanutil.js";
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
  const reads = new WeakMap();
  const direction = [["up", "Higher is better"], ["down", "Lower is better"]];
  const add = (name = "", saved = {}) => {
    const legacy = typeof saved === "string";
    const title = text(name), id = literal(text(legacy ? saved : saved.probe));
    const metric = literal(text(saved.metric)), guard = literal(text(saved.guard));
    const change = select(direction, saved.higher_is_better === false ? "down" : "up");
    const guardChange = select(direction, saved.guard_higher_is_better === true ? "up" : "down");
    const record = stack(grid(row("Indicator", title), row("Probe ID", id),
      row("Metric ID", metric), row("Metric direction", change),
      row("Guard metric ID", guard), row("Guard direction", guardChange)),
      actions(button("textkey", "Remove indicator", () => { record.remove(); addKey.focus(); })));
    entries.append(record);
    reads.set(record, (pairs) => {
      const label = title.value.trim(), probe = id.value.trim();
      if (legacy && title.value === name && id.value === saved && !metric.value && !guard.value
        && change.value === "up" && guardChange.value === "down") {
        if (pairs.has(name)) fieldError(title, "Outside indicator", "Use each indicator name once.");
        pairs.set(name, saved);
        return;
      }
      if (pairs.has(label)) fieldError(title, "Outside indicator", "Use each indicator name once.");
      for (const [input, label] of [[title, "Indicator name"], [id, "Probe ID"], [metric, "Metric ID"], [guard, "Guard metric ID"]]) {
        if (!input.value.trim()) fieldError(input, label, "Enter a value.");
      }
      if (metric.value.trim() === guard.value.trim()) fieldError(guard, "Guard metric ID", "Choose a different metric.");
      pairs.set(label, { probe, metric: metric.value.trim(), higher_is_better: change.value === "up",
        guard: guard.value.trim(), guard_higher_is_better: guardChange.value === "up" });
    });
    return title;
  };
  const addKey = button("textkey", "Add outside indicator", () => add().focus());
  for (const [name, saved] of Object.entries(existing)) add(name, saved);
  edit.raw((out) => {
    const pairs = new Map();
    for (const record of entries.children) reads.get(record)(pairs);
    out.charter.outside_indicators = Object.fromEntries(pairs);
  });
  return stack(entries, actions(addKey));
}

function feedbackProbes(edit, existing) {
  const entries = el("div", "sv-stack");
  const add = (probe = "", base = "") => {
    const id = literal(text(probe)), link = literal(text(base));
    const record = stack(grid(row("Feedback probe ID", id), row("HTTPS reference base", link)),
      actions(button("textkey", "Remove feedback probe", () => { record.remove(); addKey.focus(); })));
    entries.append(record);
    return id;
  };
  const addKey = button("textkey", "Add feedback probe", () => add().focus());
  for (const [probe, base] of Object.entries(existing)) add(probe, base);
  edit.raw((out) => {
    const pairs = new Map();
    for (const record of entries.children) {
      const [id, link] = record.querySelectorAll("input");
      const probe = id.value.trim(), base = link.value.trim();
      if (!probe || pairs.has(probe)) fieldError(id, "Feedback probe ID", "Enter a unique probe ID.");
      if (!base.startsWith("https://") || !base.endsWith("/")) fieldError(link, "HTTPS reference base", "Enter an HTTPS address ending in a slash.");
      pairs.set(probe, base);
    }
    if (pairs.size || "feedback" in out.probes) out.probes.feedback = Object.fromEntries(pairs);
  });
  return stack(entries, actions(addKey));
}

// AC-213: a never line breaks only when a checked fact the owner tied to it fires.
function neverChecks(edit, saved, checks, never) {
  const tied = saved.charter.never_checks ?? {};
  const choices = checks.map(({ detector, fact }) => {
    const options = (value) => {
      const choices = [["", "Not tied to a never rule"], ...lines(never.value).map((line) => [line, line])];
      if (value && !choices.some(([line]) => line === value)) choices.push([value, `${value} (not in the never list)`]);
      return choices;
    };
    const saved = tied[detector] ?? "";
    const node = select(options(saved), saved);
    never.addEventListener("input", () => fillSelect(node, options(node.value), node.value));
    return { detector, node, field: row(fact, node) };
  });
  edit.raw((out) => {
    const next = Object.fromEntries(choices.filter(({ node }) => node.value).map(({ detector, node }) => [detector, node.value]));
    if (JSON.stringify(next) !== JSON.stringify(tied)) out.charter.never_checks = next;
  });
  return choices.length ? [el("h3", null, "Checked never rules"),
    hint("A fact tied to a never rule makes its signal a charter breach: impact 3, and its fix is exempt from the spending limit."),
    ...choices.map(({ field }) => field)] : [];
}

function charter(edit, saved, inputs, checks) {
  const fields = [
    ["mission", "Mission"], ["users", "Users, one per line", lines], ["risk_appetite", "Risk appetite"],
    ["priorities", "Priorities, highest first, one per line", lines], ["never", "Never list, one rule per line", lines],
  ].map(([key, label, read]) => {
    const value = saved.charter[key];
    const input = text(Array.isArray(value) ? value.join("\n") : value, "area");
    if (key === "never" || key === "priorities") inputs.set(key, input);
    return edit.field(label, input, (v, out) => { out.charter[key] = read ? read(v) : v; });
  });
  const date = (label, key) => edit.field(label, text(dateValue(saved.charter[key], "date"), "date"),
    (v, out) => { out.charter[key] = v; }, timeOf);
  // AC-284: a manager's draft stays marked until the owner's Save
  const drafted = saved.charter.drafted_by_manager
    ? [hint("The project's manager drafted this charter from its repository. Review it, then Save to make it yours.")] : [];
  const node = section("Charter and allocation", ...drafted, ...fields, ...neverChecks(edit, saved, checks, inputs.get("never")),
    date("Review priorities on", "priority_review_at"),
    el("h3", null, "Allocation target"),
    hint("Set the intended share of work by task type. Leave all shares empty to measure allocation without a target."),
    allocation(edit, saved), date("Review allocation on", "allocation_review_at"));
  if (drafted.length) node.open = true;
  return node;
}

function templatePicker(templates, fill) {
  const choice = select([["", "Choose a project type"], ...templates.map(({ id, name }) => [id, name])], "");
  const filled = el("p", "msay");
  filled.setAttribute("role", "status");
  const apply = button("textkey", "Fill settings draft", () => {
    const template = templates.find(({ id }) => id === choice.value);
    fill(template);
    filled.textContent = `${template.name} settings filled. Review the draft and press Save.`;
  });
  apply.disabled = true;
  choice.addEventListener("change", () => {
    apply.disabled = !choice.value;
    filled.textContent = "";
  });
  return section("Project template", row("Project type", choice),
    hint("Fills the never list, priorities, indicator targets, T2 and T3 paths, and standard probe choices in this draft."),
    actions(apply), filled);
}

export function preferences(view, agents, { request, run }) {
  const saved = view.settings, edit = editor();
  const level = edit.field("Level", select(LEVELS, saved.level), (v, out) => { out.level = v; });
  const choices = [["", "None"], ...agents.map((a) => [a.id, a.name])];
  const boards = [["", "Ask me"], ...(view.engine_boards ?? []).map((project) => [project.id, project.name])];
  const engine = edit.field("Engine project", select(boards, saved.engine_board ?? ""), (v, out) => { out.engine_board = v || null; });
  const tiers = TIERS.map(([key, title]) => edit.field(`${title} tier`, select(choices, saved.tiers[key] ?? ""),
    (v, out) => { out.tiers[key] = v || null; }));
  const budgets = BUDGETS.map(([key, label, bounds, optional]) => {
    const field = number(saved.budgets[key], { ...bounds, placeholder: optional ? "No ceiling" : undefined });
    field.required = !optional;
    return edit.field(label, field, (v, out) => { out.budgets[key] = v ? Number(v) : null; });
  });
  const indicators = INDICATORS.map(([key, label]) => edit.field(label, check(saved.charter.indicators[key]),
    (v, out) => { out.charter.indicators[key] = v; }));
  const targetInputs = new Map();
  const targets = INDICATORS.map(([key, label]) => {
    const input = text(saved.charter.indicator_targets?.[key]);
    targetInputs.set(key, input);
    return edit.field(`${label} target`, input, (v, out) => {
      const values = out.charter.indicator_targets ??= {};
      if (v.trim()) values[key] = v.trim();
      else delete values[key];
    });
  });
  const surfaceInputs = new Map();
  const surfaces = [
    ["t1", "T1 internal boundaries"], ["t2", "T2 contracts"], ["t3", "T3 breaking surfaces"],
    ["t3_terms", "T3 phrases in task text"], ["existing_review", "Paths with an existing review process"],
  ].map(([key, label]) => {
    const input = literal(text((saved.charter.surfaces[key] ?? []).join("\n"), "area"));
    surfaceInputs.set(key, input);
    return edit.field(label, input, (v, out) => { out.charter.surfaces[key] = lines(v); });
  });
  const probeInputs = new Map();
  const probes = [["git", "Git"], ["gate_record", "Gate records"], ["code_host", "Code host"]].map(([key, label]) => {
    const input = check(saved.probes[key]);
    probeInputs.set(key, input);
    return edit.field(label, input, (v, out) => { out.probes[key] = v; });
  });
  const fileLimit = edit.field("Git source file line limit", number(saved.probes.git_file_line_cap, { min: 1, step: 1 }), (v, out) => {
    out.probes.git_file_line_cap = v;
  }, (field) => field.value === "" ? null : field.valueAsNumber);
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
  const charterInputs = new Map();
  const charterSection = charter(edit, saved, charterInputs, view.never_checks ?? []);
  const indicatorSection = section("Indicators", part("div", "sv-checks", ...indicators),
    el("h3", null, "Targets"), hint("Targets appear on the scoreboard alongside its measured verdicts."), grid(...targets),
    hint("An approved custom probe supplies the metric and its guard."), outsideIndicators(edit, saved.charter.outside_indicators));
  const surfaceSection = section("Surface map", hint("Enter one pattern or phrase per line. An edit or removal on a T3 path requires owner sign-off. An addition on the same path requires a T2 review."),
    ...surfaces, actions(preview), previewError, previewResult);
  const probeSection = section("Standard probes", hint("Choose which standard probes may collect data."), actions(...probes), grid(fileLimit));

  const template = view.templates?.length ? templatePicker(view.templates, (candidate) => {
    const fill = (input, value) => {
      input.value = Array.isArray(value) ? value.join("\n") : value ?? "";
      input.dispatchEvent(new Event("input"));
    };
    for (const [key, input] of charterInputs) fill(input, candidate[key]);
    for (const [key, input] of targetInputs) fill(input, candidate.targets[key]);
    for (const key of ["t2", "t3"]) fill(surfaceInputs.get(key), candidate.surfaces[key]);
    for (const key of ["git", "gate_record"]) probeInputs.get(key).checked = candidate.probes[key];
    for (const group of [charterSection, indicatorSection, surfaceSection, probeSection]) group.open = true;
  }) : null;

  const node = stack(...(template ? [template] : []), grid(level, engine),
    section("Tier agents and budgets", hint("With no tier agents selected, the Supervisor runs on code alone."),
    grid(...tiers), el("h3", null, "Budgets"), hint("Budgets set the limits for judgment rounds and fixes when the Supervisor admits work."), grid(...budgets)),
  charterSection, indicatorSection, surfaceSection, detectors(edit, saved, view.detector_defaults), probeSection,
  section("Feedback probes", hint("Use an approved custom probe and the HTTPS prefix for its numbered references."),
    feedbackProbes(edit, saved.probes.feedback ?? {})));
  return { node, read: () => edit.read(structuredClone(saved)) };
}
