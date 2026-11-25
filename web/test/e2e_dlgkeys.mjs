// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A window's buttons by the keyboard (terminum.md AC-281) and the cards that close after their last action
// (terminum-kanban.md AC-282), against a fake terminumd: ⌘⇧ and the underlined letter press a button, a disabled one's
// does nothing, a letter typed in a field stays typed, ⌘⇧↩ sends the answer and runs and the window closes, Archive
// closes it (refused, it stays), the help lists the window's keys, + Session's Discard and its question by keys,
// Ctrl+Shift off a Mac; with no window open the terminal keeps its keys.
import { check, runningIn, until } from "./check.mjs";
import { drawn, fakeBoard, openPage, ORIGIN, SIZES } from "./fake.mjs";
import { pressCard } from "./boardkit.mjs";

const AGENTS = [{ id: "a1", project: "api", name: "Claude", kind: "claude", color: 0, line: "claude", at_once: 2 }];
const META = { projects: [{ id: "api", name: "API", color: 0, dir: "/w/api" }], users: [], sessions: [], hosts: [], last_agents: { api: "a1" }, agents: AGENTS };
const task = (id, status, extra = {}) => ({ id, uid: "abcd1234", title: `Task ${id}`, project: "api", severity: "P2", status, text: "Choose an API.",
  owner: "human", owner_name: "Alex", created_at: 1794435000, updated_at: 1794435000, files: [], history: [], children: [], parent: null, ...extra });
const json = (r, data, status = 200) => r.fulfill({ status, contentType: "application/json", body: JSON.stringify(data) });

async function board(browser, device, platform = null) {
  const f = await openPage(browser, { device, url: "/kanban/?project=api" });
  const { page } = f;
  if (platform) await page.addInitScript((p) => Object.defineProperty(Navigator.prototype, "platform", { get: () => p }), platform);
  const all = [task("API-1", "WAIT FOR HUMAN", { agent: { name: "Claude", color: 0, conversation: "c-1" }, session: { key: "k1", closed: false }, run_holds: "API-1" }),
    task("API-2", "TODO"), task("API-3", "TODO")];
  const posts = { comments: [], runs: [], archive: [], sessions: [] };
  await fakeBoard(page, { meta: () => META, tasks: (s) => all.filter((t) => t.status === s), counts: { TODO: 2, "WAIT FOR HUMAN": 1 } });
  await page.route("**/api/tasks/API-*", (r) => json(r, all.find((t) => r.request().url().endsWith(`/${t.id}`))));
  await page.route("**/api/tasks/API-*/comments", (r) => { posts.comments.push(r.request().postDataJSON()); return json(r, {}); });
  await page.route("**/api/tasks/API-*/run?*", (r) => json(r, { first: false, ids: [new URL(r.request().url()).pathname.split("/")[3]], agents: AGENTS, dir: "/w/api", host: null, project: "api", prompt: "Run.", prompt_rev: "r1" }));
  await page.route("**/api/tasks/API-*/run", (r) => { posts.runs.push(r.request().postDataJSON()); return json(r, { run: "r1" }); });
  await page.route("**/api/tasks/API-*/archive", (r) => {
    const id = new URL(r.request().url()).pathname.split("/")[3];
    posts.archive.push(id);
    return id === "API-3" ? json(r, { error: "Archive refused." }, 400) : json(r, {});
  });
  await page.goto(`${ORIGIN}/kanban/?project=api`);
  await page.waitForSelector(".card");
  return { ...f, posts };
}

const open = async (page, id) => { await pressCard(page, id); await page.waitForSelector("#taskbox[open]"); };
const isOpen = (page, sel = "#taskbox") => page.$eval(sel, (b) => b.open);
/** The footer's buttons as label: underlined letter (or ↩ hint), as drawn. */
const foot = (page, sel = "#taskbox :is(#taskacts, .barkeys)") => page.$$eval(`${sel} button`, (bs) => Object.fromEntries(bs.filter((b) => b.offsetWidth).map((b) => [b.textContent.trim(),
  b.querySelector("u.dkey")?.textContent ?? b.dataset.keyhint ?? ""])));

