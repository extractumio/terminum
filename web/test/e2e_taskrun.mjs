// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// RUN remains visible in every column and while WAIT FOR HUMAN holds its session (AC-122, AC-157); in a task's
// window it starts at once with the comment, RUN's window opening only for what is left to choose (AC-121, AC-169).
import fs from "node:fs";
import path from "node:path";
import { check, runningIn, until } from "./check.mjs";
import { drawn, openPage, ORIGIN, PHONE, SIZES, THEME } from "./fake.mjs";
import { pressCard } from "./boardkit.mjs";

const META = { projects: [{ id: "api", name: "API", color: 0 }], users: [], sessions: [{ key: "k1", title: "API conversation" }], hosts: [], agents: [{ id: "a1", project: "api", name: "Claude", kind: "claude", color: 0, line: "claude" }] };
const COLUMNS = ["TODO", "IN PROGRESS", "WAIT FOR HUMAN", "FAILED", "DONE"];
const tasks = () => [...COLUMNS, "WAIT FOR HUMAN", "WAIT FOR HUMAN", "WAIT FOR HUMAN"].map((status, i) => ({
  id: `API-${i + 1}`, uid: "abcd1234", title: `Continue task ${i + 1}`, project: "api", severity: "P1", status, text: "Choose an API.",
  created_at: 1794435000, updated_at: 1794435000, files: [], history: [], children: [], parent: null,
  agent: { name: "Claude", color: 0, conversation: "fake-1" },
  session: i === 6 ? null : { key: "k1", closed: i === 5 }, run_holds: i < 5 ? `API-${i + 1}` : null,
  waits: i === 7 ? "Claude" : null,
}));

