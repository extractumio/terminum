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
  const shot = (name) => shots && page.locator("#projectbox").screenshot({ path: `${shots}integration-${name}-${browser}.png` });
  await pressAt(page, "#projectintegration");
  await page.waitForSelector("#integrationpanel:not([hidden]) #integrationfields:not([hidden])");
  check("AC-186 the project's Integration tab, off, with the defaults",
    await page.textContent("#projecthead") === "Fresh" && await page.getAttribute("#projectintegration", "aria-selected") === "true" && !(await page.isChecked("#integrationon"))
      && await page.inputValue("#integrationtarget") === "origin/main" && await page.inputValue("#integrationbatch") === "8" && await page.inputValue("#integrationminutes") === "90"
      && await page.inputValue("#integrationbudget") === "60" && await page.inputValue("#integrationrounds") === "3");
  const options = await page.$$eval("#integrationagent option", (os) => os.map((o) => o.textContent));
  check("AC-186 the agent is chosen from the project's own pool, none by default",
    JSON.stringify(options) === '["None","Its first"]' && await page.inputValue("#integrationagent") === "" && await page.isVisible("#integrationnoagent"), JSON.stringify(options));
  check("AC-186 an empty line says what it means",
    await page.isVisible("#integrationcheckhint") && await page.textContent("#integrationpublishhint") === "terminum pushes the candidate to origin/main, never forced." && await page.isVisible("#integrationafterhint"));
  check("AC-198 the seven stages stand in order, Queue terminum's own with no field",
    JSON.stringify(await page.$$eval("#integrationfields > .fieldrow", (rows) => rows.map((r) => r.firstChild.textContent.trim()).filter((t) => /^(Prepare|Verify|Queue|Check|Publish|After|Cleanup)$/.test(t))))
      === JSON.stringify(["Prepare", "Verify", "Queue", "Check", "Publish", "After", "Cleanup"]) && !(await page.$("#integrationqueue input")));
  check("AC-198 an empty Prepare, Verify or Cleanup says what it means",
    await page.isVisible("#integrationpreparehint") && await page.isVisible("#integrationverifyhint") && await page.textContent("#integrationcleanuphint") === "Nothing runs after a batch.");
  check("AC-186 the prompt is the default", (await page.inputValue("#integrationprompt")).startsWith("Task {task}: integration batch {batch} of {project}"));
  await page.evaluate(() => { document.querySelector("#projectbox form").scrollTop = 0; });
  await shot("top");
  await page.click("#integrationwhat summary");
  await shot("defaults");
  if (shots) {   // the window at a phone's width too, for its look (docs/UI-GUIDELINES.md §6)
    const wide = page.viewportSize();
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({ path: `${shots}integration-phone-${browser}.png`, fullPage: true });
    await page.setViewportSize(wide);
  }
  check("AC-186 What runs says where and how, in terminumd's words", /integration\/[^/]+\/tree: origin\/main/.test(await page.textContent("#integrationruns")) && /login shell/.test(await page.textContent("#integrationruns")));

  await commandTyping(page);

  await page.check("#integrationon");
  await pressAt(page, "#integrationok");
  check("AC-186 on, a project with no folder is refused where it was pressed",
    await until(async () => (await page.textContent("#integrationerror")) === "Fresh has no folder: set it in its window.") && await page.isVisible("#integrationpanel"));
  await shot("refused");
  check("AC-186 nothing is kept", !(await j(url)).on);

  await page.uncheck("#integrationon");
  await page.fill("#integrationcheck", "make smoke");
  await page.fill("#integrationverify", "make test-changed");
  await page.fill("#integrationcleanup", "make prune");
  check("AC-186 a line typed hides what empty meant", await page.isHidden("#integrationcheckhint"));
  await page.selectOption("#integrationagent", { label: "Its first" });
  check("AC-186 an agent chosen hides the words of none", await page.isHidden("#integrationnoagent"));
  await page.fill("#integrationbatch", "9");
  await pressAt(page, "#integrationok");
  check("AC-186 a field out of its range is refused in its words, what was typed stays",
    await until(async () => (await page.textContent("#integrationerror")) === "Tasks in a batch: 1 to 8.") && await page.inputValue("#integrationcheck") === "make smoke");
  await page.fill("#integrationbatch", "4");
  await pressAt(page, "#integrationok");
  await until(async () => (await j(url)).batch_most === 4);
  const kept = await j(url);
  const agent = (await j("/api/tasks/meta")).agents.find((a) => a.project === fresh && a.name === "Its first")?.id;
  check("AC-198 Save keeps the stages' lines", kept.verify === "make test-changed" && kept.cleanup === "make prune" && kept.prepare === "", JSON.stringify(kept));
  check("AC-186 Save keeps them", kept.check === "make smoke" && kept.batch_most === 4 && kept.agent === agent && !kept.on && kept.prompt === "", JSON.stringify(kept));
  await pressAt(page, "#projecttabgeneral");
  await pressAt(page, "#projectintegration");
  await page.waitForSelector("#integrationpanel:not([hidden]) #integrationfields:not([hidden])");
  check("AC-186 chosen again it shows what was kept",
    await page.inputValue("#integrationcheck") === "make smoke" && await page.inputValue("#integrationagent") === agent && /Check runs `make smoke`/.test(await page.textContent("#integrationruns")));
  check("AC-198 What runs names each stage's folder",
    /Verify runs `make test-changed` in .*verify-<task>, a tree of its own/.test(await page.textContent("#integrationruns")) && /Cleanup runs `make prune` in/.test(await page.textContent("#integrationruns")));

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
  if (shots) {   // the window's other tabs, for their look (docs/UI-GUIDELINES.md §6)
    for (const tab of ["agents", "general"]) {
      await pressAt(page, `#projecttab${tab}`);
      await page.evaluate(() => { document.querySelector("#projectbox form").scrollTop = 0; });
      await shot(`tab-${tab}`);
    }
  }
  await pressAt(page, "#projecttabgeneral");
  return gitty;
}

