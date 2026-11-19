// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum-schedules.md AC-161, AC-162 against a real private terminumd and tmux, stand-in agents: an agent keeps
// ceil(N / 2) slots for its N schedules, shown as rings naming none; a schedule's task with no room waits ahead of
// an ordinary wait; a newer Run now supersedes it and takes its place.
import { fileURLToPath } from "node:url";
import { boardKit } from "./boardkit.mjs";
import { check, until } from "./check.mjs";

const FAKE = fileURLToPath(new URL("../../scripts/fake_agent.py", import.meta.url));

export async function liveKept(page, live) {
  const { j, cli, open } = boardKit(page, live);
  const size = page.viewportSize().width < 860 ? "phone" : "wide";
  const project = (await j("/api/task-projects", "POST", { name: `Kept slots ${size}`, dir: live.work })).id;
  const agent = (await j("/api/task-agents", "POST", { project, name: `Kept owl ${size}`, kind: "claude", command: `python3 -I ${FAKE} ${cli} --kind claude`, at_once: 2 })).id;
  const schedules = [];
  for (const title of ["Kept one", "Kept two", "Kept three"]) schedules.push(await j("/api/schedules", "POST", { title, body: "WORKS", project, agent, cron: "0 0 1 1 *", budget_minutes: 5 }));
  const slots = async () => (await j(`/api/tasks?project=${project}&limit=1&counts=1`)).slots;
  const queue = async () => ((await slots()).queue?.[agent] ?? []).map((t) => t.id);
  const runs = [];
  try {
    await open(`?project=${project}`);
    if (page.viewportSize().width < 860) await page.click("#poolline");
    const owl = `#agentbar .agentcard[data-agent="${agent}"]`;
    check("AC-161 three schedules keep two slots: two rings, naming none", await until(async () => {
      const rings = await page.$$eval(`${owl} .agkept`, (all) => all.map((m) => m.title));
      return rings.length === 2 && rings.every((t) => t === "kept for 3 schedules");
    }), JSON.stringify((await slots()).kept?.[agent]));
    const ordinary = (await j("/api/tasks", "POST", { title: "Ordinary WORKS", project })).id;
    const offered = await j(`/api/tasks/${ordinary}/run`, "POST", { agent });
    check("AC-161 an ordinary run never takes a kept slot", /is at its limit of 2: 2 kept for 3 schedules\./.test(offered.full?.words ?? ""), JSON.stringify(offered));
    await j(`/api/tasks/${ordinary}/run`, "POST", { agent, queue: true });
    for (const s of schedules.slice(0, 2)) {
      const { task } = await j(`/api/schedules/${s.id}/run`, "POST");
      runs.push(task);
      check(`AC-161 ${s.key}'s task starts at once in a kept slot`, await until(async () => (await j(`/api/tasks/${task}`)).session?.key && !(await queue()).includes(task), 20000), task);
    }
    const { task: first } = await j(`/api/schedules/${schedules[2].id}/run`, "POST");
    check("AC-161 with no room its task waits ahead of the ordinary wait", await until(async () => (await queue()).join() === [first, ordinary].join(), 15000), JSON.stringify(await queue()));
    const { task: newer } = await j(`/api/schedules/${schedules[2].id}/run`, "POST");
    check("AC-162 a newer Run now supersedes the waiting task and takes its place", await until(async () => (await queue()).join() === [newer, ordinary].join() && (await j(`/api/tasks/${first}`)).status === "FAILED", 15000),
      JSON.stringify({ queue: await queue(), first: (await j(`/api/tasks/${first}`)).history?.map((e) => e.text).slice(-2) }));
    check("AC-162 it says why", (await j(`/api/tasks/${first}`)).history.some((e) => e.text === `Superseded by ${newer}, a newer run of ${schedules[2].key}, while it waited for room.`));
    check("AC-169 the card shows both queued, the count of what holds its slots", await until(async () => await page.locator(`${owl} .agqueue .agwait`).count() === 2 && /2\/2/.test(await page.textContent(`${owl} .agcount`))), await page.textContent(owl));
    runs.push(newer, ordinary);
  } finally {
    for (const id of runs) await j(`/api/tasks/${id}/give-up`, "POST");
    for (const s of schedules) await j(`/api/schedules/${s.id}`, "DELETE");
    await until(async () => (await j(`/api/task-agents/${agent}`, "DELETE")).code === 200, 15000);
  }
}
