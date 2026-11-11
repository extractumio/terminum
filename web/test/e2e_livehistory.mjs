// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Message history and the Dashboard against a real private terminumd and tmux (e2e_live.mjs;
// terminum.md AC-151, AC-152; terminum-terminal.md AC-90): an agent's prompt from its hooks and a
// shell's command from the page land in the store; words find them by rank, filters narrow them in
// the address, Back undoes one; Open in session shows the pane at the message; the Dashboard counts
// the human's only and a day leads to the same count in the history.
import path from "node:path";
import { boardKit, intoStore, nap } from "./boardkit.mjs";
import { check, until } from "./check.mjs";

const rows = (page) => page.$$eval("#results .msg", (r) => r.map((x) => x.querySelector(".mtext").textContent));

export async function liveHistory(page, opts) {
  const { base, app, work, shots } = opts;
  const { newPane, keyOf, hook, endPanes } = boardKit(page, opts);
  const agent = newPane("histagent"), key = await keyOf(agent);
  hook(agent, "SessionStart", { source: "startup" });
  hook(agent, "UserPromptSubmit", { prompt: "make the board wait for the board selector", prompt_id: "h1" });
  hook(agent, "UserPromptSubmit", { prompt: "Исправь поиск по сообщениям", prompt_id: "h2" });
  // what Dispatcher typed, as its gate keeps it (a real run needs an agent: e2e_livedispatcher.mjs)
  intoStore(app, `INSERT INTO messages (at, sender, via, agent, pane, host, cwd, text) VALUES (CAST(strftime('%s','now') AS INTEGER) * 1000, 'dispatcher', 'hook', 'claude', '${key}', 'local', '${work}', 'T-0007: the board task for Dispatcher')`);
  // a shell's command, typed in the page: what stood after the prompt, with its line
  await page.goto(`${base}/sessions/`);
  await page.waitForFunction(() => document.querySelectorAll("#sessions .item").length > 0, null, { timeout: 15000 });
  await page.click("#sessions .item");
  await page.waitForSelector("#term", { state: "visible" });
  await page.evaluate(() => document.getElementById("kbd").focus());
  // Enter tells what the screen shows at the cursor: the typed line, once it is drawn (one more
  // than before, counted on the drawn screen: the browser before this one typed the same line here)
  await page.waitForFunction(() => document.querySelectorAll("#screen .ln").length > 3, null, { timeout: 8000 });
  const typed = () => page.evaluate(() => document.getElementById("screen").innerText.split("echo history probe words").length - 1);
  const before = await typed();
  await page.keyboard.type("echo history probe words");
  check("AC-151 the typed line is drawn before its Enter", await until(() => typed().then((n) => n > before)));
  await page.keyboard.press("Enter");
  await nap(600);

  check("AC-167 the session page's sections lead to the Message history", (await page.$$('#nav .sections a[href="/history/"]')).length === 1);
  await page.goto(`${base}/history/`);
  const shownSection = () => page.evaluate(() => [...document.querySelectorAll("#bar .sections a")].map((a) => `${a.getAttribute("aria-label")}${a.getAttribute("aria-current") ? "*" : ""}${a.querySelector(".word").getBoundingClientRect().width > 0 ? "" : " (icon)"}`).join());
  check("AC-167 the Message history's head holds the sections with their words, Messages marked", await until(async () => await shownSection() === "Sessions,Kanban,Dashboard,Messages*"), await shownSection());
  check("AC-167 Messages' two pages stand first in its tools, History marked", await page.getAttribute('.mpages a[href="/history/"]', "aria-current") === "page" && await page.isVisible('.mpages a[href="/scheduled/"]'));
  await page.goto(`${base}/dashboard/`);
  check("AC-167 the Dashboard's head marks Dashboard", await until(async () => await shownSection() === "Sessions,Kanban,Dashboard*,Messages"), await shownSection());
  await page.goto(`${base}/scheduled/`);
  check("AC-167 Scheduled is Messages' page: Messages marked", await until(async () => await shownSection() === "Sessions,Kanban,Dashboard,Messages*"), await shownSection());
  await page.click('#bar .sections a[href="/history/"]');
  await page.waitForSelector("#results");
  check("AC-151 newest first, every message kept, Dispatcher's too", await until(async () => (await rows(page)).length >= 4, 8000), JSON.stringify(await rows(page)));
  const first = await rows(page);
  check("AC-151 the newest is the shell's command", first[0] === "echo history probe words", first[0]);
  check("AC-151 under a day's head with its count", /^Today · \d+/.test(await page.textContent("#results .dayhead")), await page.textContent("#results .dayhead"));
  check("AC-151 Dispatcher's is marked", await page.isVisible("#results .msg .mbot"));
  check("AC-151 the day strip draws a column a day", (await page.$$("#stripplot rect.hit")).length >= 30);

  await page.fill("#q", "boar");
  check("AC-151 words find by their start, the denser match first", await until(async () => (await rows(page)).length === 2 && (await rows(page))[0].includes("make the board"), 6000), JSON.stringify(await rows(page)));
  check("AC-151 the hits are marked", (await page.textContent("#results .msg mark.hit")) === "board");
  check("AC-151 ranked by relevance with words", await page.getAttribute('#hsort [data-sort="best"]', "aria-pressed") === "true");
  check("AC-151 the words are in the address", new URL(page.url()).searchParams.get("q") === "boar", page.url());
  await page.click("#fsender summary");
  await page.click('#fsender .pname:text-is("You")');
  check("AC-151 From: You leaves Dispatcher's out", await until(async () => (await rows(page)).length === 1, 6000), JSON.stringify(await rows(page)));
  check("AC-151 the filter is in the address", new URL(page.url()).searchParams.get("sender") === "human", page.url());
  await page.goBack();
  check("AC-151 Back undoes the filter", await until(async () => (await rows(page)).length === 2, 6000) && !new URL(page.url()).searchParams.get("sender"), page.url());
  await page.fill("#q", "поиск");
  check("AC-151 a Cyrillic word is found, any case", await until(async () => JSON.stringify(await rows(page)) === JSON.stringify(["Исправь поиск по сообщениям"]), 6000), JSON.stringify(await rows(page)));
  await page.fill("#q", "-board");
  check("AC-151 a -word alone ranks nothing: newest first", await until(async () => (await rows(page)).length >= 4, 6000) && await page.isDisabled('#hsort [data-sort="best"]'));
  await page.fill("#q", "");
  await until(async () => (await rows(page)).length >= 4, 6000);
  await page.click("#fkind summary");
  await page.click('#fkind .pname:text-is("Shells\' commands")');
  const shellsOnly = () => page.$$eval("#results .msg", (r) => r.length > 0 && r.every((x) => x.querySelector(".mtext.shell")));
  check("AC-151 Where: shells only", await until(async () => (await rows(page)).includes("echo history probe words") && await shellsOnly(), 6000), JSON.stringify(await rows(page)));
  if (shots) await page.screenshot({ path: path.join(shots, "history.png") });

  // Open in session: the pane, the message brought into view (AC-90's Jump)
  await page.click("#results .msg");
  check("AC-151 a pressed row opens with its keys", await page.isVisible("#results .msg.open .macts"));
  await page.click('#results .msg.open .macts button:has-text("Context")');
  check("AC-151 Context: what was sent around it in that pane", await until(() => page.isVisible("#results .ctx .ctxrow.self"), 4000));
  const href = await page.getAttribute("#results .msg.open .macts a", "href");
  check("AC-151 Open in session goes to its pane, at the message", /^\/sessions\/\?then=msg:\d+#\//.test(href ?? ""), href);
  // the session page's answer to its ask by id (sent.js): proof the hand-off reached it
  const jumped = [];
  page.on("websocket", (ws) => ws.on("framereceived", ({ payload }) => {
    try { const m = JSON.parse(String(payload)); if (m.t === "sentone" && m.id !== undefined) jumped.push(m); } catch { /* not JSON */ }
  }));
  await page.click("#results .msg.open .macts a");
  await page.waitForURL(/\/sessions\//);
  check("AC-151 the session page opens without its hand-off in the address", await until(() => !page.url().includes("then="), 8000), page.url());
  check("AC-151 the message is brought into view where it was sent", await until(() => page.evaluate(() => [...document.querySelectorAll("#screen .ln")].some((l) => l.textContent.includes("echo history probe words"))), 8000));
  // the line may stand on the screen already: the answer to the ask comes on its own time
  check("AC-151 the jump asked for the message by its id", await until(() => jumped.length > 0) && jumped.every((m) => m.item?.text === "echo history probe words"), JSON.stringify(jumped).slice(0, 300));

  // the Dashboard: the human's only (earlier suites in this run sent messages too: counted from the store)
  const day = await page.evaluate(() => { const t = new Date(); return `${t.getFullYear()}-${String(t.getMonth() + 1).padStart(2, "0")}-${String(t.getDate()).padStart(2, "0")}`; });
  const totalOf = (q) => page.evaluate(async (q) => (await (await fetch(`/api/messages?${q}`)).json()).total, q);
  const human = await totalOf(`from=${day}&to=${day}&sender=human`);
  const numbers = (q) => page.evaluate(async (q) => (await fetch(`/api/dashboard?${q}`)).json(), q);
  const you = await numbers("tab=you&period=month"), mine = you.series.at(-1);
  check("AC-152 the You tab counts the human's time, of one project when asked", mine.sent > 0 && mine.active_ms > 0 && you.tiles.active_units >= 1 && (await numbers("tab=you&period=month&project=nosuch")).tiles.active_ms === 0, JSON.stringify([mine, human]));
  await page.goto(`${base}/dashboard/`);
  await page.waitForSelector("#tiles .tile");
  const tileOf = (label) => page.evaluate((label) => [...document.querySelectorAll("#tiles .tile")].find((t) => t.querySelector(".tlabel").textContent === label)?.querySelector(".tvalue").textContent, label);
  const labels = () => page.$$eval("#tiles .tlabel", (l) => l.map((x) => x.textContent).join());
  check("AC-152 the first tab is You, its figures the human's time and load", await page.getAttribute('#dtabs [data-tab="you"]', "aria-selected") === "true" && await tileOf("Active time") !== "0 min"
    && await labels() === "Active time,Active days,Engagement,Longest stretch,Switches an active hour,Board actions,Waited for you,Off hours,From the phone", await page.textContent("#tiles"));
  check("AC-152 every card of You draws: a month by the day", (await page.$$("#active rect.hit")).length === 30 && (await page.$$("#engaged rect.hit")).length === 30 && (await page.$$("#days rect.hit")).length === 30 && (await page.$$("#heat rect.cell")).length === 7 * 24
    && await page.isVisible("#projects .rrow") && (await page.$$("#juggling rect.hit")).length === 30 && (await page.$$("#board rect.hit")).length === 30);
  // the document scrolls: the cards under the window's end are reached
  const size = page.viewportSize();
  await page.setViewportSize({ width: size.width, height: 500 });
  await page.evaluate(() => scrollTo(0, document.documentElement.scrollHeight));
  check("AC-152 the page scrolls to its last card", await until(() => page.evaluate(() => scrollY > 0 && document.querySelector("#cboard").getBoundingClientRect().bottom <= innerHeight + 1)), await page.evaluate(() => [scrollY, document.documentElement.scrollHeight, innerHeight].join()));
  await page.setViewportSize(size);
  await page.focus("#active");
  await page.keyboard.press("End");
  check("AC-152 a day chosen by the keys says its active time", /^Today so far · \d/.test(await page.textContent("#cactive .readout span")), await page.textContent("#cactive .readout span"));
  await page.keyboard.press("Enter");
  await page.waitForURL(/\/history\/\?/);
  check("AC-152 a day leads to the history of that day, the human's", await until(async () => (await page.textContent("#hsum")).startsWith(`${human.toLocaleString("en-US")} message`), 6000), await page.textContent("#hsum"));
  await page.goto(`${base}/dashboard/?period=2w`);
  check("AC-152 the period is in the address: two weeks", await until(async () => (await page.$$("#active rect.hit")).length === 14, 6000));
  await page.click('#dperiod [data-period="day"]');
  check("AC-152 a day is drawn by the hour, with no working day and no week", await until(async () => (await page.$$("#active rect.hit")).length === 24 && /period=day/.test(page.url()), 6000) && !(await page.$("#days")) && !(await page.$("#heat"))
    && (await labels()).split(",")[1] === "Active hours", await labels());
  for (const [period, n] of [["quarter", 13], ["year", 12]]) {
    await page.click(`#dperiod [data-period="${period}"]`);
    check(`AC-152 a ${period} is drawn in ${n} columns`, await until(async () => (await page.$$("#active rect.hit")).length === n, 6000));
  }
  check("AC-152 what is kept does not reach the year before: no comparison, and the page says from when it counts", /no earlier data/.test(await page.textContent("#tiles .tile .tsub")) && /typing counted from (Today|Yesterday)\./.test(await page.textContent("#dnote")), await page.textContent("#dnote"));
  // the other tabs: a session of Dispatcher's, a failed write and a report in the store (e2e_livedispatcher.mjs runs real sessions)
  // what other suites of this run left is counted too: the figures are told by what this adds
  const dash = `dash${Date.now()}`, sys0 = await numbers("tab=system&period=year"), cost0 = await numbers("tab=costs&period=year");
  intoStore(app, `INSERT INTO agents (id, name, kind, command, color, at_once) VALUES ('${dash}', 'Agent ${dash}', 'claude', 'claude', 2, 1);
    INSERT INTO runs (id, project, agent_id, agent_name, agent_color, kind, command, folder, state, last_walk) VALUES ('${dash}', 'default', '${dash}', 'Agent ${dash}', 2, 'claude', 'claude', '${work}', 'closed', CAST(strftime('%s','now') AS INTEGER) - 600);
    INSERT INTO inputs (at, ended, sender, component, kind, text, bytes, pane, run, version) VALUES ((CAST(strftime('%s','now') AS INTEGER) - 2400) * 1000, 0, 'dispatcher', 'dispatcher.start', 'launch', 'claude', 6, '${key}', '${dash}', 'test');
    INSERT INTO inputs (at, ended, sender, component, kind, text, bytes, pane, error, version) VALUES ((CAST(strftime('%s','now') AS INTEGER) - 900) * 1000, 0, 'human', 'page', 'typed', 'x', 1, '${key}', 'This pane has closed.', 'test');
    INSERT INTO usage (run, agent_id, at, input, output, cache_write, cache_read, model) VALUES ('${dash}', '${dash}', CAST(strftime('%s','now') AS INTEGER) - 700, 1000000, 100000, 0, 5000000, 'claude-opus-5-5')`);
  // a ranked row's text without the mark the page sets after a symbol
  const rowOf = (list, name) => page.evaluate(([list, name]) => ([...document.querySelectorAll(`#${list} .rrow`)].find((r) => r.textContent.includes(name))?.textContent ?? "").replace("︎", ""), [list, name]);
  await page.click('#dtabs [data-tab="system"]');
  check("AC-171 System: Dispatcher's sessions, the failures by reason, the agents' load", await until(async () => await tileOf("Sessions started") === String(sys0.tiles.sessions + 1), 6000)
    // its failed write by its count: among the reasons shown it falls under Other once the suites before left ten kinds
    && /tab=system/.test(page.url()) && (await numbers("tab=system&period=year")).tiles.failed_writes === sys0.tiles.failed_writes + 1 && /Write: This pane has closed|Other \(\d+\)/.test(await page.textContent("#failures")) &&new RegExp(`Agent ${dash}\\d+%1$`).test(await rowOf("agents", dash))
    && (await numbers("tab=system&period=year")).agents.find((a) => a.id === dash)?.busy_ms === 1_800_000
    && (await page.$$("#ends rect.hit")).length === 12 && (await page.$$("#load rect.hit")).length === 12 && (await page.$$("#fires rect.hit")).length === 12, `${await page.textContent("#tiles")} | ${await rowOf("agents", dash)}`);
  await page.click('#dtabs [data-tab="costs"]');
  const cost1 = await numbers("tab=costs&period=year");
  check("AC-172 Costs: dollars at list prices and tokens, by agent, project and model", Math.abs(cost1.tiles.usd - cost0.tiles.usd - 7) < 0.005 && cost1.tiles.counted - cost0.tiles.counted === 1_100_000
    && await until(async () => (await tileOf("Spent"))?.replace("︎", "") === `≈$${cost1.tiles.usd.toFixed(2)}`, 6000) && (await rowOf("agents", dash)).endsWith("≈$7.00")
    && /claude-opus-5-5/.test(await page.textContent("#models")) && (await page.textContent("#limits")).includes(`Agent ${dash} · week`) && /Dispatcher started/.test(await page.textContent("#dnote")), `${await page.textContent("#tiles")} | ${await rowOf("agents", dash)}`);
  await page.click('#cspent [data-mode="tokens"]');
  check("AC-172 the charts measure tokens by a key, which stays pressed", await until(async () => (await rowOf("agents", dash)).endsWith("1.1M")) && await page.getAttribute('#cspent [data-mode="tokens"]', "aria-pressed") === "true", await rowOf("agents", dash));
  await page.goBack();
  check("AC-171 Back returns to the tab before", await until(async () => await page.getAttribute('#dtabs [data-tab="system"]', "aria-selected") === "true" && !!(await page.$("#failures")), 6000));
  // the Tasks tab: a task the human makes from this page, a comment an agent left on it
  const work0 = await numbers("tab=tasks&period=year");
  const made = await page.evaluate(async (title) => (await (await fetch("/api/tasks", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ title }) })).json()).id, `Split ${dash}`);
  intoStore(app, `INSERT INTO users (id, kind, name, role) VALUES ('agent:${dash}', 'agent', 'Agent ${dash}', 'agent');
    INSERT INTO events (id, task, at, author, kind, text) VALUES ('ev${dash}', (SELECT id FROM tasks WHERE slug = '${made}'), CAST(strftime('%s','now') AS INTEGER), 'agent:${dash}', 'comment', 'done')`);
  await page.click('#dtabs [data-tab="tasks"]');
  const work1 = await numbers("tab=tasks&period=year");
  check("AC-176 Tasks: the operations by who decided them, and each task's split", work1.tiles.by.you === work0.tiles.by.you + 1 && work1.tiles.by.agents === work0.tiles.by.agents + 1
    && await until(async () => await tileOf("Your operations") === String(work1.tiles.by.you), 6000) && /tab=tasks/.test(page.url()) && (await rowOf("tasks", made)).endsWith("50%2")
    && (await page.$$("#ops rect.hit")).length === 12 && /Made/.test(await page.textContent("#kinds")) && await page.isVisible("#spread .rrow"), `${made} | ${await page.textContent("#tiles")} | ${await rowOf("tasks", made)}`);
  await page.evaluate((id) => [...document.querySelectorAll("#tasks .rrow")].find((r) => r.textContent.includes(id)).click(), made);
  check("AC-176 a task's row opens the task", await until(() => page.url().includes(`/kanban/${made}`), 6000), page.url());
  // what the user does in a page is counted by the minute and sent when the page goes
  await page.goto(`${base}/dashboard/`);
  await page.waitForSelector("#tiles .tile");
  await page.keyboard.press("Tab");
  await page.mouse.move(300, 300);
  await page.mouse.wheel(0, 200);
  await page.click("#dnote");
  await page.goto(`${base}/dashboard/?period=day`);
  const counts = () => numbers("tab=you&period=day");
  check("AC-175 a page counts the user's presses, keys and scrolls and sends them as it goes: engagement is counted from them", await until(async () => (await counts()).tiles.acting_min >= 1, 8000)
    && !!(await counts()).measured_from && await page.reload().then(() => page.waitForSelector("#tiles .tile")).then(() => tileOf("Engagement")).then((v) => /^\d+%$/.test(v))
    && (await page.$$("#engaged rect.hit")).length === 24, `${JSON.stringify((await counts()).tiles)} | ${await page.textContent("#dnote")}`);
  const refused = await page.evaluate(async () => (await fetch("/api/interactions", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ now: Date.now(), rows: [{ at: Date.now(), ended: Date.now(), kind: "typed", surface: "sessions", n: 1 }] }) })).json());
  check("AC-175 a count of a kind no page counts is not kept", refused.kept === 0, JSON.stringify(refused));
  if (shots) for (const t of ["tasks", "system", "costs"]) { await page.goto(`${base}/dashboard/?tab=${t}`); await page.waitForSelector("#tiles .tile"); await nap(400); await page.screenshot({ path: path.join(shots, `dashboard-${t}.png`), fullPage: true }); }
  if (shots) {
    // a month of made-up work to look at: three projects, days of different length, a weekend off
    const seed = [];
    for (let d = 1; d < 30; d++) {
      if (d % 7 === 2) continue;
      for (let k = 0; k < 20 + ((d * 37) % 60); k++) {
        const at = `(CAST(strftime('%s', date('now', 'localtime', '-${d} days'), '+${8 + (k * 7) % 12} hours', '+${(k * 13) % 60} minutes', 'utc') AS INTEGER) * 1000)`;
        const project = ["pa", "pb", "pc", null][k % 4];
        seed.push(`(${at}, 'human', 'hook', ${k % 3 ? "'claude'" : "NULL"}, 'k${k % 5}', 'local', '${work}', ${project ? `'${project}', 'Project ${project}'` : "NULL, NULL"}, 'seeded message ${d} ${k}')`);
      }
    }
    intoStore(app, `INSERT INTO messages (at, sender, via, agent, pane, host, cwd, project, project_name, text) VALUES ${seed.join(",")}`);
    await page.goto(`${base}/dashboard/`);
    await page.waitForSelector("#tiles .tile");
    await nap(500);
    await page.screenshot({ path: path.join(shots, "dashboard.png"), fullPage: true });
    await page.goto(`${base}/history/?q=seeded`);
    await page.waitForSelector("#results .msg");
    await page.screenshot({ path: path.join(shots, "history-seeded.png") });
  }
  endPanes();
}

