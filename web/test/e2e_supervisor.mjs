// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { check, runningIn, until } from "./check.mjs";
import { drawn, fakeBoard, openPage, ORIGIN, SIZES, serveRenderers, shot, THEME } from "./fake.mjs";

const ROOT = "/api/task-projects/api/supervisor";
const RULE = "flow.failed_no_reason";
const DATE = 1_790_000_000;
const COMMAND = "printf '%s\\n' 'test fixture only'\ntrue";
const LITERAL = "<img src=x onerror='window.fixtureRan=1'>" + "long-".repeat(25);
const SETTINGS = {
  level: "operator", tiers: { small: null, medium: null, strong: null, independent: null },
  budgets: { judgment_percent: 3, fixes_percent: 10, judgment_floor_tokens: 1_000_000, investigation_reserve_percent: 30 },
  charter: { mission: "", users: [], risk_appetite: "", never: [], priorities: [], allocation: {}, surfaces: { t2: [], t3: [] },
    indicators: { involvement: true, stability: true, lead_time: true, releases: true, backlog: true, cognitive_load: true, operations: true },
    outside_indicators: {} },
  detectors: { [RULE]: { mute: null } }, probes: { git: true, gate_record: true, code_host: true },
  guardrails: { threshold_percent: 54 }, language: "English",
};
const DEFAULTS = { [RULE]: { mode: "live", window_days: 7, threshold: { baseline: { percentile: 90, floor: 1 } } } };
const SIGNAL = { fingerprint: "one", project: "api", detector: RULE, subject_id: LITERAL, subject_kind: "project",
  metric: "Unexplained failures", level: 1, state: "raised", count: 2, response: "digest", last_seen: DATE,
  measure: { value: 12, baseline: 4, threshold: 8, window_start: DATE - 1000, window_end: DATE },
  evidence: [{ table: "tasks", id: "API-2" }, { table: "events", id: LITERAL }, { table: "supervisor_line", id: { channel: 2, failure: "Exit status 17." } }] };
const INCIDENT = { id: 42, task: "API-40", project: "api", project_name: "API service", level: 3, revision: 1,
  happened: "The collector missed successive heartbeats.", held: "No hold in effect.",
  needed: "Acknowledge when seen.", no_answer: "No automatic action without an answer." };
const HOLD = { id: 5, kind: "pause_integration", label: "Integration queue paused", undo: "Resume integration",
  signal: "0", detector: RULE, reason: LITERAL };
const SCORE = { name: "Release cadence", verdict: "not measured yet", guard: "Stability", metrics: [
  { name: "Releases per month", verdict: "not measured yet", missing: "release tags are unavailable" },
  { name: "Fixture measured series", verdict: "better", missing: null, before: 3, current: 5, noise: 0.5 },
] };
const PROBLEM = { id: 42, title: "Repeated failures", state: "confirmed", lens: "flow", impact: 2, share: 0.2,
  indicator: "failure_events", rank: 0.32, hypothesis: LITERAL, disproof: "Failures stop without a change.",
  yardstick: [{ detector: RULE, fingerprint: "fixture-1", threshold: 8, weeks: 4 }],
  fix_options: [{ layer: "test", action: "Exercise retries." }], tasks: ["API-2"], action_ready: true };
const SPENDING = {
  judgment: { tokens: 16000, held_tokens: 0, limit_tokens: 100000, day_tokens: 6000,
    day_limit_tokens: 20000, round_limit_tokens: 4000, investigation_left: 14000, routine_left: 0 },
  fixes: { dollars: 10.25, held_dollars: 8, limit_dollars: 20, estimated_dollars_per_run: 8, unpriced_runs: 1 },
};
const json = (route, obj, status = 200) => route.fulfill({ status, json: obj });