export async function webTaskRun(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device, secret: "a".repeat(64), stored: "a".repeat(64) });
    const { page } = f, all = tasks(), sent = [];
    let refused = false, refusedPress = null, releaseUpload = null, holdPreview = false, releasePreview = null, previews = 0;
    let offer = {}, full = null;                   // what terminumd offers for the next read, and answers a press with no room
    const comments = [];
    const json = (r, data, status = 200) => r.fulfill({ status, contentType: "application/json", body: JSON.stringify(data) });
    await drawn(page);
    await page.route(`${ORIGIN}/kanban/**`, (r) => r.fulfill({ contentType: "text/html", body: fs.readFileSync(new URL("../static/kanban.html", import.meta.url), "utf8").replace("__BUILD__", "test") }));
    await page.route("**/api/tasks/meta", (r) => json(r, { ...META, theme: THEME }));
    await page.route("**/api/schedules", (r) => json(r, { schedules: [] }));
    await page.route("**/api/tasks?*", (r) => json(r, {
      tasks: all.filter((t) => t.status === new URL(r.request().url()).searchParams.get("status")),
      counts: Object.fromEntries(COLUMNS.map((s) => [s, all.filter((t) => t.status === s).length])), slots: { agents: {}, full: [], full_projects: [] },
    }));
    await page.route("**/api/tasks/API-*", (r) => json(r, all.find((t) => r.request().url().endsWith(`/${t.id}`))));
    await page.route("**/api/tasks/API-*/run?*", async (r) => {
      const no = refused;
      previews++;
      if (holdPreview) await new Promise((done) => { releasePreview = done; });
      releasePreview = null;
      return no ? json(r, { error: "The agent is still working." }, 400)
        : json(r, { first: false, ids: [new URL(r.request().url()).pathname.split("/")[3]], agents: META.agents, dir: "/workspace", host: null, project: "api", prompt: "The human ran it.", prompt_rev: "r1", ...offer });
    });
    await page.route("**/api/tasks/API-*/run", (r) => {
      sent.push(r.request().postDataJSON());
      return refusedPress ? json(r, { error: refusedPress }, 400) : json(r, full ? { full } : { run: "r1" });
    });
    await page.route("**/api/tasks/API-*/comments", (r) => { comments.push(r.request().postDataJSON()); return json(r, {}); });
    await page.route("**/api/task-files?*", async (r) => {
      await new Promise((done) => { releaseUpload = done; });
      releaseUpload = null;
      return json(r, { id: "queued-upload", name: "queue-note.txt", mime: "text/plain", bytes: 4 });
    });
    const run = () => page.locator("#taskbox").getByRole("button", { name: "Run", exact: true });
    const open = async (id) => { await pressCard(page, id); await page.waitForSelector("#taskbox[open]"); };
    const close = () => page.click("#taskclose");
    const dropFile = () => page.evaluate(() => {
      const files = new DataTransfer();
      files.items.add(new File(["note"], "queue-note.txt", { type: "text/plain" }));
      document.querySelector(".newcomment textarea").dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: files }));
    });
    try {
      await page.goto(`${ORIGIN}/kanban/?project=api`);
      await page.waitForSelector(".card");
      check("AC-122 each column keeps its card's RUN, including an open IN PROGRESS session", await page.locator(".card .crun").count() === all.length);
      check("AC-122 a queued card's RUN is visible and disabled with why", await page.isDisabled('.card[data-id="API-8"] .crun') && /Stop waiting/.test(await page.getAttribute('.card[data-id="API-8"] .crun', "title")));
      for (const t of all) {
        await open(t.id);
        check(`AC-122 ${t.status} ${t.id} keeps RUN with session ${t.session?.closed ? "closed" : t.session ? "open" : "absent"}`, await run().count() === 1 && await run().isDisabled() === !!t.waits);
        if (t.waits) check("AC-122 a queued task keeps Stop waiting and explains disabled RUN", await page.getByRole("button", { name: "Stop waiting", exact: true }).isVisible() && /Already queued for Claude/.test(await run().getAttribute("title")));
        await close();
      }
      await open("API-8");
      await dropFile();
      if (!await until(() => releaseUpload !== null)) throw new Error("the comment upload did not start");
      const footerSave = page.locator("#taskacts").getByRole("button", { name: "Save", exact: true });
      check("AC-122 uploading keeps RUN visible and both footer actions disabled", await run().isVisible() && await run().isDisabled() && await footerSave.isDisabled());
      releaseUpload();
      await page.waitForFunction(() => document.querySelector(".newcomment textarea").value.includes("queued-upload"));
      check("AC-122 upload completion keeps queued RUN disabled and enables SAVE", await run().isDisabled() && await footerSave.isEnabled() && /Already queued/.test(await run().getAttribute("title")));
      all[7].waits = null;
      f.say({ t: "taskchange", id: "API-8" });
      check("AC-122 ending the queue wait enables RUN and retains its uploaded draft", await until(async () => await run().isEnabled() && !/queued/.test(await run().getAttribute("title"))) && /queued-upload/.test(await page.inputValue(".newcomment textarea")));
      await close();
      await open("API-3");
      await run().focus();
      all[2].title = "Focused waiting task";
      f.say({ t: "taskchange", id: "API-3" });
      await page.waitForFunction(() => document.getElementById("tasktitle").textContent.includes("Focused waiting task"));
      check("AC-122 RUN keeps keyboard focus through an ordinary task refresh", await run().evaluate((key) => document.activeElement === key));
      const pressed = await run().boundingBox();
      await page.mouse.move(pressed.x + pressed.width / 2, pressed.y + pressed.height / 2);
      await page.mouse.down();
      all[2].title = "Updated waiting task";
      f.say({ t: "taskchange", id: "API-3" });
      await page.waitForFunction(() => document.getElementById("tasktitle").textContent.includes("Updated waiting task"));
      await page.mouse.up();
      const both = () => Promise.all([page.isVisible("#runbox"), page.isVisible("#taskbox")]).then((shown) => shown.join());
      check("AC-122 a task refresh during a RUN press preserves its click: the run starts at once, RUN's window unopened, the task's closed",
        await until(async () => sent.length === 1 && await both() === "false,false") && sent[0].comment === "" && !sent[0].fresh, JSON.stringify([sent, await both()]));
      await open("API-3");
      await page.fill(".newcomment textarea", "Kept only");
      await footerSave.click();
      check("AC-108 Save keeps the comment and nothing runs: the window stays, its field empty, Save off",
        await until(async () => comments.length === 1 && await page.inputValue(".newcomment textarea") === "" && await footerSave.isDisabled()) && comments[0].text === "Kept only" && sent.length === 1
        && await both() === "false,true", JSON.stringify([comments, sent.length, await both()]));
      await page.fill(".newcomment textarea", "Use v2");
      refused = true;
      holdPreview = true;
      await run().focus();
      await page.keyboard.press("Enter");
      if (!await until(() => releasePreview !== null)) throw new Error("the Run read did not start");
      check("AC-122 preparing RUN freezes its draft and footer actions", await run().isDisabled() && await footerSave.isDisabled()
        && await page.$eval(".newcomment textarea", (area) => area.readOnly));
      await dropFile();
      check("AC-122 a file added during RUN preparation is refused without changing the draft", releaseUpload === null
        && /Wait until this action finishes/.test(await page.textContent("#toast")) && await page.inputValue(".newcomment textarea") === "Use v2");
      all[2].waits = "Claude";
      all[2].title = "Queued during Run preparation";
      f.say({ t: "taskchange", id: "API-3" });
      await page.waitForFunction(() => document.querySelector('.card[data-id="API-3"] .ctitle').textContent.includes("Queued during Run preparation"));
      check("AC-122 a refresh cannot unlock RUN preparation", await run().isDisabled() && await footerSave.isDisabled());
      holdPreview = false;
      releasePreview();
      check("AC-122 a busy refusal keeps the answer draft in the task window", await until(async () => /still working/.test(await page.textContent("#toast"))) && await page.inputValue(".newcomment textarea") === "Use v2" && sent.length === 1);
      check("AC-122 a finished action reapplies the task's current queue lock", await until(async () => await run().isDisabled() && await footerSave.isEnabled()) && /Already queued/.test(await run().getAttribute("title")));
      all[2].waits = null;
      f.say({ t: "taskchange", id: "API-3" });
      if (!await until(() => run().isEnabled())) throw new Error("RUN stayed disabled after the queue wait ended");
      const beforeRetry = previews;
      await run().focus();
      await page.keyboard.press("Enter");
      check("AC-122 a refused read returns keyboard focus to enabled RUN", await until(async () => previews > beforeRetry && await run().isEnabled())
        && await run().evaluate((key) => document.activeElement === key));
      refused = false;
      refusedPress = "The session is not at an empty input box: answer there first";
      await run().click();
      check("AC-122 a blocked input is said in the task's window, which keeps its comment; RUN's window stays shut",
        await until(async () => /empty input box/.test(await page.textContent("#toast"))) && await page.inputValue(".newcomment textarea") === "Use v2" && await both() === "false,true" && sent.length === 2);
      refusedPress = null;
      await run().click();
      check("AC-122 retry sends the answer and closes the task's window after a start", await until(async () => await both() === "false,false") && sent.length === 3 && sent.slice(1).every((b) => b.comment === "Use v2" && !b.fresh));
      // a first run: at once on the agent known, RUN's window for an agent's words, several tasks, no room, a refusal
      const first = async (what, press = null) => { offer = { first: true, last: "a1", ...what }; full = press; await open("API-7"); await page.fill(".newcomment textarea", "Mind the cache"); await run().click(); };
      let n = sent.length;
      await first({});
      check("AC-121 a first run of a task a human wrote starts at once on the agent known",
        await until(async () => sent.length === n + 1 && await both() === "false,false") && sent.at(-1).agent === "a1" && sent.at(-1).comment === "Mind the cache" && sent.at(-1).prompt_rev === "r1", JSON.stringify(sent.at(-1)));
      for (const [why, what] of [["a task an agent wrote in", { by_agent: true }], ["a task that brings its subtasks", { ids: ["API-7", "API-9"] }], ["several agents and none known", { last: null, agents: [...META.agents, { id: "a2", name: "Codex", kind: "codex", color: 1, line: "codex" }] }]]) {
        n = sent.length;
        await first(what);
        const shown = await until(() => page.isVisible("#runbox[open]"));
        check(`AC-121 RUN's window opens before a first run of ${why}: nothing is sent unread`, shown && sent.length === n && await page.inputValue("#runmessage") === "Mind the cache"
          && (!what.by_agent || (await page.isVisible("#runmarked") && await page.$eval("#runwhat", (d) => d.open))), JSON.stringify(sent.slice(n)));
        if (shown) await page.click('#runbox button[value="cancel"]');
        await close();
      }
      const room = { words: "Claude runs API-1.", close: null, queue: "Add it to Claude's queue?" };
      n = sent.length;
      await first({ agents: [{ ...META.agents[0], full: room }] });
      check("AC-169 an agent at its limit: RUN's window offers its queue, nothing waits unasked", await until(() => page.isVisible("#runbox[open]")) && await page.textContent("#rungo") === "Add to queue" && sent.length === n);
      await page.click('#runbox button[value="cancel"]');
      await close();
      n = sent.length;
      await first({}, room);
      check("AC-169 a press that finds no room after all opens RUN's window with the queue's offer", await until(() => page.isVisible("#runbox[open]")) && await page.textContent("#rungo") === "Add to queue" && sent.length === n + 1 && !sent.at(-1).queue);
      full = null;
      await page.click("#rungo");
      check("AC-169 and Add to queue is the human's yes", await until(async () => await both() === "false,false") && sent.at(-1).queue === true);
      n = sent.length;
      refusedPress = "The task's text changed: look at it again.";
      await first({});
      check("AC-121 a first run refused is said in RUN's window, with what would be sent", await until(() => page.isVisible("#runbox[open]")) && /text changed/.test(await page.textContent("#runerror")) && sent.length === n + 1);
      refusedPress = null;
      await page.click('#runbox button[value="cancel"]');
      await close();
      offer = {};
      // Run from scratch: apart from Run, on the Status line; RUN's window is where its agent is chosen
      await open("API-6");
      await page.fill(".newcomment textarea", "Start over");
      const scratch = page.locator("#taskbody .taskown .scratch");
      check("AC-138 Run from scratch stands on the task's Status line, not among the window's keys, and not in RUN's window",
        await scratch.isVisible() && await page.locator("#taskacts .scratch").count() === 0 && await page.locator("#runbox").getByText("Run from scratch", { exact: true }).count() === 0);
      n = sent.length;
      await scratch.click();
      check("AC-138 it opens RUN's window to choose the agent, the comment as its message", await until(() => page.isVisible("#runbox[open]")) && await page.isVisible("#runagentrow") && await page.inputValue("#runmessage") === "Start over" && sent.length === n);
      await page.click("#rungo");
      check("AC-138 and Run there starts a new conversation", await until(async () => await both() === "false,false") && sent.length === n + 1 && sent.at(-1).fresh === true && sent.at(-1).agent === "a1");
      await open("API-1");
      check("AC-138 a task its run holds offers no Run from scratch", await page.locator("#taskbody .scratch").count() === 0);
      await close();
      const canceledAt = sent.length;
      await open("API-3");
      holdPreview = true;
      await run().click();
      if (!await until(() => releasePreview !== null)) throw new Error("the cancelable Run read did not start");
      await page.goBack();
      await page.waitForSelector("#taskbox", { state: "hidden" });
      await open("API-4");
      await page.fill(".newcomment textarea", "Another task's draft");
      holdPreview = false;
      releasePreview();
      check("AC-122 a canceled read cannot run, open RUN's window or close another task", await until(async () => await footerSave.isEnabled())
        && !(await page.isVisible("#runbox")) && /API-4/.test(await page.textContent("#tasktitle"))
        && await page.inputValue(".newcomment textarea") === "Another task's draft" && sent.length === canceledAt);
      await close();
      for (const theme of ["dark", "light"]) {
        await page.evaluate(async (t) => (await import("/contrast.js")).pageColors(t), theme === "light" ? { ...THEME, bg: "#ffffff", fg: "#1f2328", cursor: "#222222", dark: false } : THEME);
        await open("API-3");
        const keys = await run().evaluate((b) => {
          const r = b.getBoundingClientRect(), next = b.previousElementSibling.getBoundingClientRect();
          return { width: r.width, height: r.height, aligned: Math.abs(r.top - next.top) < 1, fits: r.right <= innerWidth, primary: b.classList.contains("primary") };
        });
        check(`AC-122 ${theme} waiting RUN is primary, aligned with SAVE and fits`, keys.primary && keys.aligned && keys.fits, JSON.stringify(keys));
        if (device === PHONE) check("AC-122 phone RUN has a touch target", keys.width >= 44 && keys.height >= 40, JSON.stringify(keys));
        await run().focus();
        check("AC-122 RUN is keyboard reachable", await run().evaluate((b) => b === document.activeElement));
        if (process.env.TM_TEST_SHOTS) {
          fs.mkdirSync(process.env.TM_TEST_SHOTS, { recursive: true });
          await page.screenshot({ path: path.join(process.env.TM_TEST_SHOTS, `waiting-run-${name}-${size}-${theme}.png`) });
        }
        await page.keyboard.press("Escape");
        check("AC-122 Esc closes the task window", !(await page.isVisible("#taskbox")));
      }
      check("no page errors", f.errors.length === 0, f.errors.join(" | "));
    } finally { releaseUpload?.(); releasePreview?.(); await f.close(); }
  }
}
