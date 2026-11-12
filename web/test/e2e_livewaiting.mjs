// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-122, AC-157: the human answers before the waiting session closes. Use the real minute of quiet;
// the accelerated clock can close it while the browser opens RUN, testing a different path.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { boardKit, nap, pressCard } from "./boardkit.mjs";
import { check, until } from "./check.mjs";

const FAKE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../scripts/fake_agent.py");

export async function liveWaitingRun(page, opts) {
  // Only the harness's own server is restarted; restore its accelerated clock for the next suite.
  await opts.stop();
  await opts.start("1");
  try {
    const { j, open, card, cli } = boardKit(page, opts);
    await open();
    const name = `Waiting ${page.context().browser().browserType().name()} ${page.viewportSize().width}`;
    const project = (await j("/api/task-projects", "POST", { name, dir: opts.work })).id;
    const agent = (await j("/api/task-agents", "POST", { name, kind: "claude", command: `python3 -I ${FAKE} ${cli} --kind claude` })).id;
    const id = (await j("/api/tasks", "POST", { title: "answer before closing QUESTIONS", project })).id;
    await j(`/api/tasks/${id}/run`, "POST", { agent });
    check("AC-157 the agent asks the human and ends its turn", await until(async () => (await j(`/api/tasks/${id}`)).status === "WAIT FOR HUMAN", 15000));
    const running = await j(`/api/tasks/${id}`), conversation = running.agent?.conversation, key = running.session?.key;
    await open(`?project=${project}`);
    await page.waitForSelector(card(id));
    await pressCard(page, id);
    await page.waitForSelector("#taskbox[open]");
    const waiting = await j(`/api/tasks/${id}`);
    const run = page.locator("#taskbox").getByRole("button", { name: "Run", exact: true });
    check("AC-122 WAIT FOR HUMAN holds its open session and offers RUN", waiting.status === "WAIT FOR HUMAN" && !!waiting.run_holds && !waiting.session?.closed && await run.isVisible());
    await page.fill(".newcomment textarea", "Use v2 and keep working WORKS");
    await run.click();
    await page.waitForSelector("#runbox[open]");
    await page.click("#rungo");
    const took = await until(async () => (await j(`/api/tasks/${id}`)).status === "IN PROGRESS", 15000);
    const continued = await j(`/api/tasks/${id}`);
    check("AC-122 the answer continues in the same open pane and conversation", took && continued.session?.key === key && continued.agent?.conversation === conversation, JSON.stringify(continued.session));
    await nap(1000);
    const kept = await j(`/api/tasks/${id}`);
    check("AC-157 the answer is kept once and the resumed work stays open", !kept.session?.closed && kept.status === "IN PROGRESS" && kept.history.filter((e) => e.kind === "comment" && e.text === "Use v2 and keep working WORKS").length === 1);
    await j(`/api/tasks/${id}/give-up`, "POST");
  } finally {
    await opts.stop();
    await opts.start();
  }
}
