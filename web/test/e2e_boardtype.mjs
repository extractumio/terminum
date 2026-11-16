// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The board's words and its cog against a fake terminumd (terminum-kanban.md AC-105 "one type size", AC-106 "from
// TODO's head", AC-139 "changing it"): every field, choice, label, key, tab, switch, fold and hint of the head, the
// columns' heads, the new task's and the project's windows as large as a task's text; the cog opening the one shown
// project's window, saying what to do with several; no + Task in the head beside TODO's.
import { check, runningIn, until } from "./check.mjs";
import { drawn, fakeBoard, openPage, ORIGIN, SIZES, THEME } from "./fake.mjs";

const META = { projects: [{ id: "api", name: "API", color: 0 }, { id: "web", name: "Web", color: 1 }], users: [{ id: "human", name: "Alex" }], sessions: [], hosts: [], theme: THEME,
  agents: [{ id: "a1", project: "api", name: "Claude Code" }] };
const tasks = [{ id: "API-1", title: "A task", project: "api", status: "TODO", severity: "P2", owner: "human", owner_name: "Alex", updated_at: 1, parent: null }];
const counts = { TODO: 1, "IN PROGRESS": 0, "WAIT FOR HUMAN": 0, FAILED: 0, DONE: 0 };

/** The sizes of what shows words in `roots`, by size: a title, a count, a badge or a tick keep their own. */
const sizes = (page, roots) => page.evaluate((roots) => {
  const out = {};
  for (const root of document.querySelectorAll(roots)) for (const e of [root, ...root.querySelectorAll("*")]) {
    if (!e.getClientRects().length || e.closest(".count, .tabn, .csev, .cid, kbd, h2, .card, [type=checkbox]")) continue;
    const words = [...e.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim()) || ["INPUT", "SELECT", "TEXTAREA"].includes(e.tagName);
    if (words) (out[getComputedStyle(e).fontSize] ??= []).push(e.id || e.className || e.tagName);
  }
  return out;
}, roots);

export async function webBoardType(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device, secret: "a".repeat(64), stored: "a".repeat(64) });
    const { page } = f;
    try {
      await drawn(page);
      await fakeBoard(page, { meta: () => META, tasks: (status) => (status === "TODO" ? tasks : []), counts });
      const go = async (query) => {
        await page.goto(`${ORIGIN}/kanban/${query}`);
        await page.waitForSelector(".col .card");
        await page.waitForFunction(() => document.body.dataset.socket === "open");
      };
      await go("?project=api");
      check("AC-106 the board's head holds no + Task: TODO's + New task is the one", await page.locator("#bar #newtask").count() === 0 && await page.locator('.col[data-status="TODO"] .colhead #newtask').count() === 1);
      await page.click("#newtask");
      await page.waitForSelector("#newbox[open]");
      await page.click("#newbox details.more summary");
      const text = await page.$eval("#newtext", (e) => getComputedStyle(e).fontSize);
      const one = (got) => Object.keys(got).length === 1 && Object.keys(got)[0] === text;
      const news = await sizes(page, "#newbox");
      check(`AC-105 the new task's labels, fields, choices, keys and folds are the size of its text (${text})`, one(news), JSON.stringify(news));
      await page.keyboard.press("Escape");
      await page.waitForFunction(() => !document.getElementById("newbox").open);
      const board = await sizes(page, "#bar, .colhead, #colswitch");
      check(`AC-105 the head's keys, fields and switch, the columns' names, tabs and keys are the size of a task's text`, one(board), JSON.stringify(board));

      await page.click("#projectcog");
      await page.waitForSelector("#projectbox[open]");
      check("AC-139 the cog opens the shown project's window on General", await page.textContent("#projecthead") === "API" && await page.getAttribute("#projecttabgeneral", "aria-selected") === "true");
      for (const tab of ["general", "auto", "limits"]) {
        await page.click(`#projecttabs [data-tab="${tab}"]`);
        const got = await sizes(page, "#projectbox");
        check(`AC-105 the project's ${tab} tab: tabs, labels, fields and hints the size of a task's text`, one(got), JSON.stringify(got));
      }
      const fits = await page.evaluate(() => { const d = document.getElementById("projectbox"), t = document.getElementById("projecttabs").getBoundingClientRect(); return d.scrollWidth <= d.clientWidth + 1 && t.right <= innerWidth; });
      check("AC-139 its six tabs stay inside the window", fits);
      check("AC-170 the Auto-Dispatch tab has its own Save, General's keys gone", await page.isVisible("#autook") === false && (await page.click('#projecttabs [data-tab="auto"]'), await page.isVisible("#autook")) && !await page.isVisible("#projectok"));
      await page.keyboard.press("Escape");

      await go("?project=api,web");
      await page.click("#projectcog");
      check("AC-139 with several projects shown the cog says to select one, no window", await until(async () => /Multiple projects are selected: select only one to edit its settings\./.test(await page.textContent("#toast")))
        && !await page.evaluate(() => document.getElementById("projectbox").open), await page.textContent("#toast"));
    } finally { await f.close(); }
  }
}
