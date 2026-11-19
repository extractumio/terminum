// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum-schedules.md AC-161: an agent's card shows the slots it keeps as K dashed rings for its schedules, naming
// none of them, and one for integration; they count against its at once.
import fs from "node:fs";
import { check, runningIn } from "./check.mjs";
import { drawn, group, item, openPage, ORIGIN, SIZES, THEME } from "./fake.mjs";

export async function webKanbanKept(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const task = { id: "S_API-9", uid: "abcd1234", title: "Nightly deps check", project: "api", severity: "P2", status: "IN PROGRESS", text: "",
      created_at: 1, updated_at: 1, files: [], history: [], children: [], parent: null, session: { key: "tmux:local:terminum:%1", closed: false } };
    const sessions = [{ run: "r1", tasks: task.id, state: "working", key: task.session.key, project: "api", idle: false, integration: false, round: false, kept: true, list: [task] }];
    const agent = { id: "a1", project: "api", name: "Opus reviewer", kind: "claude", command: "claude", line: "claude", color: 0, at_once: 5, spent: { week: 0, month: 0 } };
    const meta = { projects: [{ id: "api", name: "API", color: 0, dir: "/workspace" }], users: [], hosts: [], theme: THEME, agents: [agent, { ...agent, id: "a2", name: "Plain", color: 1, at_once: 2 }], sessions: [{ key: task.session.key, title: task.title }] };
    const layout = [group("main", [item(task.session.key, task.title, { agent: "claude", shell: false, state: "working" })])];
    const f = await openPage(browser, { device, layout, secret: "a".repeat(64), stored: "a".repeat(64),
      hook: (m, say) => { if (m.t === "sub") say({ t: "theme", sid: m.id, theme: THEME }); } });
    const { page } = f, json = (r, data) => r.fulfill({ contentType: "application/json", body: JSON.stringify(data) });
    try {
      await drawn(page);
      await page.route(`${ORIGIN}/kanban/**`, (r) => r.fulfill({ contentType: "text/html", body: fs.readFileSync(new URL("../static/kanban.html", import.meta.url), "utf8").replace("__BUILD__", "test") }));
      await page.route("**/api/tasks/meta", (r) => json(r, meta));
      await page.route("**/api/schedules", (r) => json(r, { schedules: [] }));
      await page.route("**/api/tasks?*", (r) => json(r, { tasks: new URL(r.request().url()).searchParams.get("status") === "IN PROGRESS" ? [task] : [],
        counts: { TODO: 0, "IN PROGRESS": 1, "WAIT FOR HUMAN": 0, INTEGRATION: 0, FAILED: 0, DONE: 0 },
        slots: { agents: { a1: sessions }, full: [], full_projects: [], kept: { a1: { slots: 2, schedules: 3, integration: true } }, queue: {} } }));
      await page.goto(`${ORIGIN}/kanban/?project=api`);
      if (device.hasTouch) await page.click("#poolline");
      const card = '#agentbar .agentcard[data-agent="a1"]';
      await page.waitForSelector(`${card} .agkept`);
      const rings = await page.$$eval(`${card} .agkept`, (all) => all.map((m) => m.title));
      check("AC-161 K dashed rings for its schedules, naming none, then one for integration", JSON.stringify(rings) === JSON.stringify(["kept for 3 schedules", "kept for 3 schedules", "kept for integration"]), JSON.stringify(rings));
      check("AC-161 the run standing in a kept slot counts once: 3 of 5, two free", await page.textContent(`${card} .agcount`) === "3/5" && await page.locator(`${card} .agfree`).count() === 2,
        await page.textContent(`${card} .agcount`));
      const title = await page.getAttribute(card, "title");
      check("AC-161 the card's hover says each kind once", title.split("kept for 3 schedules").length === 2 && title.includes("kept for integration"), title);
      check("AC-161 an agent keeping nothing shows no ring", await page.locator('#agentbar .agentcard[data-agent="a2"] .agkept').count() === 0);
      check("no page errors", f.errors.length === 0, f.errors.join(" | "));
    } finally { await f.close(); }
  }
}
