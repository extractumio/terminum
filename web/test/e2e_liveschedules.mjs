// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The board's folded cards and its rows on the columns' tracks against a real private terminumd
// (terminum-kanban.md AC-159, terminum-dispatcher.md AC-138), and schedules (terminum-schedules.md AC-160 to
// AC-164): `node web/test/e2e_live.mjs schedules chromium`.
import { check, until } from "./check.mjs";
import { boardKit, intoStore, nap, startOf } from "./boardkit.mjs";

export async function liveFolded(page, live) {
  const { j, card, open } = boardKit(page, live);
  const shot = (name) => live.shots && page.screenshot({ path: `${live.shots}${live.shots.endsWith("-") ? "" : "/"}${name}.png` });
  const made = async (title, severity = "P2") => (await j("/api/tasks", "POST", { title, severity, body: "**text** of it" })).id;
  const a = await made("Folded first task with a title long enough to be cut at the line's end", "P1"), b = await made("Folded second");
  // three agents and one more: a row of four on the columns' tracks, the fifth starts the next
  const ids = [];
  for (const n of ["Grid one", "Grid two", "Grid three", "Grid four", "Grid five"]) ids.push((await j("/api/task-agents", "POST", { name: n, kind: "claude", command: "claude" })).id);
  await open("?project=default");                  // its tasks' project: one an earlier suite chose is remembered
  const box = (sel) => page.evaluate((s) => { const r = document.querySelector(s)?.getBoundingClientRect(); return r && { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) }; }, sel);
  const shown = (sel) => page.evaluate((s) => { const e = document.querySelector(s); return !!e && getComputedStyle(e).display !== "none" && e.getClientRects().length > 0; }, sel);

  // AC-159: a line a card in TODO, its ID, severity and title only
  const lineH = (await box(card(a))).h;
  // one line: ID, severity and title on one row, the title cut, nothing under them
  const row = await page.evaluate((s) => {
    const mid = (q) => { const r = document.querySelector(`${s} ${q}`).getBoundingClientRect(); return r.top + r.height / 2; };
    const t = document.querySelector(`${s} .ctitle`);
    return { spread: Math.max(mid(".cid"), mid(".csev"), mid(".ctitle")) - Math.min(mid(".cid"), mid(".csev"), mid(".ctitle")), cut: getComputedStyle(t).whiteSpace === "nowrap" && t.scrollWidth > t.clientWidth };
  }, card(a));
  check("AC-159 a TODO card is one line: ID, severity, title", row.spread < 4 && row.cut && !(await shown(`${card(a)} .cfoot`)), JSON.stringify(row));
  check("AC-159 it says collapsed", await page.getAttribute(card(a), "aria-expanded") === "false");
  await shot("folded");
  await page.click(`${card(a)} .ctitle`);
  await nap(150);
  check("AC-159 a press shows it whole, no window", (await box(card(a))).h > lineH && await shown(`${card(a)} .cfoot`) && !(await page.isVisible("#taskbox[open]")) && await page.getAttribute(card(a), "aria-expanded") === "true");
  await shot("unfolded");
  await page.click(`${card(b)} .ctitle`);
  await nap(150);
  check("AC-159 another's press folds the first", (await box(card(a))).h === lineH && await shown(`${card(b)} .cfoot`));
  await page.click(`${card(b)} .ctitle`);
  check("AC-159 a press on the whole card opens its window", await until(() => page.isVisible("#taskbox[open]")));
  await page.keyboard.press("Escape");
  await until(async () => !(await page.isVisible("#taskbox[open]")));
  await page.keyboard.press("Escape");
  await nap(100);
  check("AC-159 Esc folds it", (await box(card(b))).h === lineH, String((await box(card(b))).h));
  await j(`/api/tasks/${a}`, "PATCH", { title: "Folded first, renamed" });
  await page.click(`${card(a)} .ctitle`);
  await nap(100);
  await j(`/api/tasks/${a}`, "PATCH", { title: "Folded first, renamed again" });
  check("AC-159 shown whole stays so as the columns are read again", await until(async () => (await page.textContent(`${card(a)} .ctitle`)) === "Folded first, renamed again") && await shown(`${card(a)} .cfoot`));

  // AC-138: an agent a column wide, four a row, + Agent in the fifth track
  const cols = await page.evaluate(() => [...document.querySelectorAll("#board .col")].map((c) => { const r = c.getBoundingClientRect(); return [Math.round(r.x), Math.round(r.width)]; }));
  const agents = await page.evaluate(() => [...document.querySelectorAll(".agents .agentcard")].map((c) => { const r = c.getBoundingClientRect(); return [Math.round(r.x), Math.round(r.width), Math.round(r.y)]; }));
  const add = await box("#addagent");
  const wide = page.viewportSize().width >= 860;
  if (wide) {
    const aligned = agents.slice(0, 4).every(([x, w], i) => Math.abs(x - cols[i][0]) <= 1 && Math.abs(w - cols[i][1]) <= 1);
    check("AC-138 agents stand a column wide on the columns' tracks, four a row", aligned && agents[4][2] > agents[0][2] && Math.abs(agents[4][0] - cols[0][0]) <= 1, JSON.stringify({ cols, agents }));
    check("AC-138 + Agent stands in the fifth track", add.x >= cols[4][0] - 1 && add.x < cols[4][0] + cols[4][1], JSON.stringify(add));
  } else {
    check("AC-138 a phone keeps one row of agents that scrolls", agents.every(([, , y]) => y === agents[0][2]), JSON.stringify(agents));
  }
  await shot("agents");
  for (const id of ids) await j(`/api/task-agents/${id}`, "DELETE");
  for (const id of [a, b]) await j(`/api/tasks/${id}/archive`, "POST", { on: true });
}

