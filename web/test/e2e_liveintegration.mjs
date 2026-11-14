// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A project's integration settings in its window (terminum-integration.md AC-186), on the real page against a
// private terminumd: the defaults, the agent chosen from the project's own pool, terminumd's refusals where they
// were made, what is kept, and turning it on for a git checkout. Run from e2e_liveboardagents.mjs's project window.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { check, until } from "./check.mjs";

const git = (dir, ...args) => execFileSync("git", ["-C", dir, "-c", "user.name=t", "-c", "user.email=t@example.com", ...args], { stdio: "pipe" });

/** In the open window of project `fresh`, with no folder and one agent, "Its first"; `work` a folder of the test's own. */
export async function integrationChecks(page, { j, fresh, work, pressAt, shots }) {
  const url = `/api/task-projects/${fresh}/integration`;
  const browser = page.context().browser()?.browserType().name() ?? "browser";
  const shot = (name) => shots && page.locator("#integrationbox").screenshot({ path: `${shots}integration-${name}-${browser}.png` });
  await pressAt(page, "#projectintegration");
  await page.waitForSelector("#integrationbox[open]");
  check("AC-186 Integration… opens its window, off, with the defaults",
    await page.textContent("#integrationhead") === "Integration of Fresh" && !(await page.isChecked("#integrationon"))
      && await page.inputValue("#integrationtarget") === "origin/main" && await page.inputValue("#integrationbatch") === "8" && await page.inputValue("#integrationminutes") === "90"
      && await page.inputValue("#integrationbudget") === "60" && await page.inputValue("#integrationrounds") === "3");
  const options = await page.$$eval("#integrationagent option", (os) => os.map((o) => o.textContent));
  check("AC-186 the agent is chosen from the project's own pool, none by default",
    JSON.stringify(options) === '["None","Its first"]' && await page.inputValue("#integrationagent") === "" && await page.isVisible("#integrationnoagent"), JSON.stringify(options));
  check("AC-186 an empty line says what it means",
    await page.isVisible("#integrationcheckhint") && await page.textContent("#integrationpublishhint") === "terminum pushes the candidate to origin/main, never forced." && await page.isVisible("#integrationafterhint"));
  check("AC-186 the prompt is the default", (await page.inputValue("#integrationprompt")).startsWith("Task {task}: integration batch {batch} of {project}"));
  await page.click("#integrationwhat summary");
  await shot("defaults");
  if (shots) {   // the window at a phone's width too, for its look (docs/UI-GUIDELINES.md §6)
    const wide = page.viewportSize();
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({ path: `${shots}integration-phone-${browser}.png`, fullPage: true });
    await page.setViewportSize(wide);
  }
  check("AC-186 What runs says where and how, in terminumd's words", /integration\/[^/]+\/tree: origin\/main/.test(await page.textContent("#integrationruns")) && /login shell/.test(await page.textContent("#integrationruns")));

  await page.check("#integrationon");
  await pressAt(page, "#integrationok");
  check("AC-186 on, a project with no folder is refused where it was pressed",
    await until(async () => (await page.textContent("#integrationerror")) === "Fresh has no folder: set it in its window.") && await page.isVisible("#integrationbox"));
  await shot("refused");
  check("AC-186 nothing is kept", !(await j(url)).on);

  await page.uncheck("#integrationon");
  await page.fill("#integrationcheck", "make smoke");
  check("AC-186 a line typed hides what empty meant", await page.isHidden("#integrationcheckhint"));
  await page.selectOption("#integrationagent", { label: "Its first" });
  check("AC-186 an agent chosen hides the words of none", await page.isHidden("#integrationnoagent"));
  await page.fill("#integrationbatch", "9");
  await pressAt(page, "#integrationok");
  check("AC-186 a field out of its range is refused in its words, what was typed stays",
    await until(async () => (await page.textContent("#integrationerror")) === "Tasks in a batch: 1 to 8.") && await page.inputValue("#integrationcheck") === "make smoke");
  await page.fill("#integrationbatch", "4");
  await pressAt(page, "#integrationok");
  await page.waitForSelector("#integrationbox", { state: "hidden" });
  const kept = await j(url);
  const agent = (await j("/api/tasks/meta")).agents.find((a) => a.project === fresh && a.name === "Its first")?.id;
  check("AC-186 Save keeps them", kept.check === "make smoke" && kept.batch_most === 4 && kept.agent === agent && !kept.on && kept.prompt === "", JSON.stringify(kept));
  await pressAt(page, "#projectintegration");
  await page.waitForSelector("#integrationbox[open]");
  check("AC-186 opened again it shows what was kept",
    await page.inputValue("#integrationcheck") === "make smoke" && await page.inputValue("#integrationagent") === agent && /Check runs `make smoke`/.test(await page.textContent("#integrationruns")));
  await page.evaluate(() => document.getElementById("integrationbox").close());

  // on, for a git checkout with its target fetched: kept; a target it lacks: refused in git's terms
  const repo = path.join(work, "integration-repo");
  fs.mkdirSync(repo, { recursive: true });
  git(repo, "init", "-q");
  git(repo, "commit", "-q", "--allow-empty", "-m", "one");
  git(repo, "update-ref", "refs/remotes/origin/main", "HEAD");
  const gitty = (await j("/api/task-projects", "POST", { name: "Gitty", dir: repo })).id;
  const on = await j(`/api/task-projects/${gitty}/integration`, "PUT", { on: true, check: "make smoke" });
  const lacks = await j(`/api/task-projects/${gitty}/integration`, "PUT", { on: true, target: "origin/nope" });
  check("AC-186 on for a git checkout is kept; a target it has not fetched is refused",
    on.ok === true && (await j(`/api/task-projects/${gitty}/integration`)).on === true && lacks.error === "origin/nope: no such branch at origin.", JSON.stringify({ on, lacks }));
}
