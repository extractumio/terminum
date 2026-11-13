// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Closing a task cancels pending opens, but a later deliberate open stands; Save keeps the comment being written
// once, whatever is drawn anew under its press, and leaves the window open (AC-108, AC-133).
import { check, runningIn, until } from "./check.mjs";
import { drawn, openPage, SIZES } from "./fake.mjs";

const META = { projects: [{ id: "default", name: "Default" }], users: [{ id: "alex", name: "Alex" }], sessions: [], agents: [] };
const TASK = { id: "API-12", uid: "abfed14f", title: "Close a task window", status: "TODO", project: "default",
  severity: "P2", owner: "alex", owner_name: "Alex", text: "A task to close.", text_rev: 1,
  files: [], children: [], history: [], created_at: 1, updated_at: 1 };
const opened = (page) => page.$eval("#taskbox", (b) => b.open);
const footer = (page) => page.locator("#taskacts").getByRole("button", { name: "Save", exact: true });
const draft = (page) => page.inputValue(".newcomment textarea");

export async function webTaskClose(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device }), { page } = f;
    let hold = false, refused = false, pending = [], task = structuredClone(TASK), commentRefused = false;
    const posts = [];
    try {
      await page.route("**/api/tasks/API-12", async (r) => {
        const fail = refused;
        if (hold) await new Promise((done) => pending.push(done));
        await r.fulfill({ status: fail ? 503 : 200, json: fail ? { error: "Task read refused." } : task });
      });
      await page.route("**/api/tasks/API-13", (r) => r.fulfill({ json: { ...TASK, id: "API-13", title: "Another task" } }));
      await page.route("**/api/tasks/*/comments", (r) => {
        posts.push({ task: new URL(r.request().url()).pathname.split("/")[3], ...r.request().postDataJSON() });
        return r.fulfill({ status: commentRefused ? 503 : 200, json: commentRefused ? { error: "Comment refused." } : {} });
      });
      await drawn(page);
      await page.evaluate(async (meta) => {
        const { taskWindow } = await import("/kanbanwin.js");
        const { taskAddress } = await import("/kanbanroute.js");
        window.taskwin = taskWindow({ getMeta: () => meta, runs: { open: async () => false }, openTask() {}, newSubtask() {} });
        const open = window.taskwin.open;
        window.taskwin.open = (...args) => window.reading = open(...args);
        window.address = taskAddress({ windowed: window.taskwin, needs: { opened() {} } });
        history.replaceState(null, "", "/kanban/");
      }, META);
      const open = async () => {
        await page.evaluate(() => window.address.openTask("API-12"));
        await page.waitForSelector("#taskbox[open]");
      };
      const waitRead = async () => {
        hold = true;
        await page.evaluate(() => window.address.openTask("API-12"));
        if (!await until(() => pending.length > 0)) throw new Error("the pending task read never started");
      };
      const release = async () => {
        hold = false;
        pending.splice(0).forEach((done) => done());
        await page.evaluate(() => window.reading);
      };
      const backOnBoard = () => page.waitForFunction(() => location.pathname === "/kanban/");
      const closeAgain = () => page.evaluate(() => new Promise((done) => {
        document.getElementById("taskbox").addEventListener("close", done, { once: true });
        window.taskwin.tryClose();
      }));
      for (const answer of ["success", "failure"]) {
        refused = answer === "failure";
        await waitRead();
        await page.evaluate(() => { window.stale = window.reading.catch(() => {}); });
        await page.goBack();
        await backOnBoard();
        refused = false;
        await page.evaluate(() => window.address.openTask("API-12"));
        if (!await until(() => pending.length === 2)) throw new Error("the replacement task read never started");
        pending.shift()();
        await page.evaluate(() => window.stale);
        check(`AC-133 a canceled ${answer} cannot change a newer opening's address or show an error`,
          new URL(page.url()).pathname === "/kanban/API-12" && !(await page.textContent("#toast")).includes("Task read refused."));
        await release();
        check(`AC-133 the intended window opens after a canceled ${answer}`, await opened(page) && new URL(page.url()).pathname === "/kanban/API-12");
        await closeAgain();
        await backOnBoard();
      }
      for (const how of ["×", "Esc", "Back"]) {
        await open();
        await waitRead();
        await page.evaluate(() => { window.closed = new Promise((done) => document.getElementById("taskbox").addEventListener("close", done, { once: true })); });
        if (how === "×") await page.click("#taskclose");
        else if (how === "Esc") await page.keyboard.press("Escape");
        else await page.goBack();
        await page.evaluate(() => window.closed);
        await backOnBoard();
        const closed = !await opened(page);
        await release();
        check(`AC-133 ${how} cancels a pending open: its late answer cannot reopen the window`, closed && !await opened(page));
        // Leave the next case independent even when the regression is present.
        if (await opened(page)) await closeAgain();
      }
      // Back can be pressed before even the first task read has answered.
      hold = true;
      await page.evaluate(() => window.address.openTask("API-12"));
      if (!await until(() => pending.length > 0)) throw new Error("the initial task read never started");
      await page.goBack();
      await backOnBoard();
      const answered = page.waitForResponse("**/api/tasks/API-12");
      hold = false;
      pending.splice(0).forEach((done) => done());
      await answered;
      await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
      check("AC-133 Back before the first task read answers keeps the window closed", !await opened(page));
      if (await opened(page)) await closeAgain();

      await open();
      // A new press while the close's history.back is still pending (e2e_liveboard.mjs covers it too).
      await page.evaluate(() => new Promise((done) => {
        document.getElementById("taskbox").addEventListener("close", () => queueMicrotask(() => { window.address.openTask("API-12"); done(); }), { once: true });
        window.taskwin.tryClose();
      }));
      check("AC-133 a deliberate open after Close survives the close's own Back", await until(async () => await opened(page) && new URL(page.url()).pathname === "/kanban/API-12"));
      await page.click('.tasktext button[aria-label="Edit"]');
      await page.fill(".tasktext textarea", "An unsaved edit");
      await page.keyboard.press("Escape");
      await page.waitForSelector("#confirmbox[open]");
      await page.keyboard.press("Escape");
      check("AC-133 Cancel on discard keeps the window and its edit", await opened(page) && await page.inputValue(".tasktext textarea") === "An unsaved edit");
      await waitRead();
      await page.click("#taskclose");
      await page.waitForSelector("#confirmbox[open]");
      await page.click("#confirmok");
      await backOnBoard();
      await release();
      check("AC-133 confirming discard cancels a pending open too", !await opened(page));
      refused = true;
      await page.evaluate(() => window.address.openTask("API-12"));
      check("AC-133 a current read's failure still says why and returns to the board", await until(async () =>
        new URL(page.url()).pathname === "/kanban/" && (await page.textContent("#toast")).includes("Task read refused.")) && !await opened(page));
      await page.route("**/api/tasks/abfed14f", (r) => r.fulfill({ json: TASK }));
      await page.evaluate(() => window.address.openTask("abfed14f"));
      check("AC-148 an applied alias read still opens the task at its current ID", await until(async () =>
        await opened(page) && new URL(page.url()).pathname === "/kanban/API-12"));
      refused = false;
      // A real press has a down and an up. A refresh must not take its target away between them.
      const pressSave = async (words) => {
        if (words) await page.fill(".newcomment textarea", words);
        const key = footer(page), r = await key.boundingBox();
        await key.evaluate((b) => { window.pressedSave = b; });
        await page.mouse.move(r.x + r.width / 2, r.y + r.height / 2);
        await page.mouse.down();
      };
      const changed = async () => { task.updated_at++; await page.evaluate(() => window.taskwin.refresh(false, true)); };
      /** Whether exactly one comment went since `from`, saying `words`, the window open and its field empty. */
      const savedOnce = async (from, words) => await until(async () => posts.length > from && await draft(page) === "", 1500)
        && posts.length === from + 1 && posts.at(-1).text === words && posts.at(-1).task === "API-12" && await opened(page);
      check("AC-108 Save is off with nothing written", await (async () => { await open(); return footer(page).isDisabled(); })());
      for (const change of ["history", "available actions"]) {
        const from = posts.length;
        await pressSave(`Draft under ${change}`);
        if (change === "available actions") task.run_holds = TASK.id;
        await changed();
        const kept = await page.evaluate(() => window.pressedSave.isConnected);
        await page.mouse.up();
        check(`AC-133 Save survives a ${change} refresh during its press: the comment kept once, the window open`, kept && await savedOnce(from, `Draft under ${change}`), JSON.stringify(posts.slice(from)));
        task.run_holds = null;
        await changed();
      }
      let from = posts.length;
      await pressSave("Not sent");
      task.run_holds = TASK.id;
      await changed();
      await page.mouse.move(2, 2);
      await page.mouse.up();
      check("AC-133 releasing outside Save cancels the click and applies the pending footer",
        await until(async () => await page.locator('#taskacts button:text-is("Give up")').count() === 1, 1000)
        && await page.locator('#taskacts button:text-is("Run")').count() === 1 && await opened(page) && posts.length === from && await draft(page) === "Not sent");
      const keys = await page.$$eval("#taskacts button", (all) => all.map((b) => ({ word: b.textContent, left: Math.round(b.getBoundingClientRect().left), top: Math.round(b.getBoundingClientRect().top), color: getComputedStyle(b).color })));
      const giveUp = keys.find((k) => k.word === "Give up"), save = keys.find((k) => k.word === "Save");
      check("AC-173 a task's keys: Give up at the left in the danger color, Save and Run at the right, on one line",
        keys.map((k) => k.word).join() === "Give up,Save,Run" && save.left - giveUp.left > 100 && giveUp.color !== save.color && new Set(keys.map((k) => k.top)).size === 1, JSON.stringify(keys));
      await footer(page).focus();
      await page.keyboard.down("Space");
      task.run_holds = null;
      await changed();
      await page.keyboard.up("Space");
      check("AC-133 Space keeps its Save target across a refresh", await savedOnce(from, "Not sent"));
      from = posts.length;
      await page.fill(".newcomment textarea", "By its keys");
      await page.keyboard.press("ControlOrMeta+Enter");
      check("AC-133 ⌘Enter in the comment is Save", await savedOnce(from, "By its keys"));
      from = posts.length;
      await pressSave("An old draft");
      await closeAgain();
      await backOnBoard();
      await page.evaluate(() => localStorage.setItem("tm.comment.API-12", JSON.stringify("A second draft")));
      await open();
      await page.mouse.up();
      check("AC-108 reopening the same task cannot turn an old press into saving its new draft", posts.length === from && await opened(page) && await draft(page) === "A second draft");
      await closeAgain();
      await backOnBoard();
      check("AC-108 a window closed with a comment not saved keeps it as its draft", await page.evaluate(() => JSON.parse(localStorage.getItem("tm.comment.API-12"))) === "A second draft" && posts.length === from);
      await page.evaluate(() => localStorage.removeItem("tm.comment.API-12"));
      await open();
      await page.evaluate(() => localStorage.setItem("tm.comment.API-13", JSON.stringify("Draft B")));
      await pressSave("Draft A");
      await page.evaluate(() => window.taskwin.open("API-13"));
      await page.mouse.up();
      check("AC-108 a press on A cannot post B's draft after B opens", posts.length === from && await opened(page) && await draft(page) === "Draft B");
      await closeAgain();
      await backOnBoard();
      await page.evaluate(() => { localStorage.removeItem("tm.comment.API-12"); localStorage.removeItem("tm.comment.API-13"); });
      await open();
      commentRefused = true;
      await pressSave("First draft");
      await changed();
      await page.mouse.up();
      const tried = await until(() => posts.length > from, 1000);
      check("AC-108 a refused Save after refresh keeps its current draft and window", tried && posts.length === from + 1 && posts.at(-1).text === "First draft" && await opened(page) && await draft(page) === "First draft"
        && (await page.textContent("#toast")).includes("Comment refused."));
      if (tried) {
        await page.fill(".newcomment textarea", "Retry draft");
        await changed();
        commentRefused = false;
        await footer(page).click();
        check("AC-108 retry after refresh posts the current draft exactly once", await savedOnce(from + 1, "Retry draft"));
      }
      check("no page errors", !f.errors.length, f.errors.join(" | "));
    } finally {
      pending.splice(0).forEach((done) => done());
      await f.close();
    }
  }
}