export async function webDlgKeys(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const phone = Boolean(device.hasTouch);
    const f = await board(browser, device), { page, posts } = f;
    try {
      await open(page, "API-1");
      const comment = page.locator(".newcomment textarea");
      check("AC-281 a task window's keys: each footer button its letter underlined, Run its ⌘⇧↩, Run still named Run",
        await until(async () => { const k = await foot(page); return k.Archive === undefined && k.Save === "S" && k["Give up"] === "G" && k.Run === "⌘⇧↩"; })
          && await page.locator("#taskbox").getByRole("button", { name: "Run", exact: true }).count() === 1
          && await page.locator("#taskbox").getByRole("button", { name: "Run", exact: true }).getAttribute("aria-keyshortcuts") === "Meta+Shift+Enter", JSON.stringify(await foot(page)));
      check(`AC-281 ${phone ? "a touch screen draws no underline" : "a keyboard's screen draws the underline"}`,
        await page.$eval("#taskbox u.dkey", (u) => getComputedStyle(u).textDecorationLine) === (phone ? "none" : "underline"));
      await comment.focus();
      await page.keyboard.press("Meta+Shift+KeyS");
      check("AC-281 an empty comment's Save is off: its key does nothing", (await page.waitForTimeout(200), posts.comments.length === 0) && await isOpen(page));
      await page.keyboard.type("Use v2");
      await page.keyboard.press("Shift+KeyS");
      check("AC-281 a letter typed in a field stays typed", await comment.inputValue() === "Use v2S");
      await page.keyboard.press("Meta+Shift+KeyS");
      check("AC-281 ⌘⇧S in the comment field presses Save: the comment kept, the window open",
        await until(() => posts.comments.length === 1) && posts.comments[0].text === "Use v2S" && await isOpen(page));

      await page.keyboard.press("Meta+Slash");
      await page.waitForSelector("#keyhelp[open]");
      const help = (await page.textContent("#keyhelpbody")).replaceAll("\uFE0E", "");
      check("AC-281 ⌘/ opens the help: the window's keys, every window's, the page's", /Task API-1/.test(help) && /⌘⇧S\s*Save/.test(help) && /⌘⇧↩\s*Run/.test(help)
        && /Esc/.test(help) && /On this page/.test(help), help);
      await page.keyboard.press("Escape");
      check("AC-281 Esc closes the help alone", await until(async () => !await isOpen(page, "#keyhelp")) && await isOpen(page));

      await comment.fill("Yes, go on with v2");
      await page.keyboard.press("Meta+Shift+Enter");
      check("AC-282 ⌘⇧↩ sends the answer with the run, and the task's window closes",
        await until(async () => posts.runs.length === 1 && !await isOpen(page)) && JSON.stringify(posts.runs[0]).includes("Yes, go on with v2"), JSON.stringify(posts.runs));

      await open(page, "API-2");
      await until(async () => (await foot(page)).Archive === "A");
      await page.keyboard.press("Meta+Shift+KeyA");
      check("AC-282 Archive archives and closes the window", await until(async () => posts.archive.includes("API-2") && !await isOpen(page)));
      await open(page, "API-3");
      await until(async () => (await foot(page)).Archive === "A");
      await page.keyboard.press("Meta+Shift+KeyA");
      check("AC-282 a refused Archive keeps the window open and says why", await until(() => posts.archive.includes("API-3"))
        && await until(async () => /Archive refused/.test(await page.textContent("body"))) && await isOpen(page));
      await page.click("#taskclose");

      if (!phone) {
        // + Session's dock: Discard by its key, then its question's Discard by the same key over it
        await page.click('#agentbar .agentcard[data-agent="a1"] .agsess');
        await page.waitForSelector("#sesscompose:not([hidden])");
        await page.keyboard.type("A prompt to drop");
        const want = JSON.stringify({ "Add files…": "A", Discard: "D", Start: "S" });
        check("AC-281 + Session's keys: Add files, Discard, Start", await until(async () => JSON.stringify(await foot(page, "#sesscompose .sessfoot")) === want),
          JSON.stringify(await foot(page, "#sesscompose .sessfoot")));
        await page.keyboard.press("Meta+Shift+KeyD");
        await page.waitForSelector("#confirmbox[open]");
        await page.keyboard.press("Meta+Shift+KeyD");
        check("AC-281 ⌘⇧D presses Discard, then its question's Discard over it", await until(async () => !await isOpen(page, "#confirmbox") && await page.inputValue("#sesstext") === ""));
        check("AC-281 + Session's Start never pressed by those", posts.sessions.length === 0);
      }
    } catch (e) { check(`AC-281 ${size} ran to its end`, false, String(e.stack ?? e)); }
    finally { await f.close(); }
  }

  // off a Mac: Ctrl+Shift; ⌘⇧ there is nobody's
  runningIn(`${name} Linux`);
  const l = await board(browser, SIZES[0][1], "Linux x86_64");
  try {
    await open(l.page, "API-2");
    await until(async () => (await foot(l.page)).Archive === "A");
    check("AC-281 off a Mac the key is named Ctrl+Shift", await l.page.locator("#taskbox").getByRole("button", { name: "Archive", exact: true }).getAttribute("aria-keyshortcuts") === "Control+Shift+A");
    await l.page.keyboard.press("Meta+Shift+KeyA");
    check("AC-281 off a Mac ⌘⇧A presses nothing", (await l.page.waitForTimeout(200), !l.posts.archive.length));
    await l.page.keyboard.press("Control+Shift+KeyA");
    check("AC-281 off a Mac Ctrl+Shift+A presses Archive", await until(async () => l.posts.archive.includes("API-2") && !await isOpen(l.page)));
  } catch (e) { check("AC-281 Linux ran to its end", false, String(e.stack ?? e)); }
  finally { await l.close(); }

  // the session page with no window open: the terminal keeps ? and ⌘⇧A
  runningIn(`${name} terminal`);
  const t = await openPage(browser, {});
  try {
    await drawn(t.page);
    await t.page.focus("#kbd");
    await t.page.keyboard.type("a?");
    await t.page.keyboard.press("Control+Slash");
    const ins = () => t.sent.filter((m) => m.t === "in").map((m) => JSON.stringify(m)).join("");
    check("AC-281 with no window open the terminal gets its keys, ? and Ctrl+/ among them", await until(() => /a/.test(ins()) && /\?/.test(ins()) && /\\u001f/.test(ins())), ins());
    check("AC-281 the terminal's keys open no help", !await t.page.$("#keyhelp[open]"));
  } catch (e) { check("AC-281 terminal ran to its end", false, String(e.stack ?? e)); }
  finally { await t.close(); }
}