async function fixture(page, theme) {
  const projects = [{ id: "api", name: "API service", color: 0 }, { id: "web", name: "Web service", color: 1 }];
  const agents = [{ project: "api", id: "mine", name: "API agent" }, { project: "web", id: "foreign", name: "Web agent" }];
  await serveRenderers(page);
  await fakeBoard(page, { meta: () => ({ projects, agents, theme, users: [], hosts: [], sessions: [] }), tasks: () => [], counts: {} });
  await page.route("**/api/messages/meta", (route) => json(route, { projects, theme }));
  await page.route("**/api/supervisor/incidents", (route) => json(route, []));
  await page.route("**/api/supervisor/incidents/42/investigate", (route) => json(route, { message: "Incident investigation requested." }));
  await page.route("**/api/supervisor/incidents/42/talk", (route) => json(route, { error: "The marked pane changed." }, 400));
  await page.route("**/api/supervisor/incidents/42/ruling", (route) => json(route, { task: INCIDENT.task }));
  let ruling = null, held = true, stopped = true;
  const incident = () => held ? { ...INCIDENT, held: "Integration queue paused.", holds: [{ id: HOLD.id, label: HOLD.label }],
    needed: "Keep or release the hold.", no_answer: "Without an answer the hold stays." }
    : { ...INCIDENT, revision: 2 };
  await page.route("**/api/supervisor/incidents/42", (route) => {
    const body = route.request().postDataJSON();
    if (body?.hold === HOLD.id) {
      if (body.action === "release_hold") held = false;
      if (body.action === "keep_hold" || body.action === "release_hold") return json(route, incident());
    }
    return json(route, { rows: [], next: null });
  });
  const activeHolds = () => [held && HOLD, stopped && { ...HOLD, id: 8, label: "Run stopped", task: "API-2", undo: "RUN" }].filter(Boolean);
  let enabled = false, refusal = true, saved = structuredClone(SETTINGS), revision = 0, candidate = false, approved = false, commandViewed = false;
  const calls = [];
  let line = { revision: 0, settings: { channels: [], link_base: null, quiet: [1380, 480], ladder: [10, 30, 60] } };
  let failedTest = true;
  let refuseSave;
  const firstSave = new Promise((resolve) => { refuseSave = resolve; });
  const view = () => ({ revision, settings: saved, detector_defaults: DEFAULTS,
    unset_probes: candidate ? ["latency"] : [], confirmed_probes: approved ? ["latency"] : [] });
  await page.route(/\/api\/task-projects\/[^/]+\/supervisor(?:[/?]|$)/, async (route) => {
    const req = route.request(), { pathname, searchParams } = new URL(req.url());
    const suffix = pathname.slice(ROOT.length), method = req.method(), body = req.postDataJSON();
    calls.push({ suffix, method, body });
    if (pathname === "/api/task-projects/web/supervisor/screen") {
      return json(route, { project_name: "Web service", indicators: [], signals: [{ ...SIGNAL, metric: "Web signal" }], next: null });
    }
    if (suffix === "/problems/42/ruling" && method === "POST") {
      ruling = body.reason;
      return json(route, { ok: true });
    }
    if (suffix === "/verdicts" && method === "POST") return json(route, { verdict: body.verdict });
    if (suffix === "/holds") return json(route, activeHolds());
    if (suffix === "/holds/8/release" && method === "POST") {
      if (stopped === true) { stopped = "retry"; return json(route, { error: "The queue is busy. Try its board key again." }, 400); }
      stopped = false;
      return json(route, { id: 8, released: true });
    }
    if (suffix === "/screen") return json(route, { project_name: "API service", indicators: [SCORE], holds: activeHolds(),
      investigations_enabled: true, spending: SPENDING, marked_tasks_eligible: false,
      decisions: [{ id: 1, task: "API-41", problem: 42, question: "Choose a fix based on the recorded evidence." }],
      review: { task: "API-42", at: DATE, result: "## Decisions waiting for you\n\nRead the pending card.",
        items: [{ fingerprint: "0", detector: RULE, metric: SIGNAL.metric, verdict: null }] },
      digest: { at: DATE, last_ok: DATE - 10, entries: [{ ...SIGNAL, fingerprint: "0", action: "Raised for review" }] },
      problems: [{ ...PROBLEM, ...(ruling ? { state: "accepted", ruling } : {}) }],
      signals: Array.from({ length: 50 }, (_, i) => ({ ...SIGNAL, fingerprint: String(i), metric: i ? `Signal ${i}` : SIGNAL.metric })), next: "49" });
    if (suffix === "/incidents") return json(route, { rows: [incident()], next: null });
    if (suffix === "/signals/0/investigate" && method === "POST") {
      return json(route, { message: "Investigation requested." });
    }
    if (suffix === "/signals") {
      return searchParams.get("after") === "49"
        ? json(route, { signals: [{ ...SIGNAL, metric: "Last signal" }], next: null })
        : json(route, { error: "The signal page expired." }, 400);
    }
    if (suffix === "") {
      if (method === "PUT") enabled = body.enabled;
      const attachment = enabled ? { limited_history: true, detectors: { [RULE]: { can_run: true, mode: "shadow", unavailable: [],
        observed_thresholds: { failure_count: { min: 2, max: 3 } } } } } : null;
      return json(route, { enabled, attachment, holds: activeHolds() });
    }
    if (suffix === "/line") {
      if (method === "PUT") line = { revision: line.revision + 1, settings: body.settings };
      return json(route, line);
    }
    if (suffix === "/line/test") {
      if (failedTest) {
        failedTest = false;
        return json(route, { error: "Exit status 17." }, 400);
      }
      return json(route, { sent: true });
    }
    if (suffix === "/settings" && method === "PUT") {
      if (refusal) {
        refusal = false;
        await firstSave;
        return json(route, { error: `${RULE}: invalid Supervisor detector settings.` }, 400);
      }
      saved = body.settings;
      revision++;
    }
    if (suffix === "/export") return route.fulfill({ contentType: "text/plain", body: JSON.stringify({ version: 1, settings: saved }) });
    if (suffix === "/import") {
      saved = { ...JSON.parse(body.file).settings, tiers: saved.tiers };
      candidate = true;
      revision++;
    }
    if (suffix.endsWith("/preview")) { commandViewed = true; return json(route, { command: COMMAND, approval: "one-preview" }); }
    if (suffix.endsWith("/confirm")) {
      approved = commandViewed && body.approval === "one-preview";
      candidate = !approved;
    }
    return json(route, view());
  });
  return { calls, refuseSave };
}