const FAKE = new URL("../../scripts/fake_agent.py", import.meta.url).pathname;

/** AC-160 to AC-164: a schedule made in its window, kept on its agent, fired on a fake agent, its task's result on
 *  the board and its firing on the Scheduled page. Its due time is reached by moving its last due back an hour in
 *  this test's own store: of a cadence of every minute the last due time is then under a minute past, in its time. */
export async function liveSchedules(page, live) {
  const { j, card, open, cli } = boardKit(page, live);
  const shot = (name) => live.shots && page.screenshot({ path: `${live.shots}${live.shots.endsWith("-") ? "" : "/"}${name}.png` });
  const project = (await j("/api/task-projects", "POST", { name: "Nightly", dir: live.work })).id;
  const agent = (await j("/api/task-agents", "POST", { name: "Night owl", kind: "claude", command: `python3 -I ${FAKE} ${cli} --kind claude`, at_once: 2 })).id;
  await open(`?project=${project}`);
  const toggle = "#schedbar .schedtoggle";
  check("AC-163 the section is one line, folded, by default", await page.textContent(toggle) === "Scheduled 0" && await page.getAttribute(toggle, "aria-expanded") === "false" && !(await page.isVisible("#schedgrid")));

  // AC-160: the window, its picker and terminumd's words
  await page.click("#newschedule");
  await page.waitForSelector("#schedbox[open]");
  await page.fill("#schedtitle", "Nightly deps check");
  await page.fill("#schedtext", "FINISHES the check of dependencies.");
  // a choice that says "change" alone is heard too
  const choose = (id) => page.$eval("#schedagent", (s, id) => { s.value = id; s.dispatchEvent(new Event("change", { bubbles: true })); }, id);
  await choose("");
  check("AC-160 no agent chosen is said", await until(async () => await page.textContent("#schederror") === "Choose the agent that runs it."), await page.textContent("#schederror"));
  await choose(agent);
  check("AC-160 an agent chosen, its words go", await until(async () => (await page.textContent("#schedline")).includes("fake_agent.py") && !(await page.isVisible("#schederror"))), await page.textContent("#schederror"));
  await page.selectOption("#schedkind", "minutes");
  await page.selectOption("#schedevery", "15");
  check("AC-160 the picker writes cron fields and terminumd says the next runs", await until(async () => await page.textContent("#schedfields") === "*/15 * * * *" && /^Every 15 minutes\. Next: .+, .+, .+/.test(await page.textContent("#schednext"))), await page.textContent("#schednext"));
  check("AC-160 what the agent gets stands open before Save", await page.evaluate(() => document.getElementById("schedwhat").open));
  check("AC-160 what the agent gets: the prompt and its command", (await page.textContent("#schedprompt")).includes("A scheduled run of terminum's schedule SCH-? \"Nightly deps check\" (every 15 minutes)") && (await page.textContent("#schedline")).includes("fake_agent.py"), await page.textContent("#schedprompt"));
  await page.selectOption("#schedkind", "cron");
  await page.fill("#schedcron", "0 24 * * *");
  check("AC-160 a field out of its range is refused in the window", await until(async () => await page.textContent("#schederror") === "The hour \"24\" is not 0 to 23."), await page.textContent("#schederror"));
  await page.selectOption("#schedkind", "hours");
  await page.selectOption("#schedevery", "1");
  await page.selectOption("#schedkind", "minutes");
  check("AC-160 another kind's number is not carried over: 1 hour is not every minute", await page.inputValue("#schedevery") === "20", await page.inputValue("#schedevery"));
  await page.selectOption("#schedkind", "weekly");
  await page.click('#whendays button:has-text("Fri")');
  check("AC-160 weekly days are keys", await until(async () => (await page.textContent("#schedfields")).endsWith("* * 1,5")), await page.textContent("#schedfields"));
  // its window on one grid, the weekly kind's parts shown: every label and field on the form's edge, the prompt a field (its well, its text where a
  // field's is), When's fields as tall as the others, the tick on its field's middle
  const grid = await page.evaluate(() => {
    const f = document.querySelector("#schedbox form").getBoundingClientRect(), at = (id) => document.getElementById(id).getBoundingClientRect();
    const textX = (id) => Math.round(at(id).x - f.x + parseFloat(getComputedStyle(document.getElementById(id)).paddingLeft));
    const edges = ["schedtitle", "schedtext", "schedproj", "schedwhen", "schedkind", "whendays", "schedbudget", "schedwhat", "schedprompt"].map((id) => Math.round(at(id).x - f.x));
    const mid = (r) => r.y + r.height / 2;
    return { edges, line: Math.round(document.querySelector("#schedwhat .runfacts").getBoundingClientRect().x - f.x), text: [textX("schedtitle"), textX("schedprompt")],
      well: getComputedStyle(document.getElementById("schedprompt")).boxShadow !== "none",
      heights: ["schedtitle", "schedkind", "schedtime"].map((id) => Math.round(at(id).height)), tick: Math.round(mid(at("schedpaused")) - mid(at("schedbudget"))) };
  });
  check("AC-160 the schedule window's parts stand on one edge, the prompt a field with a field's text inset",
    grid.edges.every((x) => x === 0) && grid.line === 0 && grid.text[0] === grid.text[1] && grid.well, JSON.stringify(grid));
  check("AC-160 When's fields are as tall as the others; Paused stands on its field's middle",
    grid.heights.every((h) => Math.abs(h - grid.heights[0]) <= 1) && Math.abs(grid.tick) <= 1, JSON.stringify(grid));
  await page.selectOption("#schedkind", "minutes");
  await page.selectOption("#schedevery", "1");
  check("AC-160 runs a minute apart are taken", await until(async () => await page.textContent("#schedfields") === "*/1 * * * *" && /^Every minute\. Next: /.test(await page.textContent("#schednext"))), await page.textContent("#schednext"));
  await page.fill("#schedbudget", "");
  await page.click("#schedok");
  check("AC-160 no budget is refused, the window stays", await until(async () => await page.textContent("#schederror") === "A scheduled run needs a time budget: no human watches it.") && await page.isVisible("#schedbox[open]"));
  await page.fill("#schedbudget", "20");
  await shot("schedule-window");
  await page.click("#schedok");
  check("AC-160 Save keeps it and closes the window", await until(async () => !(await page.isVisible("#schedbox[open]"))));
  check("AC-163 the head counts it", await until(async () => await page.textContent(toggle) === "Scheduled 1"));
  await page.click(toggle);
  const sched = "#schedgrid .schedcard";
  check("AC-163 open, its card: key, title, cadence, next run, agent", await until(async () => {
    const t = await page.textContent(sched).catch(() => "");
    return /SCH-\d+/.test(t) && t.includes("Nightly deps check") && t.includes("Every minute") && /next /.test(t) && t.includes("Night owl");
  }), await page.textContent(sched).catch(() => ""));
  check("AC-163 its card has no colored stripe along its side: the agent's dot and name say whose it is", await page.$eval(sched, (c) => !/inset/.test(getComputedStyle(c).boxShadow) && getComputedStyle(c.querySelector(".cagent"), "::before").width === "7px"),
    await page.$eval(sched, (c) => getComputedStyle(c).boxShadow));
  const cols =await page.evaluate(() => [...document.querySelectorAll("#board .col")].map((c) => Math.round(c.getBoundingClientRect().width)));
  const w = await page.evaluate((s) => Math.round(document.querySelector(s).getBoundingClientRect().width), sched);
  if (page.viewportSize().width >= 860) check("AC-163 a card a column wide", Math.abs(w - cols[0]) <= 1, `${w} ${cols}`);
  const owl = `#agentbar .agentcard[data-agent="${agent}"]`;
  check("AC-161 its agent shows the kept slot", await until(async () => (await page.$$(`${owl} .agkept`)).length === 1 && /1\/2/.test(await page.textContent(owl))), await page.textContent(owl));
  // more agents than a few rows: the bars scroll, the columns stay
  const crowd = [];
  for (let i = 0; i < 14; i++) crowd.push((await j("/api/task-agents", "POST", { name: `Crowd ${i}`, kind: "claude", command: "claude" })).id);
  await page.reload();
  await page.waitForSelector("#board .col", { state: "attached" });
  check("AC-163 this browser remembers it open", await until(() => page.isVisible(sched)));
  const room = await page.evaluate(() => Math.round(document.querySelector("#board .col").getBoundingClientRect().height));
  check("AC-163 the columns keep their room under the pool and the section, however many agents", room > 200, String(room));
  await shot("schedules");
  for (const id of crowd) await j(`/api/task-agents/${id}`, "DELETE");
  // columns far longer than the window take no room from what stands over them: the pool and the section are as
  // tall as over empty columns, where every card is whole unless there are more than their own bars hold
  const bars = async () => {
    await page.reload();
    await page.waitForSelector(sched);
    // read once they hold still: a card drawn after the load (a session's dot, a reading) changes a bar's height
    const read = () => page.evaluate(() => ["#agentbar", "#schedbar"].map((bar) => Math.round(document.querySelector(bar).getBoundingClientRect().height)));
    let last = await read();
    await until(async () => { const now = await read(), same = now.join() === last.join(); last = now; return same; });
    return last;
  };
  const over = await bars(), many = [];
  for (let i = 0; i < 40; i++) many.push((await j("/api/tasks", "POST", { title: `Long column ${i}`, project })).id);
  const overLong = await bars();
  check("AC-163 long columns take no room from the agents and the schedules over them", overLong.every((h, i) => h >= over[i] - 1) && overLong[1] > 60, JSON.stringify({ over, overLong }));
  await shot("schedules-long");
  for (const id of many) await j(`/api/tasks/${id}/archive`, "POST", { on: true });

  // AC-162: due, fired on the fake agent, its task DONE with its result
  const s = (await j("/api/schedules")).schedules.find((x) => x.title === "Nightly deps check");
  intoStore(live.app, `UPDATE schedules SET last_due = last_due - 3600 WHERE id = '${s.id}'`);
  let task;
  check("AC-162 due, it makes a task and runs it on its agent", await until(async () => (task = (await j("/api/schedules")).schedules.find((x) => x.id === s.id)?.last?.task), 15000), JSON.stringify(s));
  check("AC-183 a firing's run keeps Dispatcher and the schedule", await until(() => startOf(live.app, task) === "dispatcher schedule", 15000), startOf(live.app, task));
  intoStore(live.app, `UPDATE schedules SET cron = '0 0 1 1 *' WHERE id = '${s.id}'`);   // no second firing a minute on, under the checks below
  check("AC-162 its task says a schedule made it, and was never in TODO", /^S_[A-Z][A-Z0-9]*-\d+$/.test(task) && !(await j(`/api/tasks/${task}`)).history.some((e) => /TODO/.test(e.text ?? "")), task);
  check("AC-162 the agent gives its result: DONE", await until(async () => (await j(`/api/tasks/${task}`)).status === "DONE", 20000), JSON.stringify((await j(`/api/tasks/${task}`)).history?.map((e) => e.text)));
  const t = await j(`/api/tasks/${task}`);
  check("AC-162 its history says the schedule and its due time", t.history.some((e) => /^Dispatcher made it from schedule SCH-\d+, due \w{3} \d+ \w{3} \d\d:\d\d\.$/.test(e.text ?? "")), JSON.stringify(t.history.map((e) => e.text)));
  check("AC-163 its window names the schedule", t.schedule?.key === s.key);
  await page.goto(`${live.base}/kanban/${task}`);
  check("AC-163 the window's line opens the schedule", await until(() => page.isVisible(".taskfrom")));
  await page.click(".taskfrom .linklike");
  check("AC-163 …in its own window", await until(() => page.isVisible("#schedbox[open]")) && (await page.textContent("#schedhead")).startsWith(s.key));
  await page.keyboard.press("Escape");
  await open(`?project=${project}`);
  check("AC-163 its card says the schedule; the schedule's card its last task", await until(async () => (await page.textContent(`${card(task)} .cfrom`).catch(() => "")) === s.key && (await page.textContent(`${sched} .stbox`).catch(() => "")) === task));

  // AC-164: the Scheduled page
  await page.goto(`${live.base}/scheduled/?schedule=${s.key}`);
  check("AC-164 the Scheduled page lists the firing with its task and status", await until(async () => /DONE/.test(await page.textContent("#fires").catch(() => "")) && (await page.textContent("#fires")).includes(task)), await page.textContent("#fires").catch(() => ""));
  await page.click("#fires .fireline");
  check("AC-164 a row opens to what went in and came out", await until(async () => { const b = await page.textContent("#fires .firebody"); return b.includes("A scheduled run of terminum's schedule") && b.includes("All done."); }), await page.textContent("#fires .firebody"));
  check("AC-164 the head has its key pressed", await page.getAttribute('.mpages a[href="/scheduled/"]', "aria-current") === "page");
  await shot("scheduled-page");

  // AC-161: an agent running schedules is not deleted; a schedule deleted leaves its tasks
  const kept = await j(`/api/task-agents/${agent}`, "DELETE");
  check("AC-161 an agent with schedules is not deleted", kept.code === 400 && /runs schedules SCH-\d+: give them another agent first\./.test(kept.error ?? ""), JSON.stringify(kept));
  check("AC-160 deleted, its tasks stay", (await j(`/api/schedules/${s.id}`, "DELETE")).code === 200 && (await j(`/api/tasks/${task}`)).status === "DONE");
}
