// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Pool execution in a private terminumd and tmux (terminum-dispatcher.md AC-138, AC-141,
// AC-142; terminum-kanban.md AC-106): assignment, running, slots and task windows.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { boardKit, pressCard, pressRun, startOf } from "./boardkit.mjs";
import { check, until } from "./check.mjs";
import { subscriptionStripes } from "./e2e_liveboardsubs.mjs";

const FAKE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../scripts/fake_agent.py");

/** Whether tmux still has `pane`, by its whole ID: %1 is no part of %12. */
const paneAlive = (tmux, pane) => tmux("list-panes", "-a", "-F", "#{pane_id}").split("\n").map((l) => l.trim()).includes(pane);

/** AC-138: tasks chosen and dropped on a free agent, one session; busy refuses more; it stays on their cards. */
export async function agentPool(page, opts, pressAt) {
  const { base, work, tmux, shots } = opts;
  const { j, card, cli, newPane, envOf, newTask } = boardKit(page, opts);
  const project = (await j("/api/task-projects", "POST", { name: "Pool", dir: work })).id;
  const fixer = (await j("/api/task-agents", "POST", { project, name: "Fast fixer", kind: "claude", effort: "high", command: `python3 -I ${FAKE} ${cli} --kind claude` })).id;
  const task = async (title) => (await j("/api/tasks", "POST", { title, project })).id;
  const a = await task("first of two"), b = await task("second of two"), c = await task("a third");
  const status = async (id) => (await j(`/api/tasks/${id}`)).status;
  const hasStatus = (id, s, ms = 15000) => until(async () => await status(id) === s, ms);
  const paneOf = async (id) => (await j(`/api/tasks/${id}`)).session?.key?.split(":").pop();
  // a task window's close goes Back: its address gone first, or the load is aborted (net::ERR_ABORTED)
  const shown = async () => {
    await page.waitForURL((u) => !/^\/kanban\/[A-Z]/.test(u.pathname), { timeout: 8000 });
    await page.goto(`${base}/kanban/?project=${project}`);
    await page.waitForSelector(card(a));
  };
  const pool = `#agentbar .agentcard[data-agent="${fixer}"]`;
  const dots = () => page.$$eval(`${pool} .agdot`, (l) => l.map((d) => `${d.className} ${d.title}`));
  const dotted = async (test) => until(async () => test(await dots()), 15000);
  await shown();

  const said = await page.textContent(pool);
  check("AC-138 a rectangle shows the agent's name, kind and effort", /Fast fixer/.test(said) && /Claude Code/.test(said) && /high/.test(said), said);
  check("AC-138 and no dot: nothing of it runs", (await dots()).length === 0, JSON.stringify(await dots()));
  await subscriptionStripes(page, { pool, cli, newPane, envOf, tmux, shots, home: opts.home });
  check("AC-138 a card names no project when one is chosen", !(await page.$(`${card(a)} .cproj`)));
  await page.click(`${card(a)} .ctitle`);           // a folded TODO card shown whole (terminum-kanban.md AC-159)
  check("AC-138 a task on no agent shows its owner", await page.isVisible(`${card(a)} .cowner`) && !(await page.$(`${card(a)} .cagent`)));

  // ⌘ or Ctrl+click chooses; Esc lets them go
  await page.click(`${card(a)} .ctitle`, { modifiers: ["Meta"] });
  await page.click(`${card(b)} .ctitle`, { modifiers: ["Meta"] });
  check("AC-138 ⌘+click chooses tasks, no window opens", await page.locator(".card.picked").count() === 2 && !(await page.$("#taskbox[open]")));
  check("AC-138 a line says what to do with them", /2 chosen/.test(await page.textContent(".pickhint")) && await page.isVisible(".pickhint"), await page.textContent(".pickhint"));
  await page.keyboard.press("Escape");
  check("AC-138 Esc lets them go", await page.locator(".card.picked").count() === 0 && !(await page.isVisible(".pickhint")));
  await page.click(`${card(a)} .ctitle`, { modifiers: ["Meta"] });
  await page.click(`${card(b)} .ctitle`, { modifiers: ["Meta"] });

  // dropped on a free agent: its window, one press
  await page.dragAndDrop(card(a), pool);
  check("AC-138 two tasks dropped on a free agent open the run window", await until(() => page.$("#runbox[open]")));
  const head = await page.textContent("#runhead");
  check("AC-138 its title names both and the agent", head.includes(a) && head.includes(b) && head.endsWith("on Fast fixer"), head);
  check("AC-138 no agent to choose: it was dropped on one", !(await page.isVisible("#runagent")));
  check("AC-138 what will be sent stands open for several", await page.evaluate(() => document.querySelector("#runwhat").open) && /The human ran 2 tasks: /.test(await page.textContent("#runprompt")), await page.textContent("#runprompt"));
  const [p, m] = await page.evaluate(() => ["runprompt", "runmessage"].map((id) => { const s = getComputedStyle(document.getElementById(id)); return { shadow: s.boxShadow, inset: s.paddingLeft, size: s.fontSize }; }));
  check("AC-138 what will be sent is a read-only field: a field's well, its text as far in and as large as the message's", p.shadow !== "none" && p.inset === m.inset && p.size === m.size, JSON.stringify([p, m]));
  check("AC-121 where and how stays folded, filled", !(await page.evaluate(() => document.querySelector("#runhow").open)) && await page.textContent("#runfolder") === work && (await page.textContent("#runcommand")).includes("fake_agent.py"));
  if (shots) await page.screenshot({ path: path.join(shots, "run-several.png") });
  await page.fill("#runmessage", "WORKS");
  await page.press("#runmessage", "Meta+Enter");
  check("AC-138 ⌘Enter in the message runs", await until(async () => !(await page.$("#runbox[open]"))));
  check("AC-138 both go IN PROGRESS", await hasStatus(a, "IN PROGRESS") && await hasStatus(b, "IN PROGRESS"), `${await status(a)} ${await status(b)}: ${(await j(`/api/tasks/${a}`)).history?.at(-1)?.text}`);
  check("AC-183 a run asked for by a card dropped on an agent keeps the human and the drop", startOf(opts.app, a) === "human drop", startOf(opts.app, a));
  const key = (await j(`/api/tasks/${a}`)).session?.key;
  check("AC-138 in one session", !!key && (await j(`/api/tasks/${b}`)).session?.key === key, key);
  check("AC-138 what was chosen is let go", await until(async () => await page.locator(".card.picked").count() === 0));
  check("AC-138 one dot blinks for the session, saying both", await dotted((d) => d.length === 1 && /agdot on/.test(d[0]) && d[0].includes(a) && d[0].includes(b)), JSON.stringify(await dots()));
  check("AC-122 a task in progress keeps its play key and names the agent", await until(async () => !!(await page.$(`${card(a)} .crun`)) && (await page.textContent(`${card(a)} .cagent`).catch(() => "")) === "Fast fixer"));
  check("AC-138 and hides the owner", !(await page.$(`${card(a)} .cowner`)));

  // busy: a third is offered the queue (AC-169) and, agreed, waits for its room (AC-158), said in RUN's window and by terminumd
  await page.click(`${card(c)} .ctitle`, { modifiers: ["Meta"] });
  await page.click(pool);
  check("AC-158 an agent at its limit takes a third to wait: said in its window", await until(async () => /Fast fixer is at its limit of 1.*Add it to the queue/.test(await page.textContent("#runnone").catch(() => ""))), await page.textContent("#runnone").catch(() => ""));
  await page.click('#runbox button[value="cancel"]');
  await page.keyboard.press("Escape");
  const waits = await j(`/api/tasks/${c}/run`, "POST", { agent: fixer, queue: true });
  check("AC-158 terminumd keeps it waiting, naming what holds the slot", /It waits for Fast fixer: Fast fixer is at its limit of 1/.test(waits.waits) && waits.waits.includes(a), JSON.stringify(waits));
  check("AC-158 its card says what it waits for", await until(async () => /waits for Fast fixer/.test(await page.textContent(card(c)).catch(() => ""))));
  const folded = await page.$eval(card(c), (k) => { const s = k.querySelector(".cstate.waiting"), r = s?.getBoundingClientRect();
    return { folded: k.matches(".compact:not(.open)"), dot: !!r && r.width >= 7 && r.width < 20, title: s?.title ?? "" }; });
  check("AC-159 a folded card shows the waiting dot, its words on hover", folded.folded && folded.dot && /waits for Fast fixer/.test(folded.title), JSON.stringify(folded));
  const again = await j(`/api/tasks/${c}/run`, "POST", { agent: fixer, queue: true });
  check("AC-158 a press again on that agent waits on, in its place", /It waits for Fast fixer/.test(again.waits), JSON.stringify(again));
  await j(`/api/tasks/${c}/give-up`, "POST");
  check("AC-158 Stop waiting calls it off and the third stays", await status(c) === "TODO" && !(await j(`/api/tasks/${c}`)).waits, JSON.stringify(await j(`/api/tasks/${c}`)));

  // a press on a dot shows its session; on the agent, its window
  await page.click(pool, { position: { x: 60, y: 8 } });
  check("AC-138 a press on the agent opens its window", await until(() => page.$("#agentbox[open]")));
  await page.click('#agentbox button[value="cancel"]');
  await page.click(`${pool} .agdot`);
  await page.waitForURL("**/sessions/**", { timeout: 8000 });
  check("AC-138 a press on a dot opens its session", await until(() => page.evaluate((k) => document.querySelector("#sessions .item.on")?.dataset.id === k, key), 8000));
  await shown();
  await page.hover(pool);
  await page.click(`${pool} .agedit`);
  check("AC-138 its pencil edits it", await until(() => page.$("#agentbox[open]")) && await page.inputValue("#agentname") === "Fast fixer" && await page.inputValue("#agenteffort") === "high");
  await page.click('#agentbox button[value="cancel"]');

  // both finish, in the words its prompt of several names them; they stay on that agent, its session idle
  const pane = await paneOf(a);
  tmux("send-keys", "-t", pane, "-l", `The human ran 2 tasks: ${a}, ${b}. FINISHES`);
  tmux("send-keys", "-t", pane, "Enter");
  check("AC-138 both finish", await hasStatus(a, "DONE") && await hasStatus(b, "DONE"), `${await status(a)} ${await status(b)}`);
  check("AC-138 and stay on that agent", await until(async () => await page.textContent(`.col[data-status="DONE"] ${card(a)} .cagent`).catch(() => "") === "Fast fixer" && await page.textContent(`.col[data-status="DONE"] ${card(b)} .cagent`).catch(() => "") === "Fast fixer"));
  check("AC-138 its session stays, its dot still", await dotted((d) => d.length === 1 && /agdot idle/.test(d[0])), JSON.stringify(await dots()));
  await pressCard(page, a);
  await page.waitForSelector("#taskbox[open]");
  check("AC-138 a task's window names its agent", await until(() => page.isVisible("#taskbody .fieldvalue .cagent"), 2000), await page.textContent("#taskbody .fieldvalue"));
  await page.click("#taskclose");
  await page.waitForFunction(() => /^\/kanban\/$/.test(location.pathname));

  // at its limit with an idle holder: one press closes it and runs
  const offered = async () => (await j(`/api/tasks/${c}/run`)).agents?.find((x) => x.id === fixer)?.full?.close;
  check("AC-141 once two walks are quiet the idle holder can be closed", await until(offered, 15000), JSON.stringify(await offered()));
  const idlePane = await paneOf(a);
  tmux("send-keys", "-t", idlePane, "-l", "a draft in its box");
  await pressRun(page, c);
  await page.waitForSelector("#runbox[open]");
  check("AC-141 the window says what holds it and names the key", /at its limit of 1: [^;]+ (is|are) idle\./.test(await page.textContent("#runnone")) && /Close .*'s session and run/.test(await page.textContent("#rungo")), await page.textContent("#rungo"));
  await page.fill("#runmessage", "WORKS");
  await page.click("#rungo");
  check("AC-141 a draft in its box stops it: said, nothing closed", await until(async () => /no longer idle/.test(await page.textContent("#runerror"))) && paneAlive(tmux, idlePane), await page.textContent("#runerror"));
  await page.click('#runbox button[value="cancel"]');
  tmux("send-keys", "-t", idlePane, "C-u");
  // pressed only once the box is drawn empty: a screen read before the agent redraws still shows the draft
  check("AC-141 Ctrl+U empties its box", await until(() => !tmux("capture-pane", "-p", "-t", idlePane).includes("a draft in its box")), tmux("capture-pane", "-p", "-t", idlePane));
  await pressRun(page, c);
  await page.waitForSelector("#runbox[open]");
  await page.fill("#runmessage", "WORKS");
  await page.click("#rungo");
  check("AC-141 with its box empty it closes the holder", await until(() => !paneAlive(tmux, idlePane), 15000));
  const ends = async () => (await j(`/api/tasks/${a}`)).history.map((e) => e.text).filter((t) => /^Session ended/.test(t ?? ""));
  // "closed 5 minutes after DONE" instead: AC-125's close came first, the press too late for TM_TEST_CLOSE_AFTER
  check("AC-141 and says so on its task", await until(async () => (await ends()).some((t) => t.startsWith("Session ended: closed for"))), JSON.stringify(await ends()));
  check("AC-141 and the third starts in the slot", await hasStatus(c, "IN PROGRESS"), await status(c));
  check("AC-138 a dot for it", await dotted((d) => d.length === 1 && d[0].includes(c)), JSON.stringify(await dots()));
  tmux("kill-pane", "-t", await paneOf(c));
  check("AC-138 closing a session frees its slot", await dotted((d) => d.length === 0), JSON.stringify(await dots()));

  await newTask();
  await page.waitForSelector("#newbox[open]");
  check("AC-106 a new task leaves its agent unspecified despite earlier runs", await page.inputValue("#newagent") === "", await page.inputValue("#newagent"));
  await page.selectOption("#newagent", fixer);
  await page.fill("#newtitle", "made and run FINISHES");
  await page.click("#newrun");
  let made;
  await until(async () => (made = (await j(`/api/tasks?q=${encodeURIComponent("made and run")}`)).tasks?.[0]?.id));
  check("AC-138 Run creates it and starts it at once on that agent", !!made && await hasStatus(made, "DONE") && await until(async () => await page.textContent(`${card(made)} .cagent`).catch(() => "") === "Fast fixer"), made);
  check("AC-138 no window on the way", !(await page.$("#runbox[open]")));
  check("AC-183 the new-task window's Run keeps the human and its way", startOf(opts.app, made) === "human new", startOf(opts.app, made));
  tmux("kill-pane", "-t", await paneOf(made));
  await dotted((d) => d.length === 0);
  await newTask();
  await page.waitForSelector("#newbox[open]");
  await page.fill("#newtitle", "made, its agent not chosen");
  check("AC-106 another new task defaults to choosing at assignment", await page.inputValue("#newagent") === "");
  // a window opened and closed while it is open: the page reads the sessions again, the choice stays
  tmux("kill-window", "-t", tmux("new-window", "-d", "-P", "-F", "#{window_id}").trim());
  check("AC-106 none picked stays none as the windows change", !(await until(async () => await page.inputValue("#newagent") !== "", 2000)), await page.inputValue("#newagent"));
  await page.click("#newrun");
  check("AC-138 with none chosen it opens the run window", await until(() => page.$("#runbox[open]")));
  await page.click('#runbox button[value="cancel"]');

  // Archive hides it, Archived shows it, Bring back returns it
  const shelved = await task("not needed");
  await page.waitForSelector(card(shelved), { timeout: 5000 });
  await pressCard(page, shelved);
  await page.waitForSelector("#taskbox[open]");
  await page.click('#taskacts button:has-text("Archive")');
  check("AC-118 Archive turns its key to Bring back", await until(() => page.isVisible('#taskacts button:has-text("Bring back")'), 2000));
  await page.click("#taskclose");
  check("AC-118 an archived task leaves the board", await until(async () => !(await page.$(card(shelved)))));
  await page.click("#farchived");
  check("AC-118 Archived shows it, in the address", await until(() => page.isVisible(card(shelved))) && new URL(page.url()).searchParams.get("archived") === "1", page.url());
  await pressCard(page, shelved);
  await page.waitForSelector("#taskbox[open]");
  const back = await until(() => page.isVisible('#taskacts button:has-text("Bring back")'), 2000);
  check("AC-118 an archived task's window offers Bring back", back);
  // with no such key the route brings it back, so what follows still runs
  if (back) await page.click('#taskacts button:has-text("Bring back")');
  else await j(`/api/tasks/${shelved}/archive`, "POST", { on: false });
  await until(() => page.isVisible('#taskacts button:has-text("Archive")'));
  await page.click("#taskclose");
  await page.waitForFunction(() => /^\/kanban\/$/.test(location.pathname));
  await page.click("#farchived");
  check("AC-118 Bring back returns it to its column", await until(() => page.isVisible(`.col[data-status="TODO"] ${card(shelved)}`)), page.url());

  // Give up: a run that holds its tasks lets go, they fail, its window stays
  const held = await task("given up one"), held2 = await task("given up two");
  await page.waitForSelector(card(held), { timeout: 5000 });
  const ran = await j(`/api/tasks/${held}/run`, "POST", { agent: fixer, comment: "WORKS", also: [held2] });
  check("AC-138 a press of several by the route starts", !!ran.run, JSON.stringify(ran));
  await hasStatus(held, "IN PROGRESS");
  const heldPane = await paneOf(held);
  // its window: the session's tasks as links (AC-138)
  await page.click(pool, { position: { x: 60, y: 8 } });
  await page.waitForSelector("#agentbox[open]");
  const view = async () => page.textContent("#agentview");
  check("AC-138 its window lists the session's tasks under Working on, each a link", await until(async () => /Working on/.test(await view()) && await page.isVisible(`#agentview .agtask .stbox:has-text("${held2}")`)), await view());
  check("AC-143 its tokens are in its window, not on its rectangle", /Tokens: week/.test(await view()) && !/Tokens/.test(await page.textContent(pool)), await page.textContent(pool));
  if (shots) await page.screenshot({ path: path.join(shots, "agent-window-tasks.png") });
  await page.click(`#agentview .agtask button.linklike >> nth=1`);
  check("AC-138 a task's link opens that task, the agent's window closed", await until(() => page.$("#taskbox[open]")) && !(await page.$("#agentbox[open]")));
  check("AC-138 the task it named", await until(async () => new RegExp(held2).test(page.url())), page.url());
  await page.click("#taskclose");
  await shown();
  await pressCard(page, held);
  await page.waitForSelector("#taskbox[open]");
  check("AC-122 a task its run holds offers Give up and Run", await until(() => page.isVisible('#taskacts button:has-text("Give up")')) && await page.isVisible('#taskacts button:has-text("Run")'));
  await page.click('#taskacts button:has-text("Give up")');
  await pressAt(page, "#confirmok");
  check("AC-138 Give up fails its open tasks", await hasStatus(held, "FAILED") && await hasStatus(held2, "FAILED"), `${await status(held)} ${await status(held2)}`);
  check("AC-138 its window stays", paneAlive(tmux, heldPane));
  await page.click("#taskclose");
  check("AC-141 giving up frees its slot", await dotted((d) => d.length === 0), JSON.stringify(await dots()));
  tmux("kill-pane", "-t", heldPane);

  // + Session: the agent in the project's place, nothing typed, a dot of its own, let go by the human (AC-142)
  await shown();
  const under = await page.evaluate(() => { const k = document.getElementById("adhocsession"), rows = [...document.querySelectorAll("#sessions .item")];
    return { inList: k?.parentElement?.id === "sessions" && k === k.parentElement.lastElementChild, below: rows.every((r) => r.getBoundingClientRect().bottom <= k.getBoundingClientRect().top), rows: rows.length }; });
  check("AC-142 one project shown: + Session under its sessions in the list", await page.isVisible("#adhocsession") && under.inList && under.below && !(await page.$("#boardacts #adhocsession")), JSON.stringify(under));
  const spare = (await j("/api/task-agents", "POST", { project, name: "Spare", kind: "claude", command: "claude" })).id;   // a pool of two: it asks which
  await until(() => page.isVisible('.agents .agentcard:has-text("Spare")'));
  await page.click("#adhocsession");
  check("AC-142 with several agents it asks which, the last used first", await until(() => page.$("#confirmbox[open]")) && /Fast fixer/.test(await page.textContent("#confirmacts")), await page.textContent("#confirmacts").catch(() => ""));
  await pressAt(page, '#confirmacts button:has-text("Fast fixer")');
  check("AC-142 it opens the session, of no task", await until(() => /\/sessions\//.test(page.url()), 15000), page.url());
  await j(`/api/task-agents/${spare}`, "DELETE");
  let adhoc;
  check("AC-142 its agent shows a dot for it, the pool's slot taken", await until(async () => (adhoc = (await j("/api/tasks?counts=1")).slots?.agents?.[fixer]?.find((s) => !s.tasks)), 15000), JSON.stringify(adhoc));
  check("AC-142 nothing is typed: it stands working, the human's", await until(async () => (await j("/api/tasks?counts=1")).slots?.agents?.[fixer]?.find((s) => !s.tasks)?.state === "working", 15000));
  check("AC-183 + Session's run keeps the human and its way", startOf(opts.app) === "human session", startOf(opts.app));
  const adPane = adhoc?.key?.split(":").pop();
  check("AC-140 it opened in the project's folder", adPane && tmux("display", "-p", "-t", adPane, "#{pane_current_path}").trim().endsWith(path.basename(work)), adPane);
  await shown();
  await page.click(pool, { position: { x: 60, y: 8 } });
  await page.waitForSelector("#agentbox[open]");
  check("AC-142 its agent's window lists it, with Let go", await until(() => page.isVisible('#agentview button:has-text("Let go")')));
  await page.click('#agentview button:has-text("Let go")');
  await pressAt(page, "#confirmok");
  check("AC-142 Let go frees its slot", await until(async () => !(await j("/api/tasks?counts=1")).slots?.agents?.[fixer]?.length, 15000));
  check("AC-142 and leaves its window", adPane && paneAlive(tmux, adPane));
  await page.click('#agentbox button[value="cancel"]');
  if (adPane) tmux("kill-pane", "-t", adPane);
  const noPlace = (await j("/api/task-projects", "POST", { name: "NoPlace" })).id;
  const homeless = await j(`/api/task-projects/${noPlace}/session`, "POST", { agent: fixer });
  check("AC-142 a project with no folder refuses it, said", /NoPlace has no folder/.test(homeless.error ?? ""), JSON.stringify(homeless));

  // two at once, and a project's own limit, whichever agent runs
  const line = (await j("/api/tasks/meta")).agents.find((x) => x.id === fixer);
  await j(`/api/task-agents/${fixer}`, "PATCH", { name: line.name, kind: line.kind, command: line.command, effort: line.effort, at_once: 2 });
  const one = await task("first at once"), two = await task("second at once"), three = await task("third at once");
  await page.waitForSelector(card(three), { timeout: 8000 });
  await j(`/api/tasks/${one}/run`, "POST", { agent: fixer });
  await j(`/api/tasks/${two}/run`, "POST", { agent: fixer });
  check("AC-141 two at once start", await hasStatus(one, "IN PROGRESS") && await hasStatus(two, "IN PROGRESS"), `${await status(one)} ${await status(two)}`);
  check("AC-138 a dot each", await dotted((d) => d.length === 2 && d[0].includes(one) && d[1].includes(two)), JSON.stringify(await dots()));
  check("AC-138 and it counts them", await until(async () => (await page.textContent(`${pool} .agcount`).catch(() => "")) === "2/2"));
  const third = await j(`/api/tasks/${three}/run`, "POST", { agent: fixer, queue: true });
  check("AC-158 a third waits, naming both", /at its limit of 2/.test(third.waits) && third.waits.includes(one) && third.waits.includes(two), JSON.stringify(third));
  await j(`/api/tasks/${three}/give-up`, "POST");
  const other = (await j("/api/task-agents", "POST", { project, name: "Other one", kind: "claude", command: `python3 -I ${FAKE} ${cli} --kind claude` })).id;
  check("AC-141 Agents at once takes 1 to 100 or nothing", /1 to 100/.test((await j(`/api/task-projects/${project}/agents-at-once`, "PUT", { limit: 0 })).error));
  await j(`/api/task-projects/${project}/agents-at-once`, "PUT", { limit: 2 });
  const limited = await j(`/api/tasks/${three}/run`, "POST", { agent: other, queue: true });
  check("AC-141 a project's limit counts every agent", /Pool runs 2 agents at once, its limit/.test(limited.waits), JSON.stringify(limited));
  await j(`/api/tasks/${three}/give-up`, "POST");
  await j(`/api/task-projects/${project}/agents-at-once`, "PUT", { limit: null });
  check("AC-141 no limit: another agent starts", !!(await j(`/api/tasks/${three}/run`, "POST", { agent: other, how: "auto" })).run);
  check("AC-183 a page's press is the human's Run, whatever way it names", startOf(opts.app, three) === "human run" && startOf(opts.app, one) === "human run", startOf(opts.app, three));
  await hasStatus(three, "IN PROGRESS");
  for (const id of [one, two, three]) tmux("kill-pane", "-t", await paneOf(id));
  check("AC-141 and their slots free", await dotted((d) => d.length === 0), JSON.stringify(await dots()));
  await j(`/api/task-agents/${other}`, "DELETE");
}
