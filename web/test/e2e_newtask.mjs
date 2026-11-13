// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-106: a new task leaves agent selection to assignment; explicit choices survive refresh.
import fs from "node:fs";
import path from "node:path";
import { check, runningIn, until } from "./check.mjs";
import { drawn, openPage, PHONE, SIZES, THEME } from "./fake.mjs";

const AGENTS = [{ id: "a1", project: "web", name: "Claude Code" }, { id: "a2", project: "api", name: "Codex" }];
const META = { projects: [{ id: "api", name: "API" }, { id: "web", name: "Web" }],
  users: [{ id: "human", name: "Alex" }], sessions: [], agents: AGENTS, last_agents: { api: "a2", web: "a1" } };

export async function webNewTask(browser, name) {
  for (const [size, device] of SIZES) for (const light of [false, true]) {
    runningIn(`${name} ${size} ${light ? "light" : "dark"}`);
    const f = await openPage(browser, { device, hook: (m, say) => {
      if (light && m.t === "sub") say({ t: "theme", sid: m.id, theme: { ...THEME, bg: "#ffffff", fg: "#1f2328", dark: false } });
    } });
    const { page } = f, sent = [];
    let refuse = false;
    try {
      await drawn(page);
      await page.route("**/api/tasks", (r) => {
        const body = r.request().postDataJSON();
        sent.push(body);
        return r.fulfill({ status: refuse ? 400 : 200, contentType: "application/json", body: JSON.stringify(refuse ? { error: "Creation refused." } : { ...body, id: `API-${sent.length}` }) });
      });
      await page.evaluate(async (meta) => {
        const html = await (await fetch("/kanban.html")).text();
        document.body.append(new DOMParser().parseFromString(html, "text/html").querySelector("#newbox"));
        const { taskForm } = await import("/kanbanform.js");
        window.newmeta = meta;
        window.newruns = [];
        window.newform = taskForm({ getMeta: () => window.newmeta, getFilter: () => ({ project: ["api"] }),
          onMade() {}, onRun: (task, agent) => window.newruns.push(agent), slots: () => ({ full: [], full_projects: [] }) });
        window.newform.open();
      }, META);
      check("AC-106 new task starts unspecified despite a last-used agent", await page.inputValue("#newagent") === "");
      await page.selectOption("#newproj", "web");
      check("AC-106 changing project keeps the agent unspecified", await page.inputValue("#newagent") === "");
      const offered = await page.$$eval("#newagent option", (all) => all.map((o) => o.value));
      check("AC-137 a new task is offered its project's own agents alone", JSON.stringify(offered) === JSON.stringify(["", "a1"]), JSON.stringify(offered));
      await page.selectOption("#newagent", "a1");
      await page.evaluate(() => window.newform.refresh());
      check("AC-106 an explicit agent survives refresh", await page.inputValue("#newagent") === "a1");
      await page.evaluate(() => { window.newmeta.agents = window.newmeta.agents.filter((a) => a.id !== "a1"); window.newform.refresh(); });
      check("AC-106 a removed agent returns to unspecified", await page.inputValue("#newagent") === "");
      await page.fill("#newtitle", "Choose the agent later");
      refuse = true;
      await page.click("#newcreate");
      check("AC-106 a refused creation keeps its title and unspecified agent", await until(() => page.isVisible("#newerror")) && await page.textContent("#newerror") === "Creation refused." && await page.inputValue("#newtitle") === "Choose the agent later" && await page.inputValue("#newagent") === "");
      refuse = false;
      await page.click("#newcreate");
      await page.waitForSelector("#newbox[open]", { state: "detached" });
      check("AC-106 Create submits no removed or default agent", !Object.hasOwn(sent.at(-1), "agent"));
      await page.evaluate((agents) => { window.newmeta.agents = agents; window.newform.open(); }, AGENTS);
      check("AC-106 another new task also starts unspecified", await page.inputValue("#newagent") === "");
      if (process.env.TM_TEST_SHOTS) {
        fs.mkdirSync(process.env.TM_TEST_SHOTS, { recursive: true });
        await page.screenshot({ path: path.join(process.env.TM_TEST_SHOTS, `newtask-${name}-${device === PHONE ? "phone" : "wide"}-${light ? "light" : "dark"}.png`) });
      }
      await page.selectOption("#newagent", "a2");
      await page.fill("#newtitle", "Use the agent I chose");
      await page.click("#newcreate");
      await page.waitForSelector("#newbox[open]", { state: "detached" });
      check("AC-106 Create keeps an explicit agent", sent.at(-1).agent === "a2");
      await page.evaluate(() => window.newform.open());
      await page.fill("#newtitle", "Choose when Run assigns it");
      await page.click("#newrun");
      check("AC-106 Run leaves assignment to its existing chooser", await until(() => page.evaluate(() => window.newruns.length === 1)) && await page.evaluate(() => window.newruns[0] === "") && !Object.hasOwn(sent.at(-1), "agent"));
      check("no page errors", !f.errors.length, f.errors.join(" | "));
    } finally { await f.close(); }
  }
}
