// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Closing a task cancels pending opens, but a later deliberate open stands (AC-108, AC-133).
import { check, runningIn, until } from "./check.mjs";
import { drawn, openPage, SIZES } from "./fake.mjs";

const META = { projects: [{ id: "default", name: "Default" }], users: [{ id: "alex", name: "Alex" }], sessions: [], agents: [] };
const TASK = { id: "API-12", uid: "abfed14f", title: "Close a task window", status: "TODO", project: "default",
  severity: "P2", owner: "alex", owner_name: "Alex", text: "A task to close.", text_rev: 1,
  files: [], children: [], history: [], created_at: 1, updated_at: 1 };
const opened = (page) => page.$eval("#taskbox", (b) => b.open);
const footer = (page) => page.getByRole("button", { name: "Close", exact: true }).last();

export async function webTaskClose(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device }), { page } = f;
    let hold = false, refused = false, pending = [];
    try {
      await page.route("**/api/tasks/API-12", async (r) => {
        const fail = refused;
        if (hold) await new Promise((done) => pending.push(done));
        await r.fulfill({ status: fail ? 503 : 200, json: fail ? { error: "Task read refused." } : TASK });
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
      for (const how of ["Close", "×", "Esc", "Back"]) {
        await open();
        await waitRead();
        await page.evaluate(() => { window.closed = new Promise((done) => document.getElementById("taskbox").addEventListener("close", done, { once: true })); });
        if (how === "Close") await footer(page).click();
        else if (how === "×") await page.click("#taskclose");
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
      await page.click('.tasktext button:text-is("Edit")');
      await page.fill(".tasktext textarea", "An unsaved edit");
      await footer(page).click();
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
      check("no page errors", !f.errors.length, f.errors.join(" | "));
    } finally {
      pending.splice(0).forEach((done) => done());
      await f.close();
    }
  }
}
