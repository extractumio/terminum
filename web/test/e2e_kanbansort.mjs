// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Column sorting, preference scope, failed and late reads, and the header's look (AC-105).
import fs from "node:fs";
import path from "node:path";
import { check, runningIn, until } from "./check.mjs";
import { drawn, openPage, ORIGIN, PHONE, SIZES, THEME } from "./fake.mjs";
import { tabKey } from "./boardkit.mjs";

const COL = '.col[data-status="TODO"]', RECENT = `${COL} [data-sort="recent"]`, PRIORITY = `${COL} [data-sort="priority"]`;
const tasks = Array.from({ length: 205 }, (_, i) => ({
  id: `API-${i + 1}`, title: `Task ${i + 1}`, project: "api", status: "TODO", severity: i ? "P2" : "P0",
  owner: "human", owner_name: "Alex", updated_at: i, parent: null,
}));
const counts = { TODO: 205, "IN PROGRESS": 0, "WAIT FOR HUMAN": 0, FAILED: 0, DONE: 0 };
const META = { projects: [{ id: "api", name: "API", color: 0 }, { id: "web", name: "Web", color: 1 }], users: [], sessions: [], agents: [], hosts: [], theme: THEME };

export async function webKanbanSort(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device, secret: "a".repeat(64), stored: "a".repeat(64) });
    const { page } = f;
    await drawn(page);
    let fail = false, hold = null, light = false;
    const pending = [], queries = [];
    const json = (r, data, status = 200) => r.fulfill({ status, contentType: "application/json", body: JSON.stringify(data) });
    await page.route(`${ORIGIN}/kanban/**`, (r) => r.fulfill({ contentType: "text/html", body: fs.readFileSync(new URL("../static/kanban.html", import.meta.url), "utf8").replace("__BUILD__", "test") }));
    await page.route("**/api/tasks/meta", (r) => json(r, { ...META, theme: light ? { ...THEME, bg: "#ffffff", fg: "#1f2328", cursor: "#222222", dark: false } : THEME }));
    await page.route("**/api/schedules", (r) => json(r, { schedules: [] }));
    await page.route("**/api/tasks?*", async (r) => {
      const q = new URL(r.request().url()).searchParams;
      const sort = q.get("sort"), after = Number(q.get("after") ?? 0), column = q.get("status"), project = q.get("project");
      queries.push({ sort, after, column, project });
      const selected = column === "TODO" ? [...tasks].sort((a, b) => (sort === "priority" ? a.severity.localeCompare(b.severity) : 0) || b.updated_at - a.updated_at) : [];
      const data = { tasks: selected.slice(after, after + 200).map((t) => project === "web" ? { ...t, id: t.id.replace("API-", "WEB-"), project: "web" } : t), counts, slots: { agents: {}, full: [], full_projects: [] } };
      if (column === "TODO" && hold?.({ sort, after, project })) await new Promise((done) => pending.push(done));
      return fail && column === "TODO" ? json(r, { error: "Sort read refused." }, 503) : json(r, data);
    });
    const go = async (project = "api") => {
      await page.goto(`${ORIGIN}/kanban/${project ? `?project=${project}` : ""}`);
      await page.waitForSelector(`${COL} .card`);
    };
    const ids = () => page.$$eval(`${COL} .card`, (l) => l.map((c) => c.dataset.id));
    const first = async (id) => until(async () => (await ids())[0] === id);
    const choose = async (words) => { await page.click("#fproject summary"); await (words === "All projects" ? page.getByRole("button", { name: words, exact: true }) : page.locator("#fproject .projname").filter({ hasText: words })).click(); };
    try {
      await go();
      check("AC-105 default priority then recent; exactly two labelled keys per column", await first("API-1") && await page.locator('.colsort button').count() === 10 && await page.getAttribute(PRIORITY, "aria-pressed") === "true");
      const head = await page.$$eval(".colhead", (heads) => heads.map((h) => {
        const sort = h.querySelector(".colsort"), count = h.querySelector(".count"), box = h.getBoundingClientRect(), s = sort.getBoundingClientRect(), c = count.getBoundingClientRect();
        return { before: sort.nextElementSibling === count && s.right <= c.left, fits: s.top >= box.top && s.bottom <= box.bottom, keys: [...sort.children].map((b) => { const r = b.getBoundingClientRect(); return [r.width, r.height]; }) };
      }));
      check("AC-105 sort keys stand immediately left of each count, inside the header", head.every((h) => h.before && h.fits), JSON.stringify(head));
      if (device === PHONE) check("AC-105 phone sort keys are at least 44 by 44", head.every((h) => h.keys.every(([w, h]) => w >= 44 && h >= 44)), JSON.stringify(head));
      await page.focus(RECENT);
      await tabKey(page);
      check("AC-105 Tab reaches the next sort icon with visible focus", await page.$eval(PRIORITY, (b) => document.activeElement === b && b.matches(":focus-visible") && getComputedStyle(b).outlineStyle !== "none"));
      await page.click(RECENT);
      check("AC-105 recently updated first changes TODO alone", await first("API-205") && await page.getAttribute('.col[data-status="DONE"] [data-sort="priority"]', "aria-pressed") === "true");
      await page.locator(`${COL} .colmore`).click();
      check("AC-105 Show more sends the same sort and appends without duplicates", await until(async () => (await ids()).length === 205) && new Set(await ids()).size === 205 && queries.some((q) => q.sort === "recent" && q.after === 200));
      await go();
      check("AC-105 reload keeps this project's column preference", await first("API-205") && await page.getAttribute(RECENT, "aria-pressed") === "true");
      await choose("Web");
      check("AC-105 a different project's default is independent", await first("WEB-1") && await page.getAttribute(PRIORITY, "aria-pressed") === "true");
      await choose("API");
      check("AC-105 switching back restores that project's sort", await first("API-205"));
      await choose("All projects");
      check("AC-105 All projects has its own preferences", await first("API-1"));
      await page.click(RECENT);
      await first("API-205");
      await go("api,web");
      check("AC-105 multiple projects share All projects' sort", await first("API-205") && await page.getAttribute(RECENT, "aria-pressed") === "true");
      await go();
      fail = true;
      await choose("Web");
      await page.waitForSelector("#boardbanner:not([hidden])");
      check("AC-105 a failed project switch cannot page into the wrong project", (await ids())[0] === "API-205" && await page.isDisabled(`${COL} .colmore`));
      fail = false;
      await choose("API");
      await page.waitForSelector("#boardbanner[hidden]", { state: "attached" });
      // Failure leaves old cards visible; their offset must never be used with the new sort/project.
      fail = true;
      await page.click(PRIORITY);
      await page.waitForSelector("#boardbanner:not([hidden])");
      check("AC-105 a failed sort keeps cards, says why and disables Show more", (await ids())[0] === "API-205" && (await page.textContent("#boardbanner")).includes("Sort read refused.") && await page.isDisabled(`${COL} .colmore`));
      fail = false;
      await page.click(RECENT);
      await page.waitForSelector("#boardbanner[hidden]", { state: "attached" });
      // An older Show more must add nothing after a sort change.
      hold = ({ after }) => after > 0;
      await page.click(`${COL} .colmore`);
      await until(() => pending.length > 0);
      await page.click(PRIORITY);
      await first("API-1");
      hold = null;
      pending.splice(0).forEach((done) => done());
      await page.waitForTimeout(100);
      check("AC-105 late Show more cannot append cards from the old sort", (await ids()).length === 200 && (await ids())[0] === "API-1");
      // An old successful read must not replace newer cards or clear a newer error.
      hold = ({ sort }) => sort === "recent";
      await page.click(RECENT);
      await until(() => pending.length > 0);
      fail = true;
      await page.click(PRIORITY);
      await page.waitForSelector("#boardbanner:not([hidden])");
      fail = false; hold = null;
      pending.splice(0).forEach((done) => done());
      await page.waitForTimeout(100);
      check("AC-105 late success keeps the newer error and cards", await page.isVisible("#boardbanner") && (await ids())[0] === "API-1");
      await page.click(RECENT);
      await first("API-205");
      // Full or denied storage must not make a sort control inert.
      await page.evaluate(() => { Storage.prototype.setItem = () => { throw new DOMException("Full", "QuotaExceededError"); }; });
      await page.click(PRIORITY);
      check("AC-105 blocked storage still permits sorting in this page", await first("API-1") && await page.getAttribute(PRIORITY, "aria-pressed") === "true");
      for (const theme of ["dark", "light"]) {
        light = theme === "light";
        await go();
        await page.mouse.move(1, 1);
        if (process.env.TM_TEST_SHOTS) {
          fs.mkdirSync(process.env.TM_TEST_SHOTS, { recursive: true });
          await page.screenshot({ path: path.join(process.env.TM_TEST_SHOTS, `sort-${name}-${device === PHONE ? "phone" : "wide"}-${theme}.png`) });
        }
        check(`AC-105 ${theme} header uses shared icon colors and pressed surface`, await page.$eval(`${COL} .colsort`, (g) => {
          const a = getComputedStyle(g.querySelector('[aria-pressed="true"]')), b = getComputedStyle(g.querySelector('[aria-pressed="false"]'));
          return a.backgroundColor !== b.backgroundColor && a.color !== b.color;
        }));
      }
      check("no page errors", f.errors.length === 0, f.errors.join(" | "));
    } finally {
      pending.splice(0).forEach((done) => done());
      await f.close();
    }
  }
}
