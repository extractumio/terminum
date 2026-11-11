// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Auto-Dispatcher against a real private terminumd (terminum-dispatcher.md AC-170): its key and window on the board,
// a round of the picking agent starting the TODO tasks on their agents with no press, its session taking no slot,
// a task's own agent, and the setting turning itself off when what it starts fails. scripts/fake_agent.py picks and
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
  const picker = await agent("Auto picker"), runner = await agent("Auto runner", "", 2), broken = await agent("Auto broken", "--start fast");
  const made = async (title, more = {}) => (await j("/api/tasks", "POST", { title, project, body: "TEXT of it", ...more })).id;
  const task = (id) => j(`/api/tasks/${id}`);
  const setting = async () => (await j("/api/tasks/meta")).projects.find((p) => p.id === project).auto;
  const [a, b] = [await made("Auto first", { severity: "P1" }), await made("Auto second")];
  await open(`?project=${project}`);

  // off until a human says: nothing starts
  check("AC-170 off, its key says so and nothing starts", await until(async () => (await page.textContent("#autokey")) === "Auto-Dispatcher: OFF") && (await task(a)).status === "TODO" && (await setting()).agent === null);
  check("AC-170 the setting takes only agents of the pool", /That agent is gone/.test((await j(`/api/task-projects/${project}/auto`, "PUT", { agent: "nobody", default: null, off_why: null })).error ?? ""));
  const other = newPane("no-round");
  await keyOf(other);
  const noRound = tool(other, "dispatch_task", { id: a, why: "x" });
  check("AC-170 only a round's own session starts tasks", /Only Auto-Dispatcher's round starts tasks/.test(noRound) && (await task(a)).status === "TODO", noRound.slice(-400));
  endPanes();

  // its window: the agent that picks, the default one
  await page.click("#autokey");
  await page.waitForSelector("#autobox[open]");
  check("AC-170 its window says what it does and offers the pool", /with no one pressing Run/.test(await page.textContent("#autotext")) && (await page.$$eval("#autoagent option", (o) => o.map((x) => x.textContent))).join().startsWith("OFF,")
    && await page.inputValue("#autoagent") === "");
  const fits = await page.evaluate(() => { const d = document.querySelector("#autobox"); return d.scrollWidth <= d.clientWidth + 1 && d.getBoundingClientRect().right <= innerWidth; });
  check("AC-170 its window fits the page, no sideways scroll", fits);
  await shot("auto-window");
  await page.selectOption("#autoagent", picker);
  await page.selectOption("#autodefault", runner);
  await page.click("#autook");
  check("AC-170 turned on, its key names the agent", await until(async () => (await page.textContent("#autokey")) === "Auto-Dispatcher: Auto picker" && await page.evaluate(() => document.querySelector("#autokey").classList.contains("on"))), await page.textContent("#autokey"));

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
  check("AC-170 off, the rest of TODO stays", ended.filter((s) => s === "FAILED").length === 2 && ended.filter((s) => s === "TODO").length === 1 && (await setting()).agent === null, ended.join());
  await open(`?project=${project}`);
  check("AC-170 its key says it turned itself off", await until(async () => (await page.textContent("#autokey")) === "Auto-Dispatcher: OFF" && await page.evaluate(() => document.querySelector("#autokey").classList.contains("stopped"))
    && /turned itself off for Autos: the last two tasks/.test(await page.getAttribute("#autokey", "title"))), await page.getAttribute("#autokey", "title"));
  const unseen = await j(`/api/task-projects/${project}/auto`, "PUT", { agent: picker, default: runner, off_why: null });
  check("AC-170 a window that did not show why it stopped does not turn it on again", /It turned itself off meanwhile: the last two tasks/.test(unseen.error ?? "") && (await setting()).agent === null, JSON.stringify(unseen));
  await page.click("#autokey");
  await page.waitForSelector("#autobox[open]");
  check("AC-170 its window says why, and Save turns it on again", /It turned itself off: the last two tasks it started failed/.test(await page.textContent("#autowhy")) && await page.inputValue("#autoagent") === "");
  await shot("auto-stopped");
  await page.click('#autobox button[value="cancel"]');

  // the sessions it started are given up, so no slot stays held for the suites after
  for (const id of [a, b]) await j(`/api/tasks/${id}/give-up`, "POST");
}
