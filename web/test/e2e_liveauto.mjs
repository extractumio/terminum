// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Auto-Dispatch against a real private terminumd (terminum-dispatcher.md AC-170): its switch and cog in the board's
// bar and its window, a round of the picking agent starting the TODO tasks on their agents with no press, its session
// taking no slot, the severities and the wait it goes by, a task's own agent, the setting turning itself off when
// what it starts fails, and the bar with several projects shown and at a phone's size. scripts/fake_agent.py picks and
// works by the real hooks and tools: `node web/test/e2e_live.mjs auto chromium`.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { boardKit } from "./boardkit.mjs";
import { check, until } from "./check.mjs";

const FAKE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../scripts/fake_agent.py");

export async function liveAuto(page, live) {
  const { j, card, open, cli, newPane, keyOf, tool, endPanes } = boardKit(page, live);
  const shot = (name) => live.shots && page.screenshot({ path: path.join(live.shots, `${name}.png`) });
  const project = (await j("/api/task-projects", "POST", { name: "Autos", dir: live.work })).id;
  const agent = async (name, more = "", at_once = 1) => (await j("/api/task-agents", "POST", { name, kind: "claude", command: `python3 -I ${FAKE} ${cli} --kind claude ${more}`.trim(), at_once })).id;
  const picker = await agent("Auto picker"), runner = await agent("Auto runner", "", 2), broken = await agent("Auto broken", "--start fast"), spare = await agent("Auto spare");
  const put = (body) => j(`/api/task-projects/${project}/auto`, "PUT", body);
  const state = () => page.evaluate(() => `${document.querySelector("#autoswitch").getAttribute("aria-checked")} ${document.querySelector("#autostate").textContent}`);
  const made = async (title, more = {}) => (await j("/api/tasks", "POST", { title, project, body: "TEXT of it", ...more })).id;
  const task = (id) => j(`/api/tasks/${id}`);
  const setting = async () => (await j("/api/tasks/meta")).projects.find((p) => p.id === project).auto;
  const [a, b] = [await made("Auto first", { severity: "P1" }), await made("Auto second")];
  await open(`?project=${project}`);

  // off until a human says: nothing starts
  const made0 = await setting();
  check("AC-170 off, its switch says so and nothing starts", await until(async () => (await state()) === "false OFF") && (await task(a)).status === "TODO" && made0.on === false && made0.agent === null);
  check("AC-170 a project is made taking P0 and P1 after 10 minutes", made0.severities.join() === "P0,P1" && made0.wait_minutes === 10, JSON.stringify(made0));
  check("AC-170 the setting takes only agents of the pool", /That agent is gone/.test((await put({ agent: "nobody", default: null, off_why: null })).error ?? ""));
  check("AC-170 on needs the agent that picks", /Choose the agent that picks first/.test((await put({ on: true, off_why: null })).error ?? ""));
  check("AC-170 the project's window holds no key of it", await page.locator("#projectauto").count() === 0);
  const other = newPane("no-round");
  await keyOf(other);
  const noRound = tool(other, "dispatch_task", { id: a, why: "x" });
  check("AC-170 only a round's own session starts tasks", /Only Auto-Dispatcher's round starts tasks/.test(noRound) && (await task(a)).status === "TODO", noRound.slice(-400));
  endPanes();

  // its switch, with no agent chosen yet, opens its window with On ticked: the agent that picks, the default one, what it takes
  await page.click("#autoswitch");
  await page.waitForSelector("#autobox[open]");
  const rules = await page.textContent("#autorules");
  check("AC-170 its window says what it takes, in what order and when it stops, and offers the pool", /with no one pressing Run/.test(await page.textContent("#autotext")) && /What it takes/.test(rules) && /Down the board/.test(rules) && /It turns itself off/.test(rules)
    && (await page.$$eval("#autoagent option", (o) => o.map((x) => x.textContent))).join().startsWith("Choose one,") && await page.inputValue("#autoagent") === "" && await page.isChecked("#autoon") && await page.locator("#autoprojectrow").isHidden());
  check("AC-170 its window shows the tasks of 24 hours and how many are started", await page.inputValue("#automost") === "10" && /Started in the last 24 hours: 0\./.test(await page.textContent("#autoday")));
  check("AC-170 its window shows the severities ticked and the wait", (await page.$$eval("#autoseverities input", (l) => l.map((t) => `${t.value}${t.checked ? "+" : "-"}`))).join() === "P0+,P1+,P2-,P3-" && await page.inputValue("#autowait") === "10");
  await page.click("#autook");
  check("AC-170 a Save with On and no agent is refused in the window", await until(async () => /Choose the agent that picks first/.test(await page.textContent("#autoerror"))));
  for (const p of ["P2", "P3"]) await page.check(`#autoseverities input[value="${p}"]`);
  await page.fill("#autowait", "0");
  const fits = await page.evaluate(() => { const d = document.querySelector("#autobox"); return d.scrollWidth <= d.clientWidth + 1 && d.getBoundingClientRect().right <= innerWidth; });
  check("AC-170 its window fits the page, no sideways scroll", fits);
  await shot("auto-window");
  await page.selectOption("#autoagent", picker);
  await page.selectOption("#autodefault", runner);
  await page.click("#autook");
  check("AC-170 turned on, its switch says so and names the agent", await until(async () => (await state()) === "true ON") && /Auto picker picks Autos's TODO tasks/.test(await page.getAttribute("#autoswitch", "title")), await state());

  // a round starts both on the default agent, no press
  const said = async (id, words) => (await task(id)).history.find((h) => words.test(h.text ?? ""));
  const started = async (id) => (await said(id, /^Auto-Dispatcher's round \S+ started it on /))?.text;
  check("AC-170 its round starts the TODO tasks for the free slots", await until(async () => (await task(a)).status === "IN PROGRESS" && (await task(b)).status === "IN PROGRESS", 30000), `${(await task(a)).status} ${(await task(b)).status}: ${JSON.stringify(await setting())}`);
  const line = await started(a), why = await said(a, /^Why the round chose it: RICE 3×2×1÷1 = 6/);
  check("AC-170 a started task says which round and on which agent, in Dispatcher's words", /started it on Auto runner\.$/.test(line ?? ""), line);
  check("AC-170 and the round's reason, as its agent's words", why?.author_name === "Auto picker", JSON.stringify(why));
  check("AC-170 it runs on the default agent, its card saying so", await until(async () => (await page.textContent(`${card(a)} .cagent`).catch(() => "")) === "Auto runner"));
  const round = /round (\S+) started/.exec(line ?? "")?.[1];
  const roundTask = await task(round);
  check("AC-170 a round is Dispatcher's task with its candidates, its result in the short form", /^Auto-dispatch: \d free, 2 candidates$/.test(roundTask.title) && roundTask.text.includes(a) && roundTask.text.includes(b)
    && roundTask.history.some((h) => /Dispatcher made it/.test(h.text ?? "")), JSON.stringify({ title: roundTask.title, history: roundTask.history.map((h) => h.text) }).slice(0, 1500));
  check("AC-170 ended with its result, a round leaves the board for Archived", await until(async () => !!(await task(round)).archived_at, 20000) && (await task(round)).status === "DONE", JSON.stringify({ status: (await task(round)).status, history: (await task(round)).history.map((h) => h.text) }).slice(0, 1800));
  // its session: shown on its agent as a round, taking no slot
  const pick = `#agentbar .agentcard[data-agent="${picker}"]`;
  const dots = await page.$$eval(`${pick} .agdot`, (l) => l.map((d) => `${d.className}|${d.title}`));
  if (dots.length) check("AC-170 a round's session is a dot of its own look that takes no slot", dots.every((d) => /round/.test(d) && /takes no slot/.test(d)) && await page.locator(`${pick} .agfree`).count() === 1, JSON.stringify(dots));
  const slots = (await j(`/api/tasks?project=${project}&limit=1&counts=1`)).slots;
  check("AC-170 the picking agent is not at its limit by its round", !slots.full.includes(picker) && slots.full.includes(runner), JSON.stringify(slots.full));
  await shot("auto-on");

  // one press turns it off and on again, its agents kept; off, nothing starts
  await page.click("#autoswitch");
  check("AC-170 a press of the switch turns it off, its agents kept", await until(async () => (await state()) === "false OFF") && (await setting()).agent === picker && (await setting()).default === runner);
  const idle = await made("Auto idle: made while it is off", { severity: "P0", agent: spare });
  await page.click("#autoswitch");
  check("AC-170 a press turns it on again, no window", await until(async () => (await state()) === "true ON") && !(await page.locator("#autobox").evaluate((d) => d.open)));
  check("AC-170 on again, it takes what stood in TODO", await until(async () => (await task(idle)).status === "IN PROGRESS", 30000), (await task(idle)).status);
  await j(`/api/tasks/${idle}/give-up`, "POST");
  // what it takes: the severities ticked, after the wait (10 minutes: 2 s at this clock)
  check("AC-170 the severities and the wait are kept", (await put({ severities: ["P0"], wait_minutes: 10, off_why: null })).ok === true && (await setting()).severities.join() === "P0" && (await setting()).wait_minutes === 10);
  check("AC-170 the tasks of 24 hours are kept, 1 to 1000", made0.day_most === 10 && /Tasks a day: 1 to 1000/.test((await put({ day_most: 0, off_why: null })).error ?? "") && (await put({ day_most: 500, off_why: null })).ok === true && (await setting()).day_most === 500);
  check("AC-170 the setting says how many it started in 24 hours", (await setting()).started_day >= 3, JSON.stringify(await setting()));
  check("AC-170 a wait past a day is refused", /Wait: 0 to 1440 minutes/.test((await put({ wait_minutes: 5000, off_why: null })).error ?? ""));
  const [low, high] = [await made("Auto low: not taken", { severity: "P1", agent: spare }), await made("Auto high: taken after the wait", { severity: "P0", agent: spare })];
  check("AC-170 a task of a severity ticked starts", await until(async () => (await task(high)).status === "IN PROGRESS", 30000), (await task(high)).status);
  const taken = await task(high), startedAt = taken.history.find((h) => /started it on Auto spare/.test(h.text ?? ""))?.at;
  check("AC-170 not before its wait passed", startedAt - taken.created_at >= 2, `${startedAt} ${taken.created_at}`);
  check("AC-170 one of a severity not ticked stays in TODO", (await task(low)).status === "TODO");
  await j(`/api/tasks/${high}/give-up`, "POST");
  await j(`/api/tasks/${low}/archive`, "POST", {});
  // the tasks of its 24 hours started: it waits, said on its switch, and goes on when the cap is raised
  const startedDay = (await setting()).started_day;
  await put({ day_most: startedDay, off_why: null });
  const capped = await made("Auto capped: waits for the day's cap", { severity: "P0", agent: spare });
  await new Promise((r) => setTimeout(r, 6000));                // its wait (2 s at this clock) and several walks
  check("AC-170 at the cap of its 24 hours nothing more starts", (await task(capped)).status === "TODO" && (await setting()).started_day === startedDay, `${(await task(capped)).status} ${JSON.stringify(await setting())}`);
  await open(`?project=${project}`);
  check("AC-170 its switch says it waits for the cap", /tasks of 24 hours are started: it waits/.test(await page.getAttribute("#autoswitch", "title")), await page.getAttribute("#autoswitch", "title"));
  await put({ day_most: 500, off_why: null });
  check("AC-170 the cap raised, it goes on", await until(async () => (await task(capped)).status === "IN PROGRESS", 30000), (await task(capped)).status);
  await j(`/api/tasks/${capped}/give-up`, "POST");
  await put({ severities: ["P0", "P1", "P2", "P3"], wait_minutes: 0, off_why: null });

  // the other way of picking: by the board's order, terminumd itself, no agent and no round
  await page.click("#autocog");
  await page.waitForSelector("#autobox[open]");
  check("AC-170 its window offers the two ways of picking, an agent's first", (await page.$$eval("#automode button", (l) => l.map((b) => `${b.value}:${b.getAttribute("aria-pressed")}`))).join() === "agent:true,order:false" && await page.locator("#autoagentrow").isVisible());
  await page.click('#automode button[value="order"]');
  check("AC-170 by the board's order no agent is chosen, and its words say what that means", await page.locator("#autoagentrow").isHidden() && await page.locator("#autorules .byorder").isVisible() && await page.locator("#autorules .byagent").isHidden()
    && /No agent reads the tasks/.test(await page.textContent("#autorules .byorder")));
  await shot("auto-order");
  await page.click("#autook");
  check("AC-170 the way of picking is kept, said on the switch", await until(async () => (await setting()).mode === "order" && /Autos's TODO tasks start by the board's order/.test(await page.getAttribute("#autoswitch", "title"))), await page.getAttribute("#autoswitch", "title"));
  check("AC-170 another way is refused", /mode: agent or order/.test((await put({ mode: "dice", off_why: null })).error ?? ""));
  const [later, sooner] = [await made("Auto later: P1, by order", { severity: "P1", agent: spare }), await made("Auto sooner: P0, by order", { severity: "P0", agent: spare })];
  check("AC-170 by the board's order the P0 starts first on its agent's one slot", await until(async () => (await task(sooner)).status === "IN PROGRESS", 30000) && (await task(later)).status === "TODO", `${(await task(sooner)).status} ${(await task(later)).status}`);
  const byOrder = (await task(sooner)).history.map((h) => h.text ?? "");
  check("AC-170 it says so in Dispatcher's words, and no round was made for it", byOrder.some((t) => /^Auto-Dispatcher started it on Auto spare, by the board's order\.$/.test(t)) && !byOrder.some((t) => /round/.test(t)), JSON.stringify(byOrder));
  await j(`/api/tasks/${sooner}/give-up`, "POST");
  check("AC-170 its slot free again, the next in order starts", await until(async () => (await task(later)).status === "IN PROGRESS", 30000), (await task(later)).status);
  await j(`/api/tasks/${later}/give-up`, "POST");
  check("AC-170 back to an agent's picking", (await put({ mode: "agent", off_why: null })).ok === true && (await setting()).mode === "agent");

  // a task's own agent: chosen by a human in its window
  const c = await made("Auto third: names its agent");
  check("AC-170 a new task names no agent", (await task(c)).runs_on === null);
  check("AC-170 an agent's tools do not set it: only a human's route", /That agent is gone/.test((await j(`/api/tasks/${c}/agent`, "PUT", { agent: "nobody" })).error ?? ""));
  await page.goto(`${live.base}/kanban/${c}?project=${project}`);
  await page.waitForSelector("#taskbox[open]");
  const field = page.locator("#taskbox label.fieldrow", { hasText: /^Agent/ }).locator("select");   // its own label: a session's title may say Agent too
  await field.selectOption(broken);
  check("AC-170 its window's Agent names the one that runs it", await until(async () => (await task(c)).runs_on === broken) && (await task(c)).history.some((h) => /Its agent: Auto broken/.test(h.text ?? "")));
  await page.keyboard.press("Escape");

  // what it starts fails twice: it turns itself off, said on the board; nothing more is touched
  const d = await made("Auto fourth: on the broken one", { agent: broken });
  check("AC-170 a task made with its agent keeps it", (await task(d)).runs_on === broken);
  const left = await made("Auto fifth: stays where it is", { agent: broken });
  check("AC-170 two tasks it started failing turn it off, saying why", await until(async () => /the last two tasks it started failed/.test((await setting()).off_why ?? ""), 60000), JSON.stringify(await setting()));
  const ended = [await task(c), await task(d), await task(left)].map((t) => t.status);
  check("AC-170 off, the rest of TODO stays, its agents still chosen", ended.filter((s) => s === "FAILED").length === 2 && ended.filter((s) => s === "TODO").length === 1 && (await setting()).on === false && (await setting()).agent === picker, ended.join());
  await open(`?project=${project}`);
  check("AC-170 its switch says it turned itself off", await until(async () => (await state()) === "false OFF" && await page.evaluate(() => document.querySelector("#autoswitch").classList.contains("stopped"))
    && /turned itself off for Autos: the last two tasks/.test(await page.getAttribute("#autoswitch", "title"))), await page.getAttribute("#autoswitch", "title"));
  const unseen = await put({ on: true, off_why: null });
  check("AC-170 a page that did not show why it stopped does not turn it on again", /It turned itself off meanwhile: the last two tasks/.test(unseen.error ?? "") && (await setting()).on === false, JSON.stringify(unseen));
  await page.click("#autoswitch");
  await page.waitForSelector("#autobox[open]");
  check("AC-170 a press of the switch then opens the window, which says why: it is not on again unread", /It turned itself off: the last two tasks it started failed/.test(await page.textContent("#autowhy")) && await page.inputValue("#autoagent") === picker && await page.isChecked("#autoon") && (await setting()).on === false);
  await page.fill("#autowait", "");
  await page.click("#autook");
  check("AC-170 an empty wait is refused, never taken as none", await until(async () => /Wait: 0 to 1440 minutes/.test(await page.textContent("#autoerror"))) && (await setting()).on === false);
  await shot("auto-stopped");
  await page.click('#autobox button[value="cancel"]');
  await j(`/api/tasks/${left}/archive`, "POST", {});                // nothing left for it to start

  // several projects shown: how many it is on for; a press turns those off, and turning on is a project's own, in the window
  const offWhy = (await setting()).off_why;
  check("AC-170 a page that showed why turns it on again", (await put({ on: true, off_why: offWhy })).ok === true && (await setting()).on === true && (await setting()).off_why === null);
  const every = (await j("/api/tasks/meta")).projects.map((p) => p.id), count = every.length;
  await open(`?project=${every.join(",")}`);                    // named: this browser remembers the project it showed last (AC-135)
  check("AC-170 with several projects its switch says for how many it is on", count > 1 && await until(async () => (await state()) === `true 1 of ${count}`) && /On for Autos\. Press to turn it off for them/.test(await page.getAttribute("#autoswitch", "title")), await state());
  await page.click("#autoswitch");
  check("AC-170 a press turns it off for those it is on for", await until(async () => (await state()) === `false 0 of ${count}`) && (await setting()).on === false, await state());
  await page.click("#autoswitch");
  await page.waitForSelector("#autobox[open]");
  check("AC-170 with none on, a press opens the window with its project to choose: no press turns several on", await page.locator("#autoprojectrow").isVisible() && (await page.$$eval("#autoproject option", (o) => o.length)) === count && (await setting()).on === false);
  await page.selectOption("#autoproject", project);
  check("AC-170 the window shows the chosen project's setting", await page.inputValue("#autoagent") === picker && await page.inputValue("#autodefault") === runner);
  await page.click('#autobox button[value="cancel"]');

  // a phone's size: the switch and the cog stay in the bar, the window fits
  const size = page.viewportSize();
  await page.setViewportSize({ width: 390, height: 844 });
  const phone = await page.evaluate(() => { const r = (id) => document.querySelector(id).getBoundingClientRect(), [k, c] = [r("#autoswitch"), r("#autocog")]; return { in: k.left >= 0 && c.right <= innerWidth && k.width > 0 && c.width > 0, tall: Math.min(k.height, c.height), wide: document.documentElement.scrollWidth <= innerWidth }; });
  check("AC-170 at a phone's size its switch and cog are in the bar, a finger's height, no sideways scroll", phone.in && phone.tall >= 40 && phone.wide, JSON.stringify(phone));
  await page.click("#autocog");
  await page.waitForSelector("#autobox[open]");
  const fitsPhone = await page.evaluate(() => { const d = document.querySelector("#autobox"), r = d.getBoundingClientRect(); return { wide: d.scrollWidth <= d.clientWidth + 1, left: r.left, right: r.right, bottom: r.bottom, w: innerWidth, h: innerHeight }; });
  check("AC-170 at a phone's size its window fits", fitsPhone.wide && fitsPhone.left >= 0 && fitsPhone.right <= fitsPhone.w && fitsPhone.bottom <= fitsPhone.h + 1, JSON.stringify(fitsPhone));
  await shot("auto-phone");
  await page.click('#autobox button[value="cancel"]');
  await page.setViewportSize(size);

  // the sessions it started are given up, so no slot stays held for the suites after
  for (const id of [a, b]) await j(`/api/tasks/${id}/give-up`, "POST");
}
