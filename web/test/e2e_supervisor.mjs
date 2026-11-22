// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { check, runningIn, until } from "./check.mjs";
import { drawn, fakeBoard, openPage, ORIGIN, SIZES, shot, THEME } from "./fake.mjs";

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
const SCORE = { name: "Release cadence", verdict: "not measured yet", guard: "Stability", metrics: [
  { name: "Releases per month", verdict: "not measured yet", missing: "release tags are unavailable" },
  { name: "Fixture measured series", verdict: "better", missing: null, before: 3, current: 5, noise: 0.5 },
] };
const json = (route, obj, status = 200) => route.fulfill({ status, json: obj });

async function fixture(page, theme) {
  const projects = [{ id: "api", name: "API service", color: 0 }, { id: "web", name: "Web service", color: 1 }];
  const agents = [{ project: "api", id: "mine", name: "API agent" }, { project: "web", id: "foreign", name: "Web agent" }];
  await fakeBoard(page, { meta: () => ({ projects, agents, theme, users: [], hosts: [], sessions: [] }), tasks: () => [], counts: {} });
  await page.route("**/api/messages/meta", (route) => json(route, { projects, theme }));
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
    if (suffix === "/screen") return json(route, { project_name: "API service", indicators: [SCORE],
      signals: Array.from({ length: 50 }, (_, i) => ({ ...SIGNAL, fingerprint: String(i), metric: i ? `Signal ${i}` : SIGNAL.metric })), next: "49" });
    if (suffix === "/signals") {
      return searchParams.get("after") === "49"
        ? json(route, { signals: [{ ...SIGNAL, metric: "Last signal" }], next: null })
        : json(route, { error: "The signal page expired." }, 400);
    }
    if (suffix === "") {
      if (method === "PUT") enabled = body.enabled;
      const attachment = enabled ? { limited_history: true, detectors: { [RULE]: { can_run: true, mode: "shadow", unavailable: [],
        observed_thresholds: { failure_count: { min: 2, max: 3 } } } } } : null;
      return json(route, { enabled, attachment });
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
  await shot(page, `supervisor-screen-${suffix}`);
  await page.locator("#sv-score summary").click();
  await page.locator("#sv-signals summary").first().click();
  const opened = page.locator("#sv-signals details[open]").first();
  await opened.getByRole("heading", { name: "Evidence", exact: true }).waitFor();
  check("AC-223 the scoreboard explains missing data and the signal has measures and literal evidence",
    await page.getByText("Not measured yet (release tags are unavailable)", { exact: true }).isVisible()
    && await page.getByText("Prior 3 · Latest 5 · Weekly noise 0.5", { exact: true }).isVisible()
    && await opened.getByRole("heading", { name: "Evidence", exact: true }).isVisible() && await opened.getByRole("link", { name: "API-2" }).isVisible()
    && (await page.locator(".sv-evidence").first().textContent()).includes(LITERAL)
    && await page.locator("#sv-signals img").count() === 0 && !await page.evaluate(() => window.fixtureRan)
    && !await page.getByText("Web signal", { exact: false }).count());
  check("AC-229 the Supervisor screen spells out the channel and exit status in signal evidence",
    await page.getByText("Channel 2: Exit status 17.", { exact: true }).first().isVisible());
  await shot(page, `supervisor-evidence-${suffix}`);
  check("AC-223 investigation shows its availability and evidence does not cause sideways scrolling",
    await page.getByRole("button", { name: "Investigate", exact: true }).first().isDisabled()
    && await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  if (phone) check("AC-223 a long signal's summary remains finger-sized",
    await page.locator("#sv-signals summary").first().evaluate((node) => node.getBoundingClientRect().height >= 44));
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
