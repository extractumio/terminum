// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The pool on the board, against a real private terminumd (e2e_live.mjs; terminum-dispatcher.md AC-137,
// AC-138, AC-140; terminum-kanban.md AC-139): kept, listed, changed and deleted by a human, refused with
// words; tasks run on one by a press or a drop, several at once, at their project's place; a project's own window.
// scripts/fake_agent.py speaks by the real hooks and tools.
import fs from "node:fs";
import path from "node:path";
import { boardKit, pressCard, pressRun } from "./boardkit.mjs";
import { check, until } from "./check.mjs";
import { agentPool } from "./e2e_liveboardpool.mjs";
import { integrationChecks } from "./e2e_liveintegration.mjs";

const COMMAND = "claude --model opus";

export async function liveBoardAgents(page, opts) {
  const { base, shots } = opts;
  const { j, backToSessions } = boardKit(page, opts);
  const project = (await j("/api/task-projects", "POST", { name: "AgentsHere" })).id;
  await page.goto(`${base}/kanban/?project=${project}`);
  await page.waitForSelector("#addagent");
  check("AC-137 an empty pool says so", await until(async () => /AgentsHere has no agents yet/.test(await page.textContent(".agents"))), await page.textContent(".agents"));

  await page.click("#addagent");
  await page.waitForSelector("#agentbox[open]");
  check("AC-137 the window asks its project, the one shown, and no server or folder", await page.inputValue("#agentproject") === project && !(await page.$("#agenthost")) && !(await page.$("#agentdir")));
  check("AC-137 its kind fills in name and command", await page.inputValue("#agentname") === "Claude Code" && await page.inputValue("#agentcommand") === "claude");
  await page.selectOption("#agentkind", "codex");
  check("AC-137 another kind changes them", await page.inputValue("#agentname") === "Codex" && await page.inputValue("#agentcommand") === "codex");
  await page.fill("#agentcommand", "codex --search");
  await page.selectOption("#agentkind", "claude");
  check("AC-137 what the user wrote stays", await page.inputValue("#agentcommand") === "codex --search" && await page.inputValue("#agentname") === "Claude Code");
  // a browser's Smart Dashes and Smart Quotes, as Safari makes them while one types; a paste stays as it came
  const smart = (value, inputType) => page.$eval("#agentcommand", (f, [v, t]) => { f.value = v; f.dispatchEvent(new InputEvent("input", { inputType: t, bubbles: true })); return f.value; }, [value, inputType]);
  await smart("claude -", "insertText");
  check("AC-137 a command kept as typed: -- stays", await smart("claude —", "insertText") === "claude --");
  check("AC-137 a command kept as typed: quotes stay", await smart("claude --append-system-prompt “be brief”", "insertReplacementText") === "claude --append-system-prompt \"be brief\"");
  check("AC-137 a paste stays as it came", await smart("claude --append-system-prompt \"be brief — always\"", "insertFromPaste") === "claude --append-system-prompt \"be brief — always\"");
  check("AC-137 the field asks for no autocorrect", await page.getAttribute("#agentcommand", "autocorrect") === "off");
  await page.fill("#agentname", "Opus reviewer");
  await page.fill("#agentcommand", COMMAND);
  if (shots) await page.screenshot({ path: path.join(shots, "agent-window.png") });

  // what is wrong is said in the window, nothing kept
  const refused = async (field, value, words) => {
    const was = await page.inputValue(field);
    await page.fill(field, value);
    await page.click("#agentok");
    check(`AC-137 refused: ${words}`, await until(async () => (await page.textContent("#agenterror")) === words) && await page.$("#agentbox[open]"), await page.textContent("#agenterror"));
    await page.fill(field, was);
  };
  await refused("#agentname", "", "An agent needs a name.");
  await refused("#agentcommand", "", "An agent needs a command.");
  await refused("#agentcommand", "x".repeat(1001), "An agent's command is 1000 characters at most.");
  const said = await j("/api/task-agents", "POST", { project, name: "line", kind: "claude", command: "claude\nrm -rf x" });
  check("AC-137 refused: a line break in a command", said.code === 400 && /one line/.test(said.error), JSON.stringify(said));
  check("AC-137 nothing was kept", (await j("/api/tasks/meta")).agents.filter((a) => a.project === project).length === 0);

  // a slip asks, Cancel means it
  await page.keyboard.press("Escape");
  check("AC-137 Esc with text typed asks", await until(() => page.$("#confirmbox[open]")));
  await page.click('#confirmbox button[value="cancel"]');
  check("AC-137 and keeps it", await page.$("#agentbox[open]") && await page.inputValue("#agentname") === "Opus reviewer");

  await page.press("#agentname", "Enter");
  const row = '.agents .agentcard:has-text("Opus reviewer")';
  check("AC-137 Enter keeps it and it stands under Agents", await until(() => page.isVisible(row)), await page.textContent(".agents"));
  check("AC-137 the window closes", !(await page.$("#agentbox[open]")));
  const hover = () => page.getAttribute(row, "title");
  check("AC-137 its rectangle says its kind; hovering, its command", await page.isVisible(`${row} .agmark img`) && /Claude Code/.test(await page.textContent(row)) && (await hover()).includes(COMMAND), await hover());
  const stored = (await j("/api/tasks/meta")).agents.find((a) => a.name === "Opus reviewer");
  check("AC-137 its command is kept as written, and it knows its project and no place", stored?.command === COMMAND && stored.kind === "claude" && !("host" in stored) && !("dir" in stored) && stored.project === project, JSON.stringify(stored));
  if (shots) await page.screenshot({ path: path.join(shots, "agents-column.png") });

  // every page hears
  const other = await page.context().newPage();
  await other.goto(`${base}/kanban/?project=${project}`);
  await other.waitForSelector(row);
  await j("/api/task-agents", "POST", { project, name: "Codex two", kind: "codex", command: "codex" });
  check("AC-137 every page hears within 2 s", await until(() => other.isVisible('.agents .agentcard:has-text("Codex two")'), 2500));
  await other.close();
  check("AC-137 this one too", await until(() => page.isVisible('.agents .agentcard:has-text("Codex two")'), 2500));

  // a name stands once in the pool
  await page.click("#addagent");
  await page.fill("#agentname", "Opus reviewer");
  await page.click("#agentok");
  check("AC-137 a name the pool has is refused", await until(async () => /already has an agent named Opus reviewer/.test(await page.textContent("#agenterror"))), await page.textContent("#agenterror"));
  await page.click('#agentbox button[value="cancel"]');
  check("AC-137 Cancel closes with no question", !(await page.$("#confirmbox[open]")) && !(await page.$("#agentbox[open]")));

  // changed and deleted
  await page.click(row);
  check("AC-137 a press opens it filled", await page.inputValue("#agentcommand") === COMMAND && await page.isVisible("#agentdrop"));
  await page.fill("#agentcommand", "claude --model sonnet");
  await page.click("#agentok");
  check("AC-137 Save keeps a change", await until(async () => (await hover()).includes("claude --model sonnet")), await hover());
  await page.click(row);
  await page.click("#agentdrop");
  await page.click("#confirmok");
  check("AC-137 Delete, confirmed, takes it", await until(async () => !(await page.isVisible(row))), await page.textContent(".agents"));

  // each project has a pool of its own
  const elsewhere = (await j("/api/task-projects", "POST", { name: "AgentsThere", dir: opts.work })).id;
  await page.goto(`${base}/kanban/?project=${elsewhere}`);
  await page.waitForSelector("#addagent");
  check("AC-137 another project lists none of it", await until(async () => /AgentsThere has no agents yet/.test(await page.textContent(".agents"))) && !(await page.isVisible('.agents .agentcard:has-text("Codex two")')), await page.textContent(".agents"));
  await page.click("#addagent");
  check("AC-137 a new one is the shown project's", await page.inputValue("#agentproject") === elsewhere && await page.isEnabled("#agentproject"), await page.inputValue("#agentproject"));
  await page.fill("#agentname", "Codex two");
  await page.click("#agentok");
  check("AC-137 a name another project has is free here", await until(() => page.isVisible('.agents .agentcard:has-text("Codex two")')), await page.textContent("#agenterror"));
  const pools = (await j("/api/tasks/meta")).agents.filter((a) => a.name === "Codex two");
  check("AC-137 they are two agents, each its project's", pools.length === 2 && pools.some((a) => a.project === project) && pools.some((a) => a.project === elsewhere), JSON.stringify(pools.map((a) => a.project)));
  const [mine, theirs] = [pools.find((a) => a.project === project), pools.find((a) => a.project === elsewhere)];
  await page.click(`.agents .agentcard[data-agent="${theirs.id}"]`);
  check("AC-137 its window shows its project and does not change it", await page.inputValue("#agentproject") === elsewhere && await page.isDisabled("#agentproject"));
  await page.click('#agentbox button[value="cancel"]');
  const foreign = (await j("/api/tasks", "POST", { title: "not theirs to run", project: elsewhere })).id;
  const ran = await j(`/api/tasks/${foreign}/run`, "POST", { agent: mine.id });
  const named = await j(`/api/tasks/${foreign}/agent`, "PUT", { agent: mine.id });
  check("AC-137 another project's agent runs nothing of this one, said", [ran, named].every((r) => /Codex two is AgentsHere's agent, not AgentsThere's: choose one of its own\./.test(r.error ?? "")), JSON.stringify([ran, named]));
  check("AC-137 an agent needs a project", /An agent needs a project\./.test((await j("/api/task-agents", "POST", { name: "Nowhere", kind: "claude", command: "claude" })).error ?? ""));
  await page.click("#fproject summary");
  await page.click('#fproject button:has-text("All projects")');
  const both = () => page.$$eval('.agents .agentcard:has-text("Codex two")', (cards) => cards.map((c) => c.textContent));
  check("AC-137 with all projects each says whose it is", await until(async () => { const t = await both(); return t.length === 2 && t.some((x) => x.includes("AgentsHere")) && t.some((x) => x.includes("AgentsThere")); }), JSON.stringify(await both()));
  await page.click("#addagent");
  check("AC-137 with several shown a new one's project is the human's to choose", await page.inputValue("#agentproject") === "");
  await page.click("#agentok");
  check("AC-137 and none is refused where it was typed", await until(async () => /An agent needs a project/.test(await page.textContent("#agenterror"))), await page.textContent("#agenterror"));
  await page.click('#agentbox button[value="cancel"]');
  if (await page.$("#confirmbox[open]")) await page.click("#confirmok");
  await j(`/api/task-agents/${theirs.id}`, "DELETE");

  // a reload finds what stays
  await page.goto(`${base}/kanban/?project=${project}`);
  check("AC-137 a reload finds it", await until(() => page.isVisible('.agents .agentcard:has-text("Codex two")')));
  await agentPool(page, opts, pressAt);
  await projectWindowChecks(page, opts);
  await page.goto(`${base}/kanban/`);
  await page.waitForSelector("#addagent");
  await page.click("#fproject summary");
  await page.click('#fproject button:has-text("All projects")');
  await backToSessions();
}

