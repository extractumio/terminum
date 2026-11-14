// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A main task run with its subtasks, against a real private terminumd (terminum-dispatcher.md AC-185): RUN's window
// names them all, one session does them, a subtask that cannot be done fails with its main task, and the main task's
// next RUN goes on in that conversation with what is not DONE. scripts/fake_agent.py works by the real hooks and tools:
// `node web/test/e2e_live.mjs main chromium`.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { boardKit, pressRun } from "./boardkit.mjs";
import { check, until } from "./check.mjs";

const FAKE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../scripts/fake_agent.py");

export async function liveMain(page, live) {
  const { j, card, open, cli } = boardKit(page, live);
  const project = (await j("/api/task-projects", "POST", { name: "Mains", dir: live.work })).id;
  const agent = (await j("/api/task-agents", "POST", { project, name: "Main runner", kind: "claude", command: `python3 -I ${FAKE} ${cli} --kind claude`, at_once: 2 })).id;
  const made = async (title, more = {}) => (await j("/api/tasks", "POST", { title, project, ...more })).id;
  const task = (id) => j(`/api/tasks/${id}`);
  const has = (id, status) => until(async () => (await task(id)).status === status, 20000);
  const main = await made("Main of three");
  const [a, b, done] = [await made("first subtask", { parent: main }), await made("second subtask", { parent: main }), await made("done before", { parent: main })];
  await j(`/api/tasks/${done}`, "PATCH", { status: "DONE" });
  await open(`?project=${project}`);
  await page.waitForSelector(card(main));

  const run = async (words) => {
    await pressRun(page, main);
    await page.waitForSelector("#runbox[open]");
    const [head, prompt, whole] = [await page.textContent("#runhead"), await page.textContent("#runprompt"), await page.evaluate(() => document.querySelector("#runwhat").open)];
    if (await page.isVisible("#runagent")) await page.selectOption("#runagent", agent);   // going on, its agent is its run's
    await page.fill("#runmessage", words);
    await page.click("#rungo");
    await until(async () => !(await page.$("#runbox[open]")));
    return { head, prompt, whole };
  };

  const first = await run("STUCK");
  check("AC-185 RUN's window names the main task and its subtasks still to do", first.head === `Run ${main}, ${a}, ${b}`, first.head);
  check("AC-185 what will be sent stands open and says how they go together", first.whole && first.prompt.startsWith(`The human ran main task ${main} with its 2 subtasks still to do: ${a}, ${b}.`) && /a main task goes last/.test(first.prompt), first.prompt.slice(0, 300));
  check("AC-185 each one's text shows before Run, the DONE subtask's not", first.prompt.includes("first subtask") && !first.prompt.includes("done before"));
  check("AC-185 what cannot be done fails with its main task; the rest is DONE", await has(a, "DONE") && await has(b, "FAILED") && await has(main, "FAILED"), `${(await task(a)).status} ${(await task(b)).status} ${(await task(main)).status}`);
  const runs = async (id) => (await task(id)).history.filter((e) => /^Session started/.test(e.text ?? "")).map((e) => e.text);
  check("AC-185 one session ran them all", (await runs(main)).length === 1 && JSON.stringify(await runs(a)) === JSON.stringify(await runs(main)) && JSON.stringify(await runs(b)) === JSON.stringify(await runs(main)), JSON.stringify([await runs(main), await runs(a)]));
  check("AC-185 the agent's reason stands on the failed subtask", (await task(b)).history.some((e) => /It cannot be done/.test(e.text ?? "")));

  await page.waitForSelector(`${card(main)} .crun`);
  const offer = await j(`/api/tasks/${main}/run?agent=${agent}`);
  check("AC-185 run again on the same agent, its run's tasks go on in its conversation", offer.first === false && JSON.stringify(offer.ids) === JSON.stringify([main, b]), JSON.stringify({ first: offer.first, ids: offer.ids }));
  const again = await run("FINISHES");
  check("AC-185 run again, the main task takes what is not DONE", again.head === `Run ${main}, ${b} again`, again.head);
  check("AC-185 and all are DONE, the same conversation gone on", await has(b, "DONE") && await has(main, "DONE") && (await runs(main)).length === 1 && (await runs(a)).length === 1, JSON.stringify(await runs(main)));
  const said = async (id) => (await task(id)).history.filter((e) => e.kind === "comment" && e.text === "FINISHES").length;
  const kept = [await said(main), await said(b)];
  check("AC-185 the press's message is kept on each of its tasks", kept[0] === 1 && kept[1] === 1, kept.join(" "));
}