/** Both pages at a phone's width: one column, no sideways scroll, the filters behind one key. */
export async function liveHistoryPhone(page, { base, shots }) {
  for (const p of ["history", "dashboard"]) {
    await page.goto(`${base}/${p}/`);
    await page.waitForSelector(p === "history" ? "#results .msg, #results .mempty" : "#tiles .tile");
    const wide = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
    check(`AC-${p === "history" ? 151 : 152} ${p} at a phone's width has no sideways scroll`, wide <= 0, String(wide));
    const keys = await page.evaluate(() => [...document.querySelectorAll("#bar .sections a")].map((a) => { const b = a.getBoundingClientRect(); return [a.querySelector(".word").getBoundingClientRect().width > 0 ? a.textContent : "", Math.round(b.width), Math.round(b.height)]; }));
    check(`AC-167 ${p} at a phone's width: the sections across its head, the shown one's word, each a finger's size`,
      keys.map((k) => k[0]).join() === (p === "history" ? ",,,Messages" : ",,Dashboard,") && keys.every((k) => k[1] >= 44 && k[2] >= 40), JSON.stringify(keys));
    if (shots) await page.screenshot({ path: path.join(shots, `${p}-phone.png`), fullPage: true });
  }
  check("AC-151 the phone's filters stand behind one key", await page.goto(`${base}/history/`).then(() => page.isVisible("#hfilters")) && !(await page.isVisible("#fsender")));
}
