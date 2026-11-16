// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum-integration.md AC-193: the integration queue's keys on the board's cards, against a fake terminumd: a card's
// menu (Integrate, Take out, a red batch's Return…, Check again and Return all), an IN PROGRESS card dropped on
// INTEGRATION's tab, a queued card dropped on IN PROGRESS's, other columns taking no drop, and a refusal in a toast.
import fs from "node:fs";
import { check, runningIn, until } from "./check.mjs";
import { drawn, group, item, openPage, ORIGIN, SIZES, THEME } from "./fake.mjs";

const STATUSES = ["TODO", "IN PROGRESS", "WAIT FOR HUMAN", "INTEGRATION", "FAILED", "DONE"];
const task = (n, status, more = {}) => ({ id: `API-${n}`, uid: "abcd1234", title: `Work ${n}`, project: "api", severity: "P2", status, text: "", created_at: 1,
  updated_at: 1, files: [], history: [], children: [], parent: null, session: null, ...more });

export async function webCardKeys(browser, name) {
  for (const [size, device] of SIZES) for (const light of [false, true]) {
    const mode = light ? "light" : "dark", theme = light ? { ...THEME, bg: "#ffffff", fg: "#1f2328", dark: false } : THEME;
    runningIn(`${name} ${size} ${mode}`);
    const tasks = [task(1, "IN PROGRESS"), task(2, "INTEGRATION", { queue: { mark: "1st" } }),
      task(3, "INTEGRATION", { queue: { mark: "in batch 7", batch: 7, batch_id: 42, keys: ["check-again", "return", "return-all"] } }),
      task(4, "IN PROGRESS", { project: "web" }), task(5, "TODO")];
    const meta = { projects: [{ id: "api", name: "API", color: 0, dir: "/workspace", integration: true }, { id: "web", name: "Web", color: 1, dir: "/web" }],
      users: [], hosts: [], theme, agents: [], sessions: [] };
    const f = await openPage(browser, { device, layout: [group("main", [item("tmux:local:terminum:%1", "a shell")])], secret: "a".repeat(64), stored: "a".repeat(64),
      hook: (m, say) => { if (m.t === "sub") say({ t: "theme", sid: m.id, theme }); } });
    const { page } = f, json = (r, data, status = 200) => r.fulfill({ status, contentType: "application/json", body: JSON.stringify(data) });
    const posted = [];
    let refuse = null;   // the next key's refusal, in terminumd's words
    try {
      await drawn(page);
      await page.route(`${ORIGIN}/kanban/**`, (r) => r.fulfill({ contentType: "text/html", body: fs.readFileSync(new URL("../static/kanban.html", import.meta.url), "utf8").replace("__BUILD__", "test") }));
      await page.route("**/api/tasks/meta", (r) => json(r, meta));
      await page.route("**/api/schedules", (r) => json(r, { schedules: [] }));
      await page.route("**/api/tasks?*", (r) => json(r, { tasks: tasks.filter((t) => t.status === new URL(r.request().url()).searchParams.get("status")),
        counts: Object.fromEntries(STATUSES.map((s) => [s, tasks.filter((t) => t.status === s).length])), slots: { agents: {}, full: [], full_projects: [], kept: {}, queue: {} } }));
      await page.route("**/api/integration/**", (r) => {
        const u = new URL(r.request().url()).pathname;
        posted.push({ u, body: r.request().postDataJSON() });
        if (refuse) { const error = refuse; refuse = null; return json(r, { error }, 400); }
        return json(r, { ok: true, said: u.endsWith("/queue") ? "API-1 is queued 1st." : u.includes("/batch/") ? "Batch 7: done." : undefined });
      });
      await page.goto(`${ORIGIN}/kanban/?project=api,web`);
      await page.waitForSelector('.card[data-id="API-1"]', { state: "attached" });
      const tab = (s) => `#colswitch button[data-status="${s}"]`, card = (id) => `.card[data-id="${id}"]`;
      const show = async (status) => { if (await page.isVisible(tab(status))) await page.click(tab(status)); await page.waitForSelector(`.col[data-status="${status}"] .card`); };
      const menuOf = async (id) => {
        await page.hover(card(id)).catch(() => {});
        await page.click(`${card(id)} .cmenu`);
        await page.waitForSelector("#cardmenu[open]");
        return page.$$eval("#cardmenu [role=menuitem]", (bs) => bs.map((b) => b.textContent.trim()));
      };
      const pick = async (words) => { await page.click(`#cardmenu [role=menuitem]:has-text("${words}")`); };
      const toastSays = (words) => until(async () => (await page.textContent("#toast").catch(() => "")).includes(words));

      await show("IN PROGRESS");
      check("AC-193 a card of a project without integration has no menu", !(await page.$(`${card("API-4")} .cmenu`)));
      check("AC-193 an IN PROGRESS card's menu holds Integrate", JSON.stringify(await menuOf("API-1")) === '["Integrate"]');
      await pick("Integrate");
      check("AC-193 Integrate queues it, said in a toast", await until(() => posted.some((p) => p.u === "/api/integration/task/API-1/queue")) && await toastSays("API-1 is queued 1st."));
      check("AC-193 the menu closes on a press", !(await page.isVisible("#cardmenu")));
      await menuOf("API-1");
      if (device.hasTouch) {   // a finger's targets (docs/UI-GUIDELINES.md §1)
        const sizes = await page.evaluate(() => [document.querySelector('.card[data-id="API-1"] .cmenu'), ...document.querySelectorAll("#cardmenu [role=menuitem]")]
          .map((b) => { const r = b.getBoundingClientRect(); return [Math.round(r.width), Math.round(r.height)]; }));
        check("AC-193 on a phone the ⋯ key and the menu's items take a finger", sizes.every(([w, h]) => w >= 40 && h >= 40), JSON.stringify(sizes));
      }
      if (process.env.TM_TEST_SHOTS) {
        fs.mkdirSync(process.env.TM_TEST_SHOTS, { recursive: true });
        await page.screenshot({ path: `${process.env.TM_TEST_SHOTS}/cardmenu-${name}-${size}-${mode}.png` });
      }
      await page.keyboard.press("Escape");

      await show("INTEGRATION");
      check("AC-193 a queued card's menu holds Take out", JSON.stringify(await menuOf("API-2")) === '["Take out"]');
      refuse = "API-2 is not queued.";
      await pick("Take out");
      check("AC-193 a refused Take out says terminumd's words in a toast", await toastSays("API-2 is not queued."));

      check("AC-193 a red batch's card holds Take out, Check again, Return… and Return all",
        JSON.stringify(await menuOf("API-3")) === '["Take out","Check again","Return API-3…","Return all"]');
      await pick("Check again");
      check("AC-193 Check again presses its batch's key", await until(() => posted.some((p) => p.u === "/api/integration/batch/42/check-again")));
      await menuOf("API-3");
      await pick("Return API-3…");
      await page.waitForSelector("#namebox[open]");
      await page.fill("#namefield", "breaks the login");
      await page.click("#nameok");
      check("AC-193 Return… asks a line of why and sends it with the task",
        await until(() => posted.some((p) => p.u === "/api/integration/batch/42/return" && p.body?.task === "API-3" && p.body?.why === "breaks the login")));
      await menuOf("API-3");
      await page.keyboard.press("Escape");
      check("AC-193 Esc closes the card's menu", await until(async () => !(await page.isVisible("#cardmenu"))));

      // the drag's own events: Playwright's WebKit carries no drag through the mouse alone
      const drop = (from, on) => page.evaluate(([from, on]) => {
        const dt = new DataTransfer(), held = document.querySelector(from), zone = document.querySelector(on), r = zone.getBoundingClientRect();
        const at = { bubbles: true, cancelable: true, dataTransfer: dt, clientX: r.left + 4, clientY: r.top + 4 };
        held.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: dt }));
        const over = new DragEvent("dragover", at);
        zone.dispatchEvent(over);
        const lit = zone.classList.contains("over");
        if (over.defaultPrevented) zone.dispatchEvent(new DragEvent("drop", at));
        held.dispatchEvent(new DragEvent("dragend", { bubbles: true, dataTransfer: dt }));
        return { taken: over.defaultPrevented, lit };
      }, [from, on]);
      const before = posted.length;
      await show("IN PROGRESS");
      const on = await drop(card("API-1"), tab("INTEGRATION"));
      check("AC-193 an IN PROGRESS card dropped on INTEGRATION's tab is queued",
        on.taken && on.lit && await until(() => posted.slice(before).some((p) => p.u === "/api/integration/task/API-1/queue")), JSON.stringify(on));
      const todo = await drop(card("API-1"), tab("DONE"));
      check("AC-107 another column takes no drop of it", !todo.taken && !todo.lit, JSON.stringify(todo));
      await show("INTEGRATION");
      const off = await drop(card("API-2"), tab("IN PROGRESS"));
      check("AC-193 a queued card dropped on IN PROGRESS's tab is taken out",
        off.taken && await until(() => posted.some((p) => p.u === "/api/integration/task/API-2/out")), JSON.stringify(off));
      const elsewhere = await drop(card("API-2"), tab("FAILED"));
      check("AC-193 a queued card takes no drop on FAILED", !elsewhere.taken, JSON.stringify(elsewhere));
      refuse = "API-3 is a batch's task: it ends with its batch.";
      await drop(card("API-3"), tab("IN PROGRESS"));
      check("AC-193 a refused drop off INTEGRATION says terminumd's words and moves nothing",
        await toastSays("ends with its batch") && await page.$eval(card("API-3"), (c) => c.closest(".col").dataset.status) === "INTEGRATION");
      check("AC-193 nothing stays lit after a drop", await page.evaluate(() => !document.querySelector(".over")));
      check("no page errors", f.errors.length === 0, f.errors.join(" | "));
    } finally { await f.close(); }
  }
}