/** AC-193: the INTEGRATION column where a shown project integrates: its head's keys, Pause and Resume said where
 *  pressed, Batches…, and a human's Integrate refused in its words in the task's window; the board comes back after. */
export async function columnChecks(page, { j, gitty }) {
  const back = page.url(), col = '.col[data-status="INTEGRATION"]';
  await j(`/api/task-projects/${gitty}/integration`, "PUT", { on: true });   // no Check: nothing runs while it is looked at
  await page.goto(new URL(`/kanban/?project=${gitty}`, back).href);
  const tab = '#colswitch button[data-status="INTEGRATION"]';   // wider than a phone, its tab shows it in the panel (terminum-kanban.md AC-105)
  await page.waitForSelector(`${tab}:not([hidden])`, { state: "attached", timeout: 8000 });
  if (await page.isVisible(".colpanel")) await page.click(tab);
  await page.waitForSelector(`${col}:not([hidden]) .colhead .queuekeys button`, { timeout: 8000 });
  const key = (words) => `${col} .colhead .queuekeys button:has-text("${words}")`;
  check("AC-193 the column stands with Pause and Batches…, and no sort keys: its order is the queue's",
    await page.isVisible(key("Pause")) && await page.isVisible(key("Batches…")) && !(await page.$(`${col} .colsort`)));
  await page.click(key("Pause"));
  const paused = await until(async () => (await j("/api/tasks/meta")).projects.find((p) => p.id === gitty)?.queue?.paused === true && await page.isVisible(key("Resume")));
  check("AC-193 Pause pauses the queue and becomes Resume; the column says who paused it",
    paused && await until(async () => /^Paused by /.test(await page.textContent(`${col} .colnote`))));
  await page.click(key("Resume"));
  check("AC-193 Resume starts it again", await until(async () => (await j("/api/tasks/meta")).projects.find((p) => p.id === gitty)?.queue?.paused === false && await page.isVisible(key("Pause"))));
  await page.click(key("Batches…"));
  await page.waitForSelector("#batcheswin[open]");
  check("AC-193 Batches… lists the project's batches: none yet", await until(() => page.isVisible("#batchesnone")));
  await page.click("#batchesclose");
  const t = (await j("/api/tasks", "POST", { title: "integrate me", project: gitty })).id;
  await j(`/api/tasks/${t}`, "PATCH", { status: "IN PROGRESS" });
  await page.goto(new URL(`/kanban/${t}?project=${gitty}`, back).href);
  await page.waitForSelector("#taskbox[open] .queuekeys button");
  await page.click("#taskbox[open] .queuekeys button");
  check("AC-193 Integrate on a task with no session's folder is refused in integrate's words, in its window",
    await until(async () => (await page.textContent("#taskbox[open] .queuekeys .error")) === "This session has no folder on this Mac: name the commit."));
  await page.goto(back);
}

/** AC-186: play the browser's substitutions on the real fields; paste, drop, IME and prose keep their text. */
async function commandTyping(page) {
  const wide = page.viewportSize();
  const smart = (field, value, inputType, isComposing = false) => page.$eval(`#integration${field}`, (f, [v, t, composing]) => {
    f.value = v;
    f.setSelectionRange(v.length, v.length);
    f.dispatchEvent(new InputEvent("input", { inputType: t, isComposing: composing, bubbles: true }));
    return { value: f.value, at: f.selectionEnd };
  }, [value, inputType, isComposing]);
  for (const viewport of [wide, { width: 390, height: 844 }]) {
    await page.setViewportSize(viewport);
    for (const field of ["prepare", "verify", "check", "publish", "after", "cleanup"]) {
      const sel = `#integration${field}`, was = await page.inputValue(sel);
      await page.focus(sel);
      await smart(field, "make -", "insertText");
      const dash = await smart(field, "make —", "insertText");
      check(`AC-186 ${viewport.width}px ${field}: typed -- and its caret survive Smart Dashes`, dash.value === "make --" && dash.at === 7, JSON.stringify(dash));
      const quotes = await smart(field, "make -- “two” ‘one’ –", "insertReplacementText");
      check(`AC-186 ${viewport.width}px ${field}: typed quotes and en dash return to command syntax`, quotes.value === `make -- "two" 'one' -` && quotes.at === quotes.value.length, JSON.stringify(quotes));
      for (const [type, composing] of [["insertFromPaste", false], ["insertFromDrop", false], ["insertCompositionText", false], ["insertText", true]]) {
        const text = `make “verbatim — ${type} ${composing}”`;
        const kept = await smart(field, text, type, composing);
        const continued = await smart(field, `${text} x`, "insertText");
        check(`AC-186 ${viewport.width}px ${field}: ${type} composing=${composing} stays verbatim when typing continues`, kept.value === text && continued.value === `${text} x`, JSON.stringify({ kept, continued }));
      }
      await page.fill(sel, was);
    }
    for (const field of ["target", "prompt"]) {
      const sel = `#integration${field}`, was = await page.inputValue(sel), text = "origin/“smart”—‘text’";
      await page.focus(sel);
      const kept = await smart(field, text, "insertReplacementText");
      check(`AC-186 ${viewport.width}px ${field} keeps typed Unicode punctuation`, kept.value === text, JSON.stringify(kept));
      await page.fill(sel, was);
    }
  }
  await page.setViewportSize(wide);
}