/** A press where a window opened over another modal one reports "element is inert" on some Playwrights. */
const pressAt = async (page, sel) => {
  await page.waitForSelector(sel, { state: "visible" });
  // a tall window's keys below the screen: a press by place would miss them; a window opening, or redrawn
  // while terminumd answers, still moves: a press waits for where it stands, on the screen
  const where = async () => { const e = await page.$(sel); await e.scrollIntoViewIfNeeded(); return e.boundingBox(); };
  let was = null, b = await where();
  for (let i = 0; i < 40 && JSON.stringify(b) !== was; i++) {
    was = JSON.stringify(b);
    await new Promise((r) => setTimeout(r, 60));
    b = await where();
  }
  const at = [b.x + b.width / 2, b.y + b.height / 2];
  // what stands there when it is pressed, for a check that says where a press went
  const under = await page.evaluate(([x, y]) => { const e = document.elementFromPoint(x, y); return e ? `${e.tagName.toLowerCase()}#${e.id}${e.disabled ? " (off)" : ""} in ${e.closest("dialog")?.id ?? "no window"}` : "nothing"; }, at);
  await page.mouse.click(...at);
  return `pressed at ${at.map(Math.round)}: ${under}`;
};

/** AC-139, AC-140: a project made empty, its window stays for the pool; with no folder nothing starts; a pencil opens it; a change is kept. */
async function projectWindowChecks(page, opts) {
  const { base, work } = opts;
  const { j } = boardKit(page, opts);
  await page.goto(`${base}/kanban/`);
  await page.waitForSelector("#fproject summary");
  const named = async (name) => (await j("/api/tasks/meta")).projects.find((p) => p.name === name);
  await page.click("#fproject summary"); await page.click("#newproject");
  await page.waitForSelector("#projectbox[open]");
  check("AC-139 a new project's window asks no agents yet", !(await page.isVisible("#projectagents")) && !(await page.isVisible("#projectdelete")));
  await page.fill("#projectname", "Fresh");
  await page.click("#projectok");
  check("AC-139 + Project makes it empty", await until(async () => !!(await named("Fresh"))) && (await j(`/api/tasks?project=${(await named("Fresh")).id}&limit=5`)).tasks.length === 0);
  check("AC-139 the window stays, for its agents", await until(() => page.isVisible("#projectaddagent")) && await page.textContent("#projecthead") === "Fresh");
  const fresh = (await named("Fresh")).id;
  check("AC-139 it is the project shown", await until(() => new URL(page.url()).searchParams.get("project") === fresh), page.url());
  await pressAt(page, "#projectaddagent");
  check("AC-139 Add agent opens the agent's window on it", await until(() => page.$("#agentbox[open]")));
  await page.fill("#agentname", "Its first");
  await pressAt(page, "#agentok");
  check("AC-139 the agent stands in the project's window", await until(() => page.isVisible('#projectagentlist .agentcard:has-text("Its first")')),
    JSON.stringify(await page.evaluate(() => ({ asking: !!document.querySelector("#agentbox[open]"), why: document.getElementById("agenterror")?.textContent, list: document.getElementById("projectagentlist")?.textContent.slice(0, 200) }))));
  await pressAt(page, "#projectlimits");
  await page.waitForSelector("#limitsbox[open]");
  check("AC-128 the window says what it does", /every 30 s\. Two readings in a row over Warn tell the agent; over End its tasks fail and it is ended\. Nothing stops a process between readings/.test(await page.textContent("#limitstext")));
  check("AC-128 empty fields show the global values", await page.getAttribute("#limitwarn", "placeholder") === "4 (global)" && await page.getAttribute("#limitend", "placeholder") === "8 (global)");
  await page.fill("#limitwarn", "2");
  await page.fill("#limitend", "3");
  await pressAt(page, "#limitsok");
  check("AC-139 limits are kept from the project's window", await until(async () => (await j("/api/tasks/meta")).limits.some((l) => l.project === fresh && l.warn === 2 && l.end === 3)), JSON.stringify((await j("/api/tasks/meta")).limits));
  // terminumd keeps them before the page has its answer and closes the limits' window: a press meanwhile lands on it
  await page.waitForSelector("#limitsbox", { state: "hidden" });
  await integrationChecks(page, { j, fresh, work, pressAt });
  const done = await pressAt(page, "#projectok");
  check("AC-139 Done keeps the project and closes its window", await until(() => page.isHidden("#projectbox"), 15000),
    `${done}; ${JSON.stringify(await page.evaluate(() => ({ says: document.getElementById("projecterror")?.textContent, open: [...document.querySelectorAll("dialog[open]")].map((d) => d.id), busy: document.getElementById("projectbox").dataset.busy ?? null, focus: document.activeElement?.id })))}`);
  if (await page.isVisible("#projectbox")) await page.evaluate(() => document.getElementById("projectbox").close());

  // no folder: no one knows where an agent would start (AC-140)
  const homeless = (await j("/api/tasks", "POST", { title: "nowhere to run", project: fresh })).id;
  await page.waitForSelector(`.card[data-id="${homeless}"]`, { timeout: 8000 });
  await page.click("#fproject summary");
  check("AC-140 its row in the menu says no folder", await until(async () => /no folder/.test(await page.textContent('#fproject .projrow:has-text("Fresh")'))));
  await page.click("#fproject summary");
  await pressRun(page, homeless);
  check("AC-140 a press on Run says it and opens its window", await until(() => page.$("#projectbox[open]")) && /Fresh has no folder/.test(await page.textContent("#toast")), await page.textContent("#toast"));
  check("AC-140 no run window opens", !(await page.$("#runbox[open]")));
  check("AC-139 its window says nothing can start", await page.isVisible("#projectnodir") && /No agent can start here until it has a folder\./.test(await page.textContent("#projectnodir")));
  await page.click('#projectbox button[value="cancel"]');
  await page.waitForSelector("#projectbox", { state: "hidden" });
  const refusedRun = await j(`/api/tasks/${homeless}/run`, "POST", { agent: (await j("/api/tasks/meta")).agents[0].id });
  check("AC-140 terminumd refuses it too, and nothing starts", /Fresh has no folder: set it in its window/.test(refusedRun.error) && (await j(`/api/tasks/${homeless}`)).status === "TODO", JSON.stringify(refusedRun));
  await pressCard(page, homeless);
  await page.waitForSelector("#taskbox[open]");
  await page.click('#taskacts button:has-text("Archive")');
  await page.click("#taskclose");

  const pencil = async (name) => {
    await page.click("#fproject summary");
    await page.click(`#fproject .projrow:has-text("${name}") .projedit`);
    await page.waitForSelector("#projectbox[open]");
  };
  await pencil("Fresh");
  check("AC-139 the pencil on a project's row opens it", await page.inputValue("#projectname") === "Fresh" && await page.isVisible("#projectdelete") && await page.isDisabled("#projecthost"));
  // its folder: 200 characters read whole; the picker lists the server's folders, up and in (AC-139)
  await page.fill("#projectdir", `/${"a".repeat(199)}`);
  check("AC-139 a folder of 200 characters reads whole", await page.$eval("#projectdir", (f) => f.scrollHeight <= f.clientHeight + 1 && f.scrollWidth <= f.clientWidth + 1),
    JSON.stringify(await page.$eval("#projectdir", (f) => [f.scrollHeight, f.clientHeight, f.clientWidth])));
  // a folder of its own: the temporary folder above holds more than a page
  const parent = path.join(work, "picker"), leaf = "billing";
  fs.mkdirSync(path.join(parent, leaf), { recursive: true });
  await page.fill("#projectdir", parent);
  await pressAt(page, "#projectpick");
  await page.waitForSelector("#folderbox[open]");
  check("AC-139 the picker opens at the folder set", await until(async () => await page.textContent("#folderpath") === parent), await page.textContent("#folderpath"));
  check("AC-139 it lists folders as Files draws them, and up", await until(() => page.isVisible(`#folderlist .folderrow:has-text("${leaf}") svg`)) && await page.isVisible('#folderlist .folderrow[title="Up"]'));
  await page.click('#folderlist .folderrow[title="Up"]');
  check("AC-139 up goes to its parent", await until(async () => await page.textContent("#folderpath") === parent.replace(/\/[^/]+$/, "")), await page.textContent("#folderpath"));
  await page.click('#folderbox button[value="cancel"]');
  check("AC-139 Cancel leaves the field", await page.inputValue("#projectdir") === parent);
  await pressAt(page, "#projectpick");
  await until(async () => await page.textContent("#folderpath") === parent);
  await page.click(`#folderlist .folderrow:has-text("${leaf}")`);
  await until(async () => await page.textContent("#folderpath") === path.join(parent, leaf));
  await pressAt(page, "#folderok");
  check("AC-139 Choose puts its path in the field", await until(async () => await page.inputValue("#projectdir") === path.join(parent, leaf)), await page.inputValue("#projectdir"));
  await page.fill("#projectdir", work);             // the session's folder: what follows starts there
  await page.fill("#projectname", "Fresher");
  await page.click("#projectok");
  await page.waitForSelector("#projectbox", { state: "hidden" });
  check("AC-139 an edit of name and folder is kept", await until(async () => (await named("Fresher"))?.dir === work), JSON.stringify(await named("Fresher")));
  check("AC-139 the head says the new name", await until(async () => /Fresher/.test(await page.textContent("#fproject summary"))));
  await page.reload();
  await page.waitForSelector("#fproject summary");
  await until(async () => /Fresher/.test(await page.textContent("#fproject summary")));
  await pencil("Fresher");
  check("AC-139 and stands after a reload", await page.inputValue("#projectname") === "Fresher" && await page.inputValue("#projectdir") === work, await page.inputValue("#projectdir"));
  await page.click('#projectbox button[value="cancel"]');
  await page.waitForSelector("#projectbox", { state: "hidden" });
}
