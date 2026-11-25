// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Fixture data of the Supervisor tab and screen checks in e2e_supervisor.mjs.
export const ROOT = "/api/task-projects/api/supervisor";
export const RULE = "flow.failed_no_reason";
export const DATE = 1_790_000_000;
export const COMMAND = "printf '%s\\n' 'test fixture only'\ntrue";
export const LITERAL = "<img src=x onerror='window.fixtureRan=1'>" + "long-".repeat(25);
export const SETTINGS = {
  level: "operator", tiers: { small: null, medium: null, strong: null, independent: null },
  budgets: { judgment_percent: 3, fixes_percent: 10, judgment_floor_tokens: 1_000_000, investigation_reserve_percent: 30 },
  charter: { drafted_by_manager: true, mission: "", users: [], risk_appetite: "", never: [], priorities: [], allocation: {}, surfaces: { t2: [], t3: [] },
    indicators: { involvement: true, stability: true, lead_time: true, releases: true, backlog: true, cognitive_load: true, operations: true },
    outside_indicators: {} },
  detectors: { [RULE]: { mute: null } }, probes: { git: true, gate_record: true, code_host: true },
  guardrails: { threshold_percent: 54 }, language: "English",
};
export const DEFAULTS = { [RULE]: { mode: "live", window_days: 7, threshold: { baseline: { percentile: 90, floor: 1 } } } };
export const SIGNAL = { fingerprint: "one", project: "api", detector: RULE, subject_id: LITERAL, subject_kind: "project",
  metric: "Unexplained failures", level: 1, state: "raised", count: 2, response: "digest", last_seen: DATE,
  measure: { value: 12, baseline: 4, threshold: 8, window_start: DATE - 1000, window_end: DATE },
  evidence: [{ table: "tasks", id: "API-2" }, { table: "events", id: LITERAL }, { table: "supervisor_line", id: { channel: 2, failure: "Exit status 17." } }] };
export const INCIDENT = { id: 42, task: "API-40", project: "api", project_name: "API service", level: 3, revision: 1,
  happened: "The collector missed successive heartbeats.", held: "No hold in effect.",
  needed: "Acknowledge when seen.", no_answer: "No automatic action without an answer.", reports: ["R_API-8"] };
export const HOLD = { id: 5, kind: "pause_integration", label: "Integration queue paused", undo: "Resume integration",
  signal: "0", detector: RULE, reason: LITERAL };
export const PROBLEM = { id: 42, title: "Repeated failures", state: "confirmed", lens: "flow", impact: 2, share: 0.2,
  indicator: "failure_events", rank: 0.32, hypothesis: LITERAL, disproof: "Failures stop without a change.",
  yardstick: [{ detector: RULE, fingerprint: "fixture-1", threshold: 8, weeks: 4 }],
  fix_options: [{ layer: "test", action: "Exercise retries." }], tasks: ["API-2"], action_ready: true,
  exception: true, breaches: [{ never: "Lose user data.", detector: "arch.migration_edited", fingerprint: "fixture-2" }] };
export const SPENDING = {
  judgment: { tokens: 16000, held_tokens: 0, limit_tokens: 100000, day_tokens: 6000,
    day_limit_tokens: 20000, round_limit_tokens: 4000, investigation_left: 14000, routine_left: 0 },
  fixes: { dollars: 10.25, held_dollars: 8, limit_dollars: 20, estimated_dollars_per_run: 8, unpriced_runs: 1 },
};
