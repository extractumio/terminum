// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-138, AC-169: integration sessions stay visible and openable without occupying slots.
import fs from "node:fs";
import path from "node:path";
import { check, runningIn, until } from "./check.mjs";
import { drawn, group, item, openPage, ORIGIN, SIZES, THEME } from "./fake.mjs";

export async function webIntegrationSlots(browser, name) {
  for (const [size, device] of SIZES) for (const light of [false, true]) {
    const mode = light ? "light" : "dark";
    runningIn(`${name} ${size} ${mode}`);
    const theme = light ? { ...THEME, bg: "#ffffff", fg: "#1f2328", dark: false } : THEME;
    const keys = Array.from({ length: 5 }, (_, i) => `tmux:local:terminum:%${i + 1}`);
    const tasks = keys.map((key, i) => ({ id: `API-${i + 1}`, uid: "abcd1234", title: `Integration work ${i + 1}`, project: "api", severity: "P2",
      status: i < 4 ? "INTEGRATION" : "IN PROGRESS", text: "", created_at: 1, updated_at: 1, files: [], history: [], children: [], parent: null, session: { key, closed: false } }));
    const sessions = tasks.map((t, i) => ({ run: `r${i}`, tasks: t.id, state: "working", key: keys[i], project: "api", idle: false,
      integration: i < 4, round: false, kept: false, list: [t] }));
    const agent = { id: "a1", project: "api", name: "Codex", kind: "codex", command: "codex", line: "codex", color: 0, at_once: 3, spent: { week: 0, month: 0 } };
    const meta = { projects: [{ id: "api", name: "API", color: 0, dir: "/workspace" }], users: [], hosts: [], theme,
      agents: [agent, { ...agent, id: "a2", name: "Idle comparison", color: 1 }], sessions: keys.map((key) => ({ key, title: key })) };
    const layout = [group("main", keys.map((key, i) => item(key, tasks[i].title, { agent: "codex", shell: false, state: i < 4 ? "done" : "working" })))];
    const f = await openPage(browser, { device, layout, secret: "a".repeat(64), stored: "a".repeat(64),
      hook: (m, say) => { if (m.t === "sub") say({ t: "theme", sid: m.id, theme }); } });
    const { page } = f, json = (r, data) => r.fulfill({ contentType: "application/json", body: JSON.stringify(data) });
    try {
      await drawn(page);
      await page.route(`${ORIGIN}/kanban/**`, (r) => r.fulfill({ contentType: "text/html", body: fs.readFileSync(new URL("../static/kanban.html", import.meta.url), "utf8").replace("__BUILD__", "test") }));
      await page.route("**/api/tasks/meta", (r) => json(r, meta));
      await page.route("**/api/schedules", (r) => json(r, { schedules: [] }));
      await page.route("**/api/tasks?*", (r) => json(r, { tasks: tasks.filter((t) => t.status === new URL(r.request().url()).searchParams.get("status")),
        counts: Object.fromEntries(["TODO", "IN PROGRESS", "WAIT FOR HUMAN", "INTEGRATION", "FAILED", "DONE"].map((s) => [s, tasks.filter((t) => t.status === s).length])),
        slots: { agents: { a1: sessions }, full: [], full_projects: [], kept: {}, queue: {} } }));
      await page.goto(`${ORIGIN}/kanban/?project=api`);
      if (device.hasTouch) await page.click("#poolline");
      const card = '#agentbar .agentcard[data-agent="a1"]';
      await page.waitForSelector(card);
      check("AC-138 all five live sessions keep their dots", await page.locator(`${card} .agdot`).count() === 5);
      check("AC-169 four integration waits stay pale and leave two free slots", await page.locator(`${card} .agdot.idle`).count() === 4
        && await page.locator(`${card} .agfree`).count() === 2 && await page.textContent(`${card} .agcount`) === "1/3");
      if (process.env.TM_TEST_SHOTS) {
        fs.mkdirSync(process.env.TM_TEST_SHOTS, { recursive: true });
        await page.locator("#agentbar .agentlist").screenshot({ path: path.join(process.env.TM_TEST_SHOTS, `integration-slots-${name}-${size}-${mode}.png`) });
      }
      await page.click(`${card} .agedit`);
      await page.waitForSelector("#agentbox[open]");
      check("AC-138 every live session remains openable in the agent window", await page.locator("#agentbox .agsession").count() === 5
        && await page.locator("#agentbox").getByRole("button", { name: "Open session", exact: true }).count() === 5);
      check("AC-169 the window names each integration wait", await page.locator("#agentbox .agsessname").filter({ hasText: "waiting for integration" }).count() === 4);
      await page.evaluate(() => document.activeElement?.blur());
      await page.locator("#agentbox").evaluate((box) => { box.scrollTop = 0; box.querySelector("form").scrollTop = 0; });
      await page.waitForFunction(() => document.querySelector("#agentbox").scrollTop === 0 && document.querySelector("#agentbox form").scrollTop === 0);
      check("AC-138 the retained-session view fits the viewport", await page.evaluate(() => {
        const box = document.querySelector("#agentbox");
        return box.scrollWidth <= box.clientWidth + 1 && document.documentElement.scrollWidth <= innerWidth + 1;
      }));
      if (process.env.TM_TEST_SHOTS) await page.locator("#agentbox").screenshot({ path: path.join(process.env.TM_TEST_SHOTS, `integration-window-${name}-${size}-${mode}.png`) });
      await page.keyboard.press("Escape");
      check("AC-138 Esc closes the unchanged agent window", await until(async () => !(await page.isVisible("#agentbox"))));
      check("no page errors", f.errors.length === 0, f.errors.join(" | "));
    } finally { await f.close(); }
  }
}
