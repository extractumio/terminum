// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A task's Details against a real private terminumd (e2e_live.mjs; terminum-kanban.md AC-199): its type shown as the
// title's word, an external link, a target, and Blocked by and Follow-ups chosen by search, in the task's window and
// in New task's. `node web/test/e2e_live.mjs tasks chromium` runs it with the tasks' suites.
import { boardKit, tabKey } from "./boardkit.mjs";
import { check, until } from "./check.mjs";

export async function liveBoardAttrs(page, opts) {
  const { j, card, open } = boardKit(page, opts);
  const project = (await j("/api/task-projects", "POST", { name: `Details ${page.viewportSize().width}` })).id;
  const make = async (title, more = {}) => j("/api/tasks", "POST", { title, project, ...more });
  const a = await make("Bug: drops the last line");
  check("AC-199 a title's type word is its type: the title is kept without it, shown with it", a.type === "Bug" && a.name === "drops the last line" && a.title === "Bug: drops the last line", JSON.stringify(a));
  const b = (await make("the column first", { type: "Feature" })).id, c = (await make("then the docs")).id;
  const conflict = await make("Bug: two words", { type: "Debt" });
  check("AC-199 a type that disagrees with the title's word is refused, said", conflict.code === 400 && /The title says Bug: and Type says Debt/.test(conflict.error), JSON.stringify(conflict));
  await open(`?project=${project}`);
  check("AC-199 the card shows the title with its type's word", await until(async () => (await page.textContent(`${card(b)} .ctitle`)) === "Feat: the column first"));

  // the task's window: Details folded until opened
  await page.goto(`${opts.base}/kanban/${a.id}`);
  await page.waitForSelector("#taskbox[open] .taskattrs");
  const pane = "#taskbox .taskattrs";
  check("AC-199 Details are folded, the summary saying what is set", !(await page.$eval(pane, (d) => d.open)) && (await page.textContent(`${pane} > summary`)) === "Details: Bug");
  await page.click(`${pane} > summary`);
  const api = () => j(`/api/tasks/${a.id}`);
  await page.selectOption(`${pane} select`, "Story");
  check("AC-199 Type changes the task's type and the word its title is shown with", await until(async () => (await api()).type === "Story") && await until(async () => (await page.textContent("#tasktitle")) === `${a.id} Story: drops the last line`));
  check("AC-199 Details stay open as the window is drawn anew", await page.$eval(pane, (d) => d.open));
  const link = `${pane} input[type=url]`;
  await page.fill(link, "https://tracker.example/browse/API-7");
  await page.focus(link); await tabKey(page);
  check("AC-199 an external link is kept", await until(async () => (await api()).external_link === "https://tracker.example/browse/API-7"));
  check("AC-199 its Open opens it in a new tab, never in this page", await page.$eval(`${pane} .attrgo`, (g) => !g.hidden && g.target === "_blank" && g.rel.includes("noopener") && g.href === "https://tracker.example/browse/API-7"));
  await page.fill(link, "javascript:alert(1)");
  await page.focus(link); await tabKey(page);
  check("AC-199 a link that is no web address is refused and the field goes back", await until(async () => (await page.inputValue(link)) === "https://tracker.example/browse/API-7") && (await api()).external_link === "https://tracker.example/browse/API-7");
  const target = `${pane} input[placeholder^="A release"]`;
  await page.fill(target, "v0.32.0");
  await page.focus(target); await tabKey(page);
  check("AC-199 a target is kept", await until(async () => (await api()).target === "v0.32.0"));

  // Blocked by: + opens a search over the project's tasks; Enter takes the one marked
  await page.click(`${pane} button[aria-label="Add a task to Blocked by"]`);
  const search = `${pane} input[type=search]`;
  await page.waitForSelector(search);
  check("AC-199 the search has the focus", await until(() => page.$eval(search, (f) => document.activeElement === f)));
  await page.fill(search, "column");
  await page.waitForFunction((p) => [...document.querySelectorAll(`${p} .attrfind`)].length === 1, pane);
  check("AC-199 the search finds the project's tasks by their words, never the task itself", (await page.textContent(`${pane} .attrfind .attrid`)) === b);
  await page.press(search, "Enter");
  check("AC-199 Enter puts the task found in Blocked by", await until(async () => (await api()).lists.blocked_by[0]?.id === b));
  await page.waitForSelector(`${pane} .attrrow`);
  const reason = `${pane} .attrrow .attrreason`;
  await page.fill(reason, "needs the column");
  await page.focus(reason); await tabKey(page);
  check("AC-199 a reason is kept beside its task", await until(async () => (await api()).lists.blocked_by[0]?.reason === "needs the column"));
  await page.click(`${pane} button[aria-label="Add a task to Follow-ups"]`);
  await page.fill(search, "docs");
  await page.click(`${pane} .attrfind`);
  check("AC-199 a press on a task found puts it in Follow-ups", await until(async () => (await api()).lists.follow_ups[0]?.id === c));
  await page.click(`${pane} button[aria-label="Take ${c} out of Follow-ups"]`);
  check("AC-199 × takes it out", await until(async () => !(await api()).lists.follow_ups.length));
  const ring = await j(`/api/tasks/${b}`, "PATCH", { link: { list: "blocked_by", task: a.id } });
  check("AC-199 a task never waits for one that waits for it", ring.code === 400 && /waits for this task already/.test(ring.error), JSON.stringify(ring));
  // the window drawn anew after × (one line left), then one press: a read of this task still on its way when the
  // listed one opens draws nothing over it (AC-133)
  await page.waitForFunction((p) => document.querySelectorAll(`${p} .attrrow`).length === 1, pane);
  await page.click(`${pane} .attrrow .linklike`);
  check("AC-199 a listed task's ID opens its window", await until(async () => (await page.textContent("#tasktitle")).startsWith(b)), await page.textContent("#tasktitle"));
  await page.click("#taskclose");

  // New task: the same Details, sent with Create
  await page.goto(`${opts.base}/kanban/?project=${project}`);
  await page.click("#newtask");
  await page.waitForSelector("#newbox[open]");
  if (await page.inputValue("#newproj") !== project) await page.selectOption("#newproj", project);
  await page.fill("#newtitle", "the next step");
  const fresh = "#newattrs .taskattrs";
  check("AC-199 New task's Details are folded", !(await page.$eval(fresh, (d) => d.open)));
  const offered = await page.$$eval("#newtype option", (o) => o.map((x) => x.textContent));
  check("AC-199 New task's Type stands beside Severity, none chosen, offering every type", await page.isVisible("#newtype") && (await page.inputValue("#newtype")) === ""
    && JSON.stringify(offered) === JSON.stringify(["None", "Feature", "Bug", "Debt", "Story"]), JSON.stringify(offered));
  await page.selectOption("#newtype", "Debt");
  await page.click(`${fresh} > summary`);
  check("AC-199 New task's Details hold no second Type", !(await page.$(`${fresh} select`)) && (await page.textContent(`${fresh} > summary`)) === "Details");
  await page.click(`${fresh} button[aria-label="Add a task to Blocked by"]`);
  await page.fill(`${fresh} input[type=search]`, "drops");
  await page.waitForSelector(`${fresh} .attrfind`);
  await page.press(`${fresh} input[type=search]`, "Enter");
  await page.waitForSelector(`${fresh} .attrrow`);
  check("AC-199 Enter in the search picks, never creates the task", await page.isVisible("#newbox[open]"));
  await page.click("#newcreate");
  await page.waitForSelector("#newbox", { state: "hidden" });
  const made = (await j(`/api/tasks?project=${project}&q=next`)).tasks[0];
  const kept = made && await j(`/api/tasks/${made.id}`);
  check("AC-199 Create keeps the type and the lists chosen", kept?.title === "Debt: the next step" && kept.lists.blocked_by[0]?.id === a.id, JSON.stringify(kept?.lists));
  await page.click("#newtask");
  await page.waitForSelector("#newbox[open]");
  check("AC-199 the next new task's Type is none chosen again", (await page.inputValue("#newtype")) === "");
  await page.click("#newclose");
}
