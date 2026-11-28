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
/** A row of the rail's group `group` (kind, sender, when, project) by its words. */
const railRow = (group, label) => `#f${group} .frow:has(.pname:text-is("${label}"))`;

/** AC-152's selected day and navigation readiness, before Enter and after a timeout. */
const dayState = async (page) => ({ url: page.url(), ...await page.evaluate(() => {
  const plot = document.getElementById("active"), go = document.querySelector("#cactive .readout a");
  return { ready: document.readyState, focus: document.activeElement?.id || document.activeElement?.tagName,
    chosen: plot?.dataset.chosen, marked: [...(plot?.querySelectorAll(".hit") ?? [])].findIndex((m) => m.classList.contains("on")),
    readout: document.querySelector("#cactive .readout span")?.textContent, href: go?.getAttribute("href"), hidden: go?.hidden,
    summary: document.getElementById("hsum")?.textContent, error: document.getElementById("derr")?.textContent };
}).catch((e) => ({ unreadable: e.message })) });

export async function liveHistory(page, opts) {
  const { base, app, work, shots } = opts;
  const { newPane, keyOf, hook, endPanes } = boardKit(page, opts);
  const agent = newPane("histagent"), key = await keyOf(agent);
  hook(agent, "SessionStart", { source: "startup" });
  hook(agent, "UserPromptSubmit", { prompt: "make the board wait for the board selector", prompt_id: "h1" });
  hook(agent, "UserPromptSubmit", { prompt: "Исправь поиск по сообщениям", prompt_id: "h2" });
  // what Dispatcher typed, as its gate keeps it (a real run needs an agent: e2e_livedispatcher.mjs)
  // and what a project's manager typed (send_to_session), the session's tasks kept with each
  intoStore(app, `INSERT OR IGNORE INTO users (id, kind, name, role) VALUES ('manager', 'agent', 'Manager', 'agent');
    INSERT INTO messages (at, sender, via, agent, pane, host, cwd, title, tasks, text) VALUES
      (CAST(strftime('%s','now') AS INTEGER) * 1000, 'dispatcher', 'hook', 'claude', '${key}', 'local', '${work}', 'histagent', 'T-0007,T-0008,T-0009', 'T-0007: the board task for Dispatcher'),
      (CAST(strftime('%s','now') AS INTEGER) * 1000 + 1, 'manager', 'hook', 'codex', '${key}', 'local', '${work}', 'histagent', 'T-0007', 'say where its task stands')`);
  // a shell's command, typed in the page: what stood after the prompt, with its line. In a pane of its own: the
  // list's first row is whatever the suites before left there, and a row an agent's mark stands on sends prompts
  // (in the test's short folder: a prompt with the runner's checkout in it wraps the typed line). Its window is named
  // after an agent: a shell there is no agent's, and what is typed in it is kept as a command (AC-39 a named pane, a shell)
  const shell = newPane("codex", work), shellKey = await keyOf(shell);
  await page.goto(`${base}/sessions/`);
  await page.waitForSelector(`#sessions .item[data-id="${shellKey}"]`, { timeout: 15000 });
  await page.click(`#sessions .item[data-id="${shellKey}"]`);
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
  check("AC-151 newest first, every message kept, Dispatcher's and a manager's too", await until(async () => (await rows(page)).length >= 5, 8000), JSON.stringify(await rows(page)));
  const first = await rows(page);
  check("AC-151 the newest is the shell's command", first[0] === "echo history probe words", first[0]);
  check("AC-151 under a day's head with its count", /^Today · \d+/.test(await page.textContent("#results .dayhead")), await page.textContent("#results .dayhead"));
  // a row's head reads from its time: who, to what, the session, its project and tasks, right over the text
  const headOf = (text) => page.evaluate((text) => {
    const row = [...document.querySelectorAll("#results .msg")].find((r) => r.querySelector(".mtext").textContent === text);
    const left = (sel) => row.querySelector(sel)?.getBoundingClientRect().left, top = (sel) => row.querySelector(sel).getBoundingClientRect().top;
    return { words: [...row.querySelector(".mhead").children].map((c) => c.textContent).join("|").replace("\ufe0e", ""), who: row.querySelector(".mwho").className,
      order: [".mtime", ".mwho", ".mto", ".msess", ".mtags"].map(left), gap: top(".mtext") - row.querySelector(".mhead").getBoundingClientRect().bottom, time: left(".mtime") - row.getBoundingClientRect().left };
  }, text);
  const sent = await headOf("T-0007: the board task for Dispatcher");
  check("AC-151 a row says when, who sent it to what, in which session, for which tasks: Dispatcher's two tasks and how many more", /^\d\d:\d\d\|Dispatcher\|→\|Claude Code\|histagent\|T-0007T-0008\+1$/.test(sent.words) && /by-system/.test(sent.who), JSON.stringify(sent));
  check("AC-151 the head starts with its time at the row's left, each part after the one before, the text right under it", sent.time < 30 && sent.order.every((x, i) => i === 0 || x > sent.order[i - 1]) && sent.gap >= 0 && sent.gap < 8, JSON.stringify(sent));
  const managed = await headOf("say where its task stands");
  check("AC-151 a manager's message says Manager, to Codex", /\|Manager\|→\|Codex\|histagent\|T-0007$/.test(managed.words) && /by-agent/.test(managed.who), JSON.stringify(managed));
  const shellHead = await headOf("echo history probe words");
  check("AC-151 a shell's command says You, to a shell", /^\d\d:\d\d\|You\|→\|Shell\|/.test(shellHead.words) && /by-human/.test(shellHead.who), JSON.stringify(shellHead));
  const wide = await page.evaluate(() => { const r = (id) => document.getElementById(id).getBoundingClientRect(); return { main: r("hmain").width, rail: r("hrail"), left: r("hmain").left, window: innerWidth }; });
  check("AC-151 the list is a reading column beside the quick filters' rail, however wide the window", wide.main <= 900 && wide.rail.width > 200 && wide.rail.right <= wide.left && wide.window >= 1400, JSON.stringify(wide));
  const railCounts = () => page.evaluate(() => Object.fromEntries([...document.querySelectorAll("#hrail .frow")].filter((r) => r.querySelector(".fnum")).map((r) => [r.querySelector(".pname").textContent, Number(r.querySelector(".fnum").textContent.replaceAll(",", ""))])));
  const all = await railCounts(), total = Number((await page.textContent("#hsum")).replace(/\D/g, ""));
  check("AC-151 the rail counts what each choice would show: prompts and commands, each agent, each sender", all.Prompts + all.Commands === total && all["Claude Code"] + all.Codex === all.Prompts && all.You + all.Dispatcher + all.Manager === total
    && all.Dispatcher >= 1 && all.Manager === 1 && all.Codex >= 1 && all.Today >= 1 && all.Today <= total && all["Last 7 days"] === total, JSON.stringify([all, total]));   // a run past midnight leaves some on yesterday
  // by the keys: a row pressed stays in focus though its group is drawn anew, and lets go on the next press
  await page.focus(railRow("sender", "Dispatcher"));
  await page.keyboard.press("Enter");
  const focused = () => page.evaluate(() => `${document.activeElement?.dataset.row}:${document.activeElement?.getAttribute("aria-pressed")}`);
  // other suites of a whole run leave Dispatcher's messages too: every row shown is Dispatcher's, however many
  const onlyDispatchers = () => page.$$eval("#results .msg .mwho", (w) => w.length > 0 && w.every((x) => x.classList.contains("by-system")));
  check("AC-151 a rail's row pressed by the keys keeps the focus, pressed", await until(async () => await onlyDispatchers() && await focused() === "Dispatcher:true", 6000), await focused());
  await page.keyboard.press("Enter");
  check("AC-151 pressed again by the keys it lets go, still in focus", await until(async () => (await rows(page)).length >= 5 && await focused() === "Dispatcher:false", 6000), await focused());
  check("AC-151 the day strip draws a column a day", (await page.$$("#stripplot rect.hit")).length >= 30);

  await page.fill("#q", "boar");
  check("AC-151 words find by their start, the denser match first", await until(async () => (await rows(page)).length === 2 && (await rows(page))[0].includes("make the board"), 6000), JSON.stringify(await rows(page)));
  check("AC-151 the hits are marked", (await page.textContent("#results .msg mark.hit")) === "board");
  check("AC-151 ranked by relevance with words", await page.getAttribute('#hsort [data-sort="best"]', "aria-pressed") === "true");
  check("AC-151 the words are in the address", new URL(page.url()).searchParams.get("q") === "boar", page.url());
  await page.click(railRow("sender", "You"));
  check("AC-151 From: You, one press, leaves Dispatcher's out", await until(async () => (await rows(page)).length === 1, 6000), JSON.stringify(await rows(page)));
  check("AC-151 the filter is in the address, its row pressed, a chip says it", new URL(page.url()).searchParams.get("sender") === "human" && await page.getAttribute(railRow("sender", "You"), "aria-pressed") === "true"
    && await page.textContent("#hchips .hchip") === "From You", `${page.url()} | ${await page.textContent("#hchips")}`);
  const narrowed = await railCounts();
  check("AC-151 a group's own choice does not narrow its own counts; the words and the other groups' do", narrowed.Dispatcher === 1 && narrowed.You === 1 && narrowed.Prompts === 1 && narrowed.Commands === 0, JSON.stringify(narrowed));
  await page.goBack();
  check("AC-151 Back undoes the filter", await until(async () => (await rows(page)).length === 2, 6000) && !new URL(page.url()).searchParams.get("sender"), page.url());
  await page.fill("#q", "поиск");
  check("AC-151 a Cyrillic word is found, any case", await until(async () => JSON.stringify(await rows(page)) === JSON.stringify(["Исправь поиск по сообщениям"]), 6000), JSON.stringify(await rows(page)));
  check("AC-151 the best matches have no day heads: each row says its day too", await page.$("#results .dayhead") === null && await page.evaluate(() => document.getElementById("results").classList.contains("dated")));
  await page.fill("#q", "-board");
  check("AC-151 a -word alone ranks nothing: newest first", await until(async () => (await rows(page)).length >= 3, 6000) && await page.isDisabled('#hsort [data-sort="best"]'));
  await page.fill("#q", "");
  await until(async () => (await rows(page)).length >= 5, 6000);
  await page.click(railRow("kind", "Codex"));
  // Earlier suites can leave Codex prompts too. Check the filter against every visible row.
  const onlyCodex = () => page.$$eval("#results .msg", (messages) =>
    messages.some((m) => m.querySelector(".mtext").textContent === "say where its task stands")
    && messages.every((m) => m.querySelector(".mto").textContent === "Codex"));
  check("AC-151 Type: one agent's prompts", await until(onlyCodex, 6000) && new URL(page.url()).searchParams.get("kind") === "codex");
  await page.click(railRow("kind", "Codex"));
  check("AC-151 a chosen row pressed again lets its filter go", await until(async () => (await rows(page)).length >= 5, 6000) && !new URL(page.url()).searchParams.get("kind") && await page.isHidden("#hchips"), page.url());
  // a task's messages, from an opened row's keys; its chip lets it go
  await page.click('#results .msg:has-text("T-0007: the board task for Dispatcher")');
  const keys = await page.$$eval("#results .msg.open :is(.macts, .mfind) :is(a, button)", (k) => k.map((x) => x.textContent).join("|"));
  check("AC-151 an opened row's keys: its session, its tasks, its context, and the list narrowed to them", keys === "Open in session|Open T-0007|Open T-0008|Open T-0009|Context|Copy|This session|T-0007|T-0008|T-0009"
    && await page.getAttribute('#results .msg.open .macts a:text-is("Open T-0008")', "href") === "/kanban/T-0008", keys);
  await page.click('#results .msg.open .mfind button:text-is("T-0008")');
  check("AC-151 Show only a task: the messages sent for it, in the address, a chip", await until(async () => JSON.stringify(await rows(page)) === JSON.stringify(["T-0007: the board task for Dispatcher"]), 6000)
    && new URL(page.url()).searchParams.get("task") === "T-0008" && await page.textContent("#hchips .hchip") === "Task T-0008", `${JSON.stringify(await rows(page))} | ${page.url()}`);
  await page.click("#hchips .hchip");
  await until(async () => (await rows(page)).length >= 5, 6000);
  await page.click('#results .msg:has-text("say where its task stands")');
  await page.click('#results .msg.open .mfind button:text-is("This session")');
  check("AC-151 Show only this session: its pane's messages, the chip naming it", await until(async () => (await rows(page)).length === 4 && new URL(page.url()).searchParams.get("pane") === key && await page.textContent("#hchips .hchip") === "Session: histagent", 6000),
    `${JSON.stringify(await rows(page))} | ${page.url()} | ${await page.textContent("#hchips")}`);
  await page.click(railRow("sender", "Manager"));
  check("AC-151 two filters: a chip each and Clear all, which lets both go", await until(async () => (await rows(page)).length === 1 && (await page.$$("#hchips .hchip")).length === 2, 6000));
  await page.click('#hchips button:text-is("Clear all")');
  check("AC-151 Clear all shows every message again", await until(async () => (await rows(page)).length >= 5 && !new URL(page.url()).search, 6000), page.url());
  await page.click(railRow("kind", "Commands"));
  const shellsOnly = () => page.$$eval("#results .msg", (r) => r.length > 0 && r.every((x) => x.querySelector(".mtext.shell")));
  check("AC-151 Type: commands only", await until(async () => (await rows(page)).includes("echo history probe words") && await shellsOnly(), 6000), JSON.stringify(await rows(page)));
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
    && await page.isVisible("#projects .rrow") && (await page.$$("#juggling rect.hit")).length === 30 && (await page.$$("#board rect.hit")).length === 30 && (await page.$$("#load rect.hit")).length === 30);
  const look = await page.evaluate(() => {
    const tile = [...document.querySelectorAll("#tiles .tile")].find((t) => t.querySelector(".tlabel").textContent === "Active time");
    const dots = (sel) => [...document.querySelectorAll(`${sel} .legend .pdot`)].map((d) => d.style.getPropertyValue("--proj"));
    const [active, load] = ["#cactive", "#cload"].map((id) => document.querySelector(id).getBoundingClientRect());
    return { trend: tile.querySelector(".spark .trend")?.getAttribute("class"), board: dots("#cboard"), juggle: document.querySelectorAll("#cjuggling .legend .chip").length,
      inner: document.querySelectorAll("#juggling .inner").length, cols: document.querySelectorAll("#juggling .col:has(path:not(.inner):not(.sheen))").length,
      beside: Math.abs(active.top - load.top) < 1 && load.left > active.right, stretch: document.querySelector("#days .stretch")?.style.fill ?? "" };
  });
  check("AC-152 the tiles' sparklines draw their trend; Sessions at once a bar inside each column and a legend; the board's kinds no project's colors; Context switching beside Active time; the working day blue by its hour",
    /^trend (good|bad|flat)$/.test(look.trend ?? "") && look.juggle === 2 && look.inner > 0 && look.inner === look.cols && look.board.length > 0 && look.board.every((c) => /--act-|--faint/.test(c))
    && look.beside && /--day-late/.test(look.stretch), JSON.stringify(look));
  await page.click("#dproject summary");
  const menu = await page.evaluate(() => {
    const box = document.querySelector("#dproject .box"), r = box.getBoundingClientRect();
    const under = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    return { bg: getComputedStyle(box).backgroundColor, over: box.contains(under), lefts: [...box.querySelectorAll(".pname")].map((n) => Math.round(n.getBoundingClientRect().left)) };
  });
  check("AC-152 the project's menu stands opaque over the tiles, its names in one column", !/rgba\(0, 0, 0, 0\)|transparent/.test(menu.bg) && menu.over && menu.lefts.length > 1 && new Set(menu.lefts).size === 1, JSON.stringify(menu));
  await page.click("#dproject summary");
  // the projects' answer cut off (as a navigation leaves a read in WebKit, TERM-167): said, the numbers drawn, read again with the next numbers
  const thrown = [], onThrow = (e) => thrown.push(String(e)), said = async () => `${await page.textContent("#derr")} | ${thrown.join(" | ")}`;
  page.on("pageerror", onThrow);
  await page.route("**/api/messages/meta", (r) => r.fulfill({ contentType: "application/json", body: '{"projects": [' }), { times: 1 });
  await page.reload();
  await page.waitForSelector("#tiles .tile");
  check("AC-152 a projects' answer that cannot be read is said, the numbers still drawn, no page error", await until(async () => (await page.textContent("#derr")).startsWith("The projects could not be read:"))
    && await tileOf("Active time") !== "0 min" && thrown.length === 0, await said());
  await page.click('#dperiod [data-period="2w"]');
  check("AC-152 the next numbers read the projects again: the message goes, the menu holds them", await until(async () => await page.textContent("#derr") === "" && (await page.$$("#dproject .box .pname")).length > 1, 6000)
    && thrown.length === 0, await said());
  page.off("pageerror", onThrow);
  await page.goto(`${base}/dashboard/`);
  await page.waitForSelector("#tiles .tile");
  // the document scrolls: the cards under the window's end are reached
  const size = page.viewportSize();
  await page.setViewportSize({ width: size.width, height: 500 });
  await page.evaluate(() => scrollTo(0, document.documentElement.scrollHeight));
  check("AC-152 the page scrolls to its last card", await until(() => page.evaluate(() => scrollY > 0 && document.querySelector("#cboard").getBoundingClientRect().bottom <= innerHeight + 1)), await page.evaluate(() => [scrollY, document.documentElement.scrollHeight, innerHeight].join()));
  await page.setViewportSize(size);
  await page.focus("#active");
  await page.keyboard.press("End");
  check("AC-152 a day chosen by the keys says its active time", /^Today so far · \d/.test(await page.textContent("#cactive .readout span")), await page.textContent("#cactive .readout span"));
  // a scrollbar that comes or goes (Linux's take room) changes the charts' width: drawn anew, the day stays chosen
  await page.setViewportSize({ width: size.width - 15, height: size.height });
  await nap(400);
  check("AC-152 the charts drawn anew keep the day chosen by the keys", /^Today so far · \d/.test(await page.textContent("#cactive .readout span")), await page.textContent("#cactive .readout span"));
  await page.setViewportSize(size);
  await nap(400);
  const beforeOpen = await dayState(page);
  try {
    await page.keyboard.press("Enter");
    await page.waitForURL(/\/history\/\?/);
  } catch (e) {
    console.error(`AC-152 day-to-history navigation: ${JSON.stringify({ before: beforeOpen, after: await dayState(page) })}`);
    throw e;
  }
  const opened = new URL(page.url());
  check("AC-152 the history address keeps the chosen day and human sender", opened.searchParams.get("from") === day
    && opened.searchParams.get("to") === day && opened.searchParams.get("sender") === "human", page.url());
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
  intoStore(app, `INSERT INTO agents (id, project, name, kind, command, color, at_once) VALUES ('${dash}', 'default', 'Agent ${dash}', 'claude', 'claude', 2, 1);
    INSERT INTO runs (id, project, agent_id, agent_name, agent_color, kind, command, folder, state, last_walk) VALUES ('${dash}', 'default', '${dash}', 'Agent ${dash}', 2, 'claude', 'claude', '${work}', 'closed', CAST(strftime('%s','now') AS INTEGER) - 600);
    INSERT INTO inputs (at, ended, sender, component, kind, text, bytes, pane, run, version) VALUES ((CAST(strftime('%s','now') AS INTEGER) - 2400) * 1000, 0, 'dispatcher', 'dispatcher.start', 'launch', 'claude', 6, '${key}', '${dash}', 'test');
    INSERT INTO inputs (at, ended, sender, component, kind, text, bytes, pane, error, version) VALUES ((CAST(strftime('%s','now') AS INTEGER) - 900) * 1000, 0, 'human', 'page', 'typed', 'x', 1, '${key}', 'This pane has closed.', 'test');
    INSERT INTO usage (run, agent_id, at, input, output, cache_write, cache_read, model) VALUES ('${dash}', '${dash}', CAST(strftime('%s','now') AS INTEGER) - 700, 1000000, 100000, 0, 5000000, 'claude-opus-5-5')`);
  // a ranked row's text without the mark the page sets after a symbol
  const rowOf = (list, name) => page.evaluate(([list, name]) => ([...document.querySelectorAll(`#${list} .rrow`)].find((r) => r.textContent.includes(name))?.textContent ?? "").replace("︎", ""), [list, name]);
  await page.click('#dtabs [data-tab="system"]');
  // sessions of Dispatcher's that the suites before left can start meanwhile: the counts hold this one at least,
  // and the page shows what terminumd counts; each part said on its own, so a failure names it
  check("AC-171 System: the tab is in the address", await until(async () => /tab=system/.test(page.url()), 6000), page.url());
  let sys1 = null;
  const counted = await until(async () => {
    sys1 = await numbers("tab=system&period=year");
    return sys1.tiles.sessions >= sys0.tiles.sessions + 1 && await tileOf("Sessions started") === String(sys1.tiles.sessions);
  }, 6000);
  check("AC-171 System: Dispatcher's sessions count this one, as terminumd counts them", counted, `before ${sys0.tiles.sessions}, now ${sys1.tiles.sessions}, shown ${await tileOf("Sessions started")}`);
  // its failed write by its count: among the reasons shown it falls under Other once the suites before left ten kinds
  const reasons = await page.textContent("#failures");
  check("AC-171 System: the failures by reason count its failed write", sys1.tiles.failed_writes >= sys0.tiles.failed_writes + 1 && /Write: This pane has closed|Other \(\d+\)/.test(reasons),
    `before ${sys0.tiles.failed_writes}, now ${sys1.tiles.failed_writes} | ${reasons}`);
  const row = await rowOf("agents", dash), busy = sys1.agents.find((a) => a.id === dash)?.busy_ms;
  check("AC-171 System: the agents' load, its run's half hour busy", new RegExp(`Agent ${dash}\\d+%1$`).test(row) && busy === 1_800_000, `${row} | busy ${busy}`);
  const columns = JSON.stringify(await Promise.all(["ends", "load", "fires"].map(async (id) => (await page.$$(`#${id} rect.hit`)).length)));
  check("AC-171 System: a year in 12 columns on each chart", columns === "[12,12,12]", columns);
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
  check("AC-175 a page counts the user's presses, keys and scrolls and sends them as it goes: engagement is counted from them", await until(async () => (await counts()).tiles.engaged?.acting >= 1, 8000)
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
        const words = k % 4 ? `seeded message ${d} ${k}` : `seeded message ${d} ${k}: ${"make the board wait for its selector, then say what is left of the task and what a human must decide. ".repeat(4)}`;
        seed.push(`(${at}, '${k % 9 ? "human" : "dispatcher"}', 'hook', ${k % 3 ? (k % 5 ? "'claude'" : "'codex'") : "NULL"}, 'k${k % 5}', 'local', '${work}', 'session ${k % 5}', ${k % 2 ? `'T-00${k % 7}'` : "NULL"}, ${project ? `'${project}', 'Project ${project}'` : "NULL, NULL"}, '${words}')`);
      }
    }
    intoStore(app, `INSERT INTO messages (at, sender, via, agent, pane, host, cwd, title, tasks, project, project_name, text) VALUES ${seed.join(",")}`);
    await page.goto(`${base}/dashboard/`);
    await page.waitForSelector("#tiles .tile");
    await nap(500);
    await page.screenshot({ path: path.join(shots, "dashboard.png"), fullPage: true });
    const size = page.viewportSize();
    const shot = async (name, query = "") => {
      await page.goto(`${base}/history/${query}`);
      await page.waitForSelector("#results .msg");
      await nap(300);
      await page.screenshot({ path: path.join(shots, `history-${name}.png`) });
    };
    await shot("seeded", "?q=seeded");
    await page.setViewportSize({ width: 2250, height: 925 });
    await shot("wide");
    await page.click(railRow("sender", "Dispatcher"));
    await page.click('#results .msg:has-text("make the board wait for its selector")');
    await page.screenshot({ path: path.join(shots, "history-open.png") });
    // a light theme: the words in who's hue, the chips and the chosen row must still read
    await page.route("**/api/messages/meta", async (r) => { const got = await r.fetch(), meta = await got.json(); await r.fulfill({ response: got, json: { ...meta, theme: { ...meta.theme, bg: "#ffffff", fg: "#1f2328", cursor: "#0969da", dark: false } } }); }, { times: 1 });
    await shot("light", "?sender=dispatcher");
    await page.setViewportSize({ width: 1000, height: 925 });
    await shot("narrow");
    await page.setViewportSize(size);
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
  await page.goto(`${base}/history/`);
  check("AC-151 the phone's filters stand behind one key", await page.isVisible("#hfilters") && !(await page.isVisible("#fsender")));
  await page.click("#hfilters");
  const rail = await page.evaluate(() => [...document.querySelectorAll("#hrail :is(.frow, input, button)")].map((r) => Math.round(r.getBoundingClientRect().height)));
  check("AC-151 Filters shows the quick filters, each row, field and key a finger's size, with no sideways scroll", await page.isVisible("#fsender") && rail.length >= 9 && rail.every((h) => h >= 44)
    && await page.evaluate(() => document.documentElement.scrollWidth - innerWidth) <= 0, JSON.stringify(rail));
  if (shots) await page.screenshot({ path: path.join(shots, "history-phone-filters.png") });
  await page.click("#hfilters");
  await nap(500);                               // WebKit paints the rows that came back into view a frame later
  if (shots) await page.screenshot({ path: path.join(shots, "history-phone-top.png") });
}
