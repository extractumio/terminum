// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A draft's file that terminumd let go (kept unsaved over 7 days) leaves the draft with its link, and the window
// names it; a file it cannot tell about stays (AC-106, AC-108).
import { check, runningIn, until } from "./check.mjs";
import { drawn, openPage, SIZES } from "./fake.mjs";

const META = { projects: [{ id: "default", name: "Default" }], users: [{ id: "human", name: "Alex" }], sessions: [], agents: [] };
const TASK = { id: "API-12", uid: "abfed14f", title: "A task with a draft", status: "TODO", project: "default",
  severity: "P2", owner: "human", owner_name: "Alex", text: "Its text.", text_rev: 1,
  files: [], children: [], history: [], created_at: 1, updated_at: 1 };
const id = (c) => c.repeat(32);
const [LOG, SHOT, SLOW, ODD, LATE] = ["a", "b", "c", "d", "e"].map(id);
const link = (name, f, picture = false) => `${picture ? "!" : ""}[${name}](/api/task-files/${f})`;
const GONE_LOG = "log.txt: gone, this draft was not saved for 7 days. Drop it again to keep it.";
const REFUSED = { error: "A file of this window is gone: add it again" };

export async function webDraftFiles(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device }), { page } = f;
    // what terminumd answers of each file: a status, or null for no answer at all
    const files = { [LOG]: 404, [SHOT]: 200, [SLOW]: 500, [ODD]: null, [LATE]: 200 };
    const posts = [], made = [];
    let refuse = false;
    try {
      await page.route("**/api/task-files/*", (r) => {
        const status = files[r.request().url().split("/").pop()];
        return status === null ? r.abort() : r.fulfill({ status, body: "" });
      });
      await page.route("**/api/tasks/API-12", (r) => r.fulfill({ json: TASK }));
      await page.route("**/api/tasks/API-12/comments", (r) => {
        posts.push(r.request().postDataJSON());
        return r.fulfill({ status: refuse ? 400 : 200, json: refuse ? REFUSED : {} });
      });
      await page.route("**/api/tasks", (r) => {
        made.push(r.request().postDataJSON());
        return r.fulfill({ status: refuse ? 400 : 200, json: refuse ? REFUSED : { ...TASK, id: "API-13" } });
      });
      await drawn(page);
      await page.evaluate(async (meta) => {
        const html = await (await fetch("/kanban.html")).text();
        document.body.append(new DOMParser().parseFromString(html, "text/html").querySelector("#newbox"));
        const { taskWindow } = await import("/kanbanwin.js");
        const { taskForm } = await import("/kanbanform.js");
        window.taskwin = taskWindow({ getMeta: () => meta, runs: { open: async () => false }, openTask() {}, newSubtask() {} });
        window.newform = taskForm({ getMeta: () => meta, getFilter: () => ({ project: ["default"] }), onMade() {}, onRun() {}, slots: () => ({ full: [], full_projects: [] }) });
      }, META);
      const stored = (key) => page.evaluate((k) => JSON.parse(localStorage.getItem(k)), key);
      const seed = (text, ids) => page.evaluate(([t, i]) => {
        localStorage.setItem("tm.comment.API-12", JSON.stringify(t));
        localStorage.setItem("tm.commentfiles.API-12", JSON.stringify(i));
      }, [text, ids]);
      const open = async () => { await page.evaluate(() => window.taskwin.open("API-12")); await page.waitForSelector("#taskbox[open]"); };
      const close = () => page.evaluate(() => window.taskwin.tryClose());
      const draft = () => page.inputValue(".newcomment textarea");
      const toast = async () => (await page.textContent("#toast").catch(() => "")) ?? "";
      const save = () => page.locator("#taskacts").getByRole("button", { name: "Save", exact: true }).click();

      await seed(`See:\n${link("log.txt", LOG)}\n${link("shot.png", SHOT, true)}\n`, [LOG, SHOT]);
      await open();
      check("AC-108 a comment draft's file gone leaves it, its link too, the window naming it",
        await until(async () => !(await draft()).includes(LOG)) && (await toast()).includes(GONE_LOG)
        && await draft() === `See:\n${link("shot.png", SHOT, true)}\n` && JSON.stringify(await stored("tm.commentfiles.API-12")) === JSON.stringify([SHOT]),
        JSON.stringify([await draft(), await toast()]));
      check("AC-108 its draft is kept without it", (await stored("tm.comment.API-12")) === await draft());
      await save();
      check("AC-108 SAVE keeps the comment with the files still there", await until(async () => posts.length === 1 && await draft() === "")
        && JSON.stringify(posts[0].files) === JSON.stringify([SHOT]) && !posts[0].text.includes(LOG));
      await close();

      const kept = `${link("slow.txt", SLOW)}\n${link("odd.txt", ODD)}\n`;
      await seed(kept, [SLOW, ODD]);
      await page.evaluate(() => { document.getElementById("toast")?.replaceChildren(); });
      await open();
      await page.waitForTimeout(400);
      check("AC-108 a file terminumd cannot tell about stays in the draft, nothing said",
        await draft() === kept && JSON.stringify(await stored("tm.commentfiles.API-12")) === JSON.stringify([SLOW, ODD]) && !(await toast()).includes("gone"),
        JSON.stringify([await draft(), await stored("tm.commentfiles.API-12"), await toast()]));
      await close();

      await seed(`${link("late.txt", LATE)}\nWith a note`, [LATE]);
      await open();
      await page.waitForTimeout(300);
      files[LATE] = 404;
      refuse = true;
      await save();
      check("AC-108 a SAVE refused as a file went meanwhile names it and drops it",
        await until(async () => (await toast()).includes("late.txt: gone")) && await draft() === "With a note"
        && JSON.stringify(await stored("tm.commentfiles.API-12")) === "[]" && posts.length === 2);
      refuse = false;
      await save();
      check("AC-108 the next SAVE keeps the rest", await until(async () => posts.length === 3 && await draft() === "") && posts[2].text === "With a note" && posts[2].files.length === 0);
      await close();

      files[LATE] = 200;
      await page.evaluate(([log, shot, late]) => localStorage.setItem("tm.taskdraft", JSON.stringify({ title: "With files", project: "default",
        text: `[log.txt](/api/task-files/${log})\n![shot.png](/api/task-files/${shot})\n[late.txt](/api/task-files/${late})\n`,
        files: [{ id: log, name: "log.txt", mime: "text/plain" }, { id: shot, name: "shot.png", mime: "image/png" }, { id: late, name: "late.txt", mime: "text/plain" }] })), [LOG, SHOT, LATE]);
      await page.evaluate(() => window.newform.open());
      const listed = () => page.$$eval("#newfiles li", (all) => all.map((li) => li.firstChild.textContent).join());
      check("AC-106 a new task's draft file gone leaves its Files and text, the window naming it",
        await until(async () => await listed() === "shot.png,late.txt") && await page.textContent("#newerror") === GONE_LOG
        && !(await page.inputValue("#newtext")).includes(LOG) && (await stored("tm.taskdraft")).files.length === 2, await listed());
      files[LATE] = 404;
      refuse = true;
      await page.click("#newcreate");
      check("AC-106 a Create refused as a file went meanwhile names it and drops it",
        await until(async () => (await page.textContent("#newerror")).startsWith("late.txt: gone")) && await listed() === "shot.png" && made.length === 1);
      refuse = false;
      await page.click("#newcreate");
      check("AC-106 the next Create keeps the rest", await until(() => made.length === 2) && JSON.stringify(made[1].files) === JSON.stringify([SHOT]));
      check("no page errors", !f.errors.length, f.errors.join(" | "));
    } finally {
      await f.close();
    }
  }
}
