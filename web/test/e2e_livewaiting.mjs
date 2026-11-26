// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-122, AC-157: the human answers before the waiting session closes. Use the real minute of quiet;
// the accelerated clock can close it while the browser opens RUN, testing a different path.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { boardKit, cardInSight, nap, pressCard } from "./boardkit.mjs";
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
    const agent = (await j("/api/task-agents", "POST", { project, name, kind: "claude", command: `python3 -I ${FAKE} ${cli} --kind claude` })).id;
    const id = (await j("/api/tasks", "POST", { title: "answer before closing QUESTIONS", project })).id;
    await j(`/api/tasks/${id}/run`, "POST", { agent });
    check("AC-157 the agent asks the human and ends its turn", await until(async () => (await j(`/api/tasks/${id}`)).status === "WAIT FOR HUMAN", 15000));
    const running = await j(`/api/tasks/${id}`), conversation = running.agent?.conversation, key = running.session?.key;
    await open(`?project=${project}`);
    await cardInSight(page, id);
    await pressCard(page, id);
    await page.waitForSelector("#taskbox[open]");
    const waiting = await j(`/api/tasks/${id}`);
    // its agent asked: Answer stands in Run's place and goes as RUN does (AC-285)
    const run = page.locator("#taskbox").getByRole("button", { name: "Answer", exact: true });
    check("AC-122 AC-285 WAIT FOR HUMAN holds its open session and offers Answer in RUN's place", waiting.status === "WAIT FOR HUMAN" && !!waiting.run_holds && !waiting.session?.closed && await run.isVisible());
    await page.fill(".newcomment textarea", "Use v2 and keep working WORKS");
    await run.click();                             // at once: RUN's window has nothing to ask (AC-122)
    await page.waitForSelector("#taskbox", { state: "hidden" });
    check("AC-122 AC-285 Answer in the task's window starts at once, RUN's window unopened", !(await page.isVisible("#runbox")));
    const took = await until(async () => (await j(`/api/tasks/${id}`)).status === "IN PROGRESS", 15000);
    const continued = await j(`/api/tasks/${id}`);
    check("AC-122 the answer continues in the same open pane and conversation", took && continued.session?.key === key && continued.agent?.conversation === conversation, JSON.stringify(continued.session));
    await nap(1000);
    const kept = await j(`/api/tasks/${id}`);
    check("AC-157 the answer is kept once and the resumed work stays open", !kept.session?.closed && kept.status === "IN PROGRESS" && kept.history.filter((e) => e.kind === "comment" && e.text === "Use v2 and keep working WORKS").length === 1);
    await j(`/api/tasks/${id}/give-up`, "POST");
    await savedComments(page, opts, { j, open, cli }, project);
  } finally {
    await opts.stop();
    await opts.start();
  }
}


// The browser's Save button and its empty Run field, with what reaches the private tmux pane.
async function savedComments(page, opts, { j, open, cli }, project) {
  const log = path.join(opts.work, `saved-${project}.log`);
  const prompts = () => (fs.existsSync(log) ? fs.readFileSync(log, "utf8").trim().split("\n") : []);
  const agent = (await j("/api/task-agents", "POST", {
    project, name: "Saved comments", kind: "claude", command: `python3 -I ${FAKE} ${cli} --kind claude --prompts "${log}"`,
  })).id;
  const id = (await j("/api/tasks", "POST", { title: "Save then run FINISHES", project })).id;
  await j(`/api/tasks/${id}/run`, "POST", { agent });
  const task = () => j(`/api/tasks/${id}`);
  check("AC-122 the first run ends without asking", await until(async () => (await task()).status === "DONE", 15000));
  const session = (await task()).session?.key;
  await open(`?project=${project}`);
  const show = async () => {
    await cardInSight(page, id);
    await pressCard(page, id);
    await page.waitForSelector("#taskbox[open]");
  };
  const box = page.locator("#taskbox");
  const comment = box.locator(".newcomment textarea");
  const save = box.getByRole("button", { name: "Save", exact: true });
  const run = box.getByRole("button", { name: "Run", exact: true });
  await show();
  for (const text of ["Use the v2 API", "Keep the old route FINISHES"]) {
    await comment.fill(text);
    await save.click();
    await until(() => comment.inputValue().then((value) => value === ""));
  }
  await run.click();
  await page.waitForSelector("#taskbox", { state: "hidden" });
  check("AC-122 Save, Save, Run sends both comments in order to the same pane", await until(async () => {
    const words = prompts()[1];
    return words?.match(/Use the v2 API/g)?.length === 1 && words?.match(/Keep the old route/g)?.length === 1
      && words.indexOf("Use the v2 API") < words.indexOf("Keep the old route") && (await task()).session?.key === session;
  }, 15000), prompts()[1]);
  check("AC-122 the continuation ends", await until(async () => (await task()).status === "DONE", 15000));
  await show();
  await run.click();
  check("AC-122 a second empty Run does not replay the comments", await until(() => {
    const words = prompts()[2];
    return words && !/Use the v2 API|Keep the old route/.test(words);
  }, 15000), prompts()[2]);
  await j(`/api/tasks/${id}/give-up`, "POST");
}