const openGroup = (page, text) => page.locator("#supervisorpanel summary").filter({ hasText: new RegExp(`^${text.replaceAll(".", "\\.")}$`) }).click();

async function settingsTab(page, { calls, refuseSave }, suffix, phone) {
  await page.click("#projectcog");
  await page.click("#projectsupervisor");
  const panel = page.locator("#supervisorpanel");
  const level = panel.getByLabel("Level", { exact: true });
  await level.waitFor();
  await shot(page, `supervisor-tab-${suffix}`);
  check("AC-222 the project tab has its own switch, level and save control",
    await page.isVisible("#supervisorpanel") && await level.inputValue() === "operator" && !await page.isChecked("#supervisoron")
    && await page.isVisible("#supervisorok") && await page.isHidden("#projectok"));
  check("AC-245 the off tab lists the holds that remain in force",
    await panel.getByText("The Supervisor is off. Holds still in effect: Integration queue paused · Run stopped · API-2.", { exact: true }).isVisible());
  await openGroup(page, "Tier agents and budgets");
  const small = panel.getByLabel("Small tier");
  check("AC-222 a tier starts empty and offers only this project's pool",
    await small.inputValue() === "" && await small.locator("option").allTextContents().then((v) => v.join(",") === "None,API agent")
    && await panel.getByLabel("Fixes, % of weekly spend").inputValue() === "10");
  await small.selectOption("mine");
  await openGroup(page, "Critical line");
  const line = panel.locator('[data-section="Critical line"]');
  const test = line.getByRole("button", { name: "Send a test alert" });
  await test.waitFor();
  check("AC-227 the critical line names shared settings and starts without a channel",
    await test.isDisabled() && await line.getByText("Delivery settings apply to every project.").isVisible()
    && await line.getByLabel("Quiet hours start").inputValue() === "23:00"
    && await line.getByLabel("First repeat, minutes").inputValue() === "10");
  await shot(page, `supervisor-critical-line-${suffix}`);
  const paths = line.getByLabel("Channel executables", { exact: false });
  await paths.fill("/opt/example/channel-one\n/opt/example/channel-two");
  const saveLine = line.getByRole("button", { name: "Save line settings" });
  await saveLine.click();
  await line.getByText("Critical line settings saved.").waitFor();
  check("AC-227 the line saves ordered channels separately from project settings",
    calls.find(({ method, suffix }) => method === "PUT" && suffix === "/line").body.settings.channels.join(",")
      === "/opt/example/channel-one,/opt/example/channel-two" && await saveLine.evaluate((node) => node === document.activeElement));
  await line.getByLabel("Saved channel to test").selectOption("1");
  await test.click();
  await line.getByText("Exit status 17.").waitFor();
  await test.click();
  await line.getByText("The command accepted the test alert.", { exact: false }).waitFor();
  check("AC-229 the tab reports both outcomes for the selected saved channel",
    calls.filter(({ suffix }) => suffix === "/line/test").every(({ body }) => body.channel === 1 && body.revision === 1));
  await shot(page, `supervisor-line-result-${suffix}`);
  check("AC-229 the line controls fit and have touch-size buttons on a phone",
    await line.locator("button").evaluateAll((buttons, touch) => buttons.every((key) => {
      const box = key.getBoundingClientRect();
      return box.left >= 0 && box.right <= innerWidth && (!touch || (box.width >= 40 && box.height >= 40));
    }), phone));
  await openGroup(page, "Surface map");
  await panel.getByLabel("T3 paths, one pattern per line").fill("src/storage/**");
  check("AC-222 the surface preview says what it will count", await panel.getByRole("button", { name: "Preview last month" }).isDisabled()
    && await panel.getByText("The preview will count", { exact: false }).isVisible());
  await openGroup(page, "Charter and allocation");
  const priorities = panel.getByLabel("Priorities, highest first, one per line");
  await priorities.fill("First project priority\nSecond project priority");
  await panel.getByLabel("Review priorities on").fill("2027-01-15");
  await priorities.press("Enter");
  await priorities.fill("First project priority\nSecond project priority");
  check("AC-222 typing in a Supervisor field leaves the project window open",
    await page.isVisible("#projectbox") && !calls.some(({ method, suffix }) => method === "PUT" && suffix === "/settings"));
  await page.click("#supervisorok");
  await page.locator("#projectbox[data-busy]").waitFor();
  await page.keyboard.press("Escape");
  check("AC-222 the window waits while terminumd saves",
    await page.isVisible("#projectbox") && await page.locator("#projectsupervisor").isDisabled());
  refuseSave();
  check("AC-222 a server refusal is in the tab and preserves the draft",
    await until(async () => (await page.textContent("#supervisorsaveerror")).includes(RULE))
    && (await priorities.inputValue()).includes("Second project priority") && await small.inputValue() === "mine");
  await page.click("#supervisorok");
  await until(() => calls.filter(({ method, suffix }) => method === "PUT" && suffix === "/settings").length === 2);
  await page.locator("#projectbox:not([data-busy])").waitFor();
  const keep = calls.filter(({ method, suffix }) => method === "PUT" && suffix === "/settings").at(-1).body;
  check("AC-222 save round-trips priorities, pool IDs, surface patterns and unedited inherited settings",
    keep.settings.charter.priorities[1] === "Second project priority" && keep.settings.tiers.small === "mine"
    && keep.settings.charter.surfaces.t3[0] === "src/storage/**" && keep.settings.guardrails.threshold_percent === 54
    && JSON.stringify(keep.settings.detectors) === JSON.stringify(SETTINGS.detectors));
  check("AC-222 the review date survives a save in a timezone west of UTC",
    await panel.getByLabel("Review priorities on").inputValue() === "2027-01-15");
  await page.locator("#supervisoron").click();
  await until(() => page.getByText("Scan complete. Less than three weeks of history.").isVisible());
  await openGroup(page, "Detectors and mutes");
  await openGroup(page, RULE);
  const kind = panel.getByLabel("Threshold", { exact: true });
  await kind.selectOption("absolute");
  const limit = panel.getByLabel("Limit", { exact: true });
  await limit.fill("-1");
  await limit.press("Enter");
  check("AC-222 Enter in a threshold field keeps the draft open", await page.isVisible("#projectbox") && await limit.inputValue() === "-1");
  await page.click("#supervisorok");
  check("AC-222 a negative threshold identifies and focuses its field before a write",
    (await panel.locator(".sv-field-error").textContent()).includes(RULE) && await limit.getAttribute("aria-invalid") === "true"
    && await limit.evaluate((node) => node === document.activeElement)
    && calls.filter(({ suffix, method }) => suffix === "/settings" && method === "PUT").length === 2);
  if (phone) {
    check("AC-222 the tab and screen key fit a finger on a phone", await panel.getByRole("link", { name: "Open Supervisor screen" })
      .evaluate((key) => key.getBoundingClientRect().height >= 44 && key.getBoundingClientRect().width >= 44));
  }
  await openGroup(page, "Import and export");
  const file = panel.getByLabel("Settings file");
  await file.setInputFiles({ name: "supervisor-settings.json", mimeType: "application/json",
    buffer: Buffer.from(JSON.stringify({ settings: SETTINGS })) });
  await panel.getByRole("button", { name: "Import settings" }).click();
  await until(() => calls.some(({ suffix }) => suffix === "/import"));
  await openGroup(page, "Probe commands");
  const view = panel.getByRole("button", { name: "View command" });
  await view.waitFor();
  check("AC-220 AC-222 an import leaves the staged command unset and the local tier selected",
    !calls.some(({ suffix }) => suffix.endsWith("/confirm")) && await small.inputValue() === "mine");
  await view.click();
  const approve = panel.getByRole("button", { name: "Approve command" });
  await approve.waitFor();
  check("AC-220 AC-222 a user sees the whole command before approval",
    await panel.locator("pre").textContent() === COMMAND && !calls.some(({ suffix }) => suffix.endsWith("/confirm")));
  await approve.click();
  await panel.getByText("latency (approved)", { exact: true }).waitFor();
  const download = page.waitForEvent("download");
  await panel.getByRole("button", { name: "Export settings" }).click();
  check("AC-222 export downloads the project pack with the default redaction choices",
    (await download).suggestedFilename() === "supervisor-settings.json"
    && calls.some(({ suffix, body }) => suffix === "/export" && !body.include_commands && !body.include_secrets));
}

async function screen(page, suffix, phone) {
  await page.getByRole("link", { name: "Open Supervisor screen" }).click();
  await page.getByRole("heading", { name: "API service · Supervisor" }).waitFor();
  const holds = page.locator("#sv-holds article");
  const hold = holds.filter({ hasText: "Run stopped" });
  const integrationHold = holds.filter({ hasText: "Integration queue paused" });
  const incidentHold = page.locator("#sv-incidents [data-incident='42']");
  const keep = incidentHold.getByRole("button", { name: "Keep the hold" });
  const release = incidentHold.getByRole("button", { name: "Release the hold" });
  const undo = hold.getByRole("button", { name: "RUN", exact: true });
  await undo.waitFor();
  await keep.waitFor();
  await shot(page, `supervisor-holds-${suffix}`);
  check("AC-245 holds show their reason, signal and one undo with the incident's keys",
    await hold.getByText(LITERAL, { exact: true }).isVisible() && await hold.locator("img").count() === 0
    && await hold.getByRole("link", { name: /Signal/ }).getAttribute("href") === "?project=api&signal=0#signal-0"
    && await release.isVisible() && await integrationHold.getByRole("button", { name: "Resume integration" }).isVisible());
  if (phone) {
    for (const key of [undo, keep, release]) {
      check("AC-245 a hold key fits a finger and its card fits the phone",
        await key.evaluate((el) => el.getBoundingClientRect().height >= 44
          && document.documentElement.scrollWidth <= innerWidth));
    }
  }
  await keep.click();
  await release.click();
  await integrationHold.waitFor({ state: "detached" });
  check("AC-245 releasing from an incident also removes that hold from the same screen", await release.isHidden());
  await undo.click();
  check("AC-245 a failed release shows the server's reason beside the hold",
    await until(() => hold.getByRole("alert").isVisible()) && (await hold.getByRole("alert").textContent()).includes("queue is busy"));
  await undo.click();
  await page.getByText("No holds in effect.", { exact: true }).waitFor();
  check("AC-245 a successful release updates the list and focuses its heading",
    await until(() => page.locator("#sv-holds-title").evaluate((el) => el === document.activeElement)));
  await shot(page, `supervisor-screen-${suffix}`);
  const reports = page.locator("#sv-reports");
  await reports.getByText("Decisions waiting (1)", { exact: true }).click();
  await reports.getByText("Weekly review ·", { exact: false }).click();
  await reports.getByText("Exception digest ·", { exact: false }).click();
  await reports.getByRole("heading", { name: "Decisions waiting for you" }).waitFor();
  const useful = reports.getByRole("button", { name: "Useful", exact: true });
  await useful.click();
  check("AC-240 AC-242 AC-243 the Supervisor screen links decisions and reports and records a verdict",
    await until(async () => await useful.getAttribute("aria-pressed") === "true")
    && await reports.getByRole("link", { name: "API-41" }).getAttribute("href") === "/kanban/API-41"
    && await reports.getByRole("link", { name: "Open API-42 in Routines" }).isVisible()
    && await reports.getByText("Collector last ran", { exact: false }).isVisible()
    && await reports.getByRole("link", { name: "Findings" }).getAttribute("href") === "/supervisor/?project=api&signal=0#signal-0"
    && await reports.getByRole("button", { name: "Not useful" }).isEnabled()
    && await reports.getByRole("button", { name: "Wrong" }).isEnabled());
  if (phone) check("AC-242 verdict keys fit a finger on a phone", await useful.evaluate((key) => key.getBoundingClientRect().height >= 40));
  await shot(page, `supervisor-reports-${suffix}`);
  await page.locator("#sv-score summary").click();
  await page.locator("#sv-signals summary").first().click();
  const opened = page.locator("#sv-signals details[open]").first();
  await opened.getByRole("heading", { name: "Evidence", exact: true }).waitFor();
  check("AC-223 the scoreboard explains missing data and the signal has measures and literal evidence",
    await page.getByText("Not measured yet (release tags are unavailable)", { exact: true }).isVisible()
    && await page.getByText("Prior 3 · Latest 5 · Weekly noise 0.5", { exact: true }).isVisible()
    && await opened.getByRole("heading", { name: "Evidence", exact: true }).isVisible() && await opened.getByRole("link", { name: "API-2" }).isVisible()
    && (await opened.locator(".sv-evidence").textContent()).includes(LITERAL)
    && await page.locator("#sv-signals img").count() === 0 && !await page.evaluate(() => window.fixtureRan)
    && !await page.getByText("Web signal", { exact: false }).count());
  check("AC-229 the Supervisor screen spells out the channel and exit status in signal evidence",
    await page.getByText("Channel 2: Exit status 17.", { exact: true }).first().isVisible());
  const incident = page.locator("#sv-incidents [data-incident='42']");
  const incidentInvestigate = incident.getByRole("button", { name: "Investigate now", exact: true });
  await incidentInvestigate.click();
  check("AC-233 the incident card starts an investigation and reports the result",
    await until(async () => await incident.getByRole("status").textContent() === "Incident investigation requested."));
  if (phone) check("AC-233 the incident investigation key fits a finger on a phone",
    await incidentInvestigate.evaluate((node) => node.getBoundingClientRect().height >= 44));
  const talk = incident.getByRole("button", { name: "Talk", exact: true });
  await talk.click();
  check("AC-244 the incident task and the Talk refusal remain visible together",
    await until(() => incident.getByText("The marked pane changed.").isVisible())
    && await incident.getByRole("link", { name: "Thread" }).getAttribute("href") === "/kanban/API-40");
  await shot(page, `supervisor-investigation-${suffix}`);
  const investigate = opened.getByRole("button", { name: "Investigate now", exact: true });
  await investigate.click();
  check("AC-233 Investigate now requests a round for this signal",
    await until(async () => await opened.getByRole("status").textContent() === "Investigation requested."));
  await shot(page, `supervisor-evidence-${suffix}`);
  check("AC-223 evidence does not cause sideways scrolling",
    await investigate.isEnabled()
    && await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  if (phone) check("AC-223 a long signal's summary remains finger-sized",
    await page.locator("#sv-signals summary").first().evaluate((node) => node.getBoundingClientRect().height >= 44));
  const panel = page.locator("#sv-problems details").first();
  await panel.locator("summary").click();
  await page.locator("#sv-spending summary").first().click();
  await page.locator("#sv-spending summary").last().click();
  await panel.getByText("Failures stop without a change.").waitFor();
  await page.getByText("16,000 of 100,000 tokens used", { exact: false }).waitFor();
  await page.getByText("$10.25 of $20.00 used", { exact: false }).waitFor();
  check("AC-236 AC-237 AC-238 the screen shows the problem, protected yardstick and both spends",
    await panel.getByText("Failures stop without a change.").isVisible()
    && await panel.getByText("fixture-1", { exact: false }).isVisible()
    && await panel.getByRole("link", { name: "API-2" }).isVisible()
    && await panel.locator("img").count() === 0
    && await page.getByText("16,000 of 100,000 tokens used", { exact: false }).isVisible()
    && await page.getByText("$10.25 of $20.00 used", { exact: false }).isVisible()
    && await page.getByText("Auto-Dispatcher is set to exclude Supervisor tasks.").isVisible()
    && !/\bnull\b/.test(await page.locator("#sv-spending").textContent() + await panel.textContent()));
  await shot(page, `supervisor-register-${suffix}`);
  const accept = panel.getByRole("button", { name: "Accept as is" });
  if (phone) check("AC-236 the ruling button fits a finger and the register fits a phone",
    await accept.evaluate((key) => key.getBoundingClientRect().height >= 40)
    && await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  await panel.getByLabel("Reason to accept this problem as is").fill("Leave this condition until there is new evidence.");
  await accept.click();
  await panel.getByText("Owner ruling", { exact: false }).waitFor();
  check("AC-236 the owner ruling is recorded and the new status retains focus",
    await panel.locator("summary").evaluate((node) => node === document.activeElement && node.textContent.includes("Accepted as is")));
  await page.locator("#sv-signals summary").first().click();
  await opened.getByRole("heading", { name: "Evidence", exact: true }).waitFor();
  await page.click("#sv-more");
  check("AC-223 more signals can be read without losing the opened evidence",
    await until(() => page.getByText("Last signal", { exact: false }).isVisible())
    && await opened.getByRole("heading", { name: "Evidence", exact: true }).isVisible() && await page.isHidden("#sv-more"));
}

export async function webSupervisor(browser, engine) {
  for (const [size, device] of SIZES) {
    const phone = device.hasTouch;
    runningIn(`${engine} ${size}`);
    const suffix = `${engine}-${phone ? "phone" : "wide"}`;
    const theme = phone ? { ...THEME, bg: "#ffffff", fg: "#1f2328", dark: false } : THEME;
    const f = await openPage(browser, { device: { ...device, timezoneId: "America/Los_Angeles" }, hook(m, say) {
      if (m.t === "sub") say({ t: "theme", sid: m.id, theme });
    } });
    try {
      await drawn(f.page);
      const settings = await fixture(f.page, theme);
      await f.page.goto(`${ORIGIN}/kanban/?project=api`);
      await f.page.waitForFunction(() => document.body.dataset.socket === "open");
      await settingsTab(f.page, settings, suffix, phone);
      await screen(f.page, suffix, phone);
      check("AC-222 AC-223 no page error", f.errors.length === 0, f.errors.join("\n"));
    } catch (e) {
      await shot(f.page, `supervisor-failure-${suffix}`);
      throw new Error(`${e.message} | page errors: ${f.errors.join("; ")}`, { cause: e });
    } finally { await f.close(); }
  }
}
