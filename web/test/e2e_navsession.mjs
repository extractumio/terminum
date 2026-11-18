// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum-dispatcher.md AC-142 against a fake terminumd (fake.mjs): the session list's head key is [+ Session], the
// board's text key, its word from a 400 px list, the plus alone below; it starts an agent's session in the project in
// view (the session shown, the one project the board shows), else a menu of the projects opens; a phone has none.
import { check, runningIn, until } from "./check.mjs";
import { drawn, fakeBoard, ORIGIN, openPage, PHONE, WIDE } from "./fake.mjs";

const ID = (n) => `tmux:local:terminum:%${n}`;
const PROJECTS = [{ id: "pa", name: "API", color: 0, host: null, dir: "/workspace/api", members: [ID(1)] },
  { id: "pb", name: "Docs", color: 3, host: null, dir: "/workspace/docs", members: [] }];
const META = { projects: PROJECTS.map(({ members, ...p }) => p), users: [], sessions: [], hosts: [], last_agents: {},
  agents: [{ id: "a1", project: "pa", name: "Claude", kind: "claude", color: 0, line: "claude", at_once: 2 },
    { id: "a2", project: "pb", name: "Codex", kind: "codex", color: 1, line: "codex", at_once: 1 }] };
const json = (r, data, status = 200) => r.fulfill({ status, contentType: "application/json", body: JSON.stringify(data) });

const key = (page) => page.evaluate(() => {
  const k = document.getElementById("navnew"), r = k.getBoundingClientRect(), head = document.querySelector(".nav-head").getBoundingClientRect();
  const word = k.querySelector(".word"), tops = [...document.querySelector(".nav-head").children].filter((c) => c.offsetWidth).map((c) => Math.round(c.getBoundingClientRect().top));
  return { shown: !!k.offsetWidth, w: Math.round(r.width), h: Math.round(r.height), word: !!word?.offsetWidth, name: k.getAttribute("aria-label"),
    textkey: k.classList.contains("textkey"), oneRow: new Set(tops).size === 1 && head.height < 60 };
});

/** Starts asked of terminumd, its answer `answer(project)`. */
async function sessions(page, answer = (p) => ({ key: p === "pa" ? ID(1) : ID(9) })) {
  const posts = [];
  await page.route("**/api/task-projects/*/session", (r) => {
    const p = new URL(r.request().url()).pathname.split("/")[3];
    posts.push({ project: p, body: JSON.parse(r.request().postData() || "{}") });
    return json(r, answer(p));
  });
  return posts;
}

export async function webNavSession(browser, name) {
  // the session page, wide
  let at = `(${name} session page)`;
  runningIn(`${name} 1400×900`);
  let f = await openPage(browser, { device: WIDE });
  let { page } = f;
  await drawn(page);
  await page.route("**/api/tasks/meta", (r) => json(r, META));
  await page.route("**/api/tasks?*", (r) => json(r, { tasks: [], counts: {}, slots: { agents: {}, full: [], full_projects: [] } }));
  const posts = await sessions(page);
  f.say({ t: "projects", items: PROJECTS, order: [] });
  await page.evaluate(() => document.documentElement.style.setProperty("--nav-w", "300px"));
  const narrow = await key(page);
  check(`AC-142 ${at} a 300 px list: the plus alone at 30×30, named "New agent session", the head on one row`,
    narrow.shown && narrow.textkey && !narrow.word && narrow.w === 30 && narrow.h === 30 && narrow.name === "New agent session" && narrow.oneRow, JSON.stringify(narrow));
  await page.evaluate(() => document.documentElement.style.setProperty("--nav-w", "420px"));
  const wide = await key(page);
  check(`AC-142 ${at} a 420 px list: [+ Session] as the board's text key, 30 px high, the head on one row`,
    wide.shown && wide.word && wide.h === 30 && wide.w > 60 && wide.oneRow, JSON.stringify(wide));
  // the session shown (%1) is API's, one agent free there: it starts at once, no form, no menu
  await until(() => page.evaluate(() => !!document.querySelector("#sessions .item")));
  await page.click("#navnew");
  check(`AC-142 ${at} the shown session's project with one agent free: terminumd asked for that agent's session, the page goes to it`,
    await until(() => posts.length === 1) && posts[0].project === "pa" && posts[0].body.agent === "a1" && await until(() => page.url().startsWith(`${ORIGIN}/sessions/`)),
    JSON.stringify({ posts, url: page.url() }));
  check(`AC-142 ${at} the tmux session's form never opens from the head`, await page.evaluate(() => document.getElementById("newsess").hidden), "");
  check(`no page error ${at}`, f.errors.length === 0, f.errors.join("; "));
  await f.close();

  // no project in view (the session shown is in none): a menu of the projects; Esc closes it, the focus back on the key
  at = `(${name} no project in view)`;
  f = await openPage(browser, { device: WIDE });
  ({ page } = f);
  await drawn(page);
  await page.route("**/api/tasks/meta", (r) => json(r, META));
  await page.route("**/api/tasks?*", (r) => json(r, { tasks: [], counts: {}, slots: { agents: {}, full: [], full_projects: [] } }));
  const later = await sessions(page);
  f.say({ t: "projects", items: PROJECTS.map((p) => ({ ...p, members: [] })), order: [] });
  await page.focus("#navnew");
  await page.keyboard.press("Enter");
  const menu = await until(() => page.evaluate(() => document.getElementById("rowmenu").open)) && await page.evaluate(() => ({
    head: document.querySelector("#rowmenu .mhead span")?.textContent, rows: [...document.querySelectorAll('#rowmenu [data-action="join"]')].map((b) => b.textContent),
    dots: document.querySelectorAll("#rowmenu .picon").length, expanded: document.getElementById("navnew").getAttribute("aria-expanded") }));
  check(`AC-142 ${at} a menu "A session of an agent in", a row per project with its mark, the key sunk`,
    menu?.head === "A session of an agent in" && JSON.stringify(menu.rows) === '["API","Docs"]' && menu.dots === 2 && menu.expanded === "true", JSON.stringify(menu));
  await page.keyboard.press("Escape");
  const back = await page.evaluate(() => ({ open: document.getElementById("rowmenu").open, focus: document.activeElement?.id, expanded: document.getElementById("navnew").getAttribute("aria-expanded") }));
  check(`AC-142 ${at} Esc closes it, the focus back on the key`, !back.open && back.focus === "navnew" && back.expanded === "false" && later.length === 0, JSON.stringify(back));
  await page.click("#navnew");
  await until(() => page.evaluate(() => document.getElementById("rowmenu").open));
  await page.click('#rowmenu [data-action="join"]:has-text("Docs")');
  check(`AC-142 ${at} a project chosen: its agent's session asked for`, await until(() => later.length === 1) && later[0].project === "pb" && later[0].body.agent === "a2", JSON.stringify(later));
  check(`no page error ${at}`, f.errors.length === 0, f.errors.join("; "));
  await f.close();

  // the board with one project chosen: the head's key is its + Session; the list's own key is the phone's only
  at = `(${name} Kanban)`;
  f = await openPage(browser, { device: WIDE, url: "/kanban/?project=pb" });
  ({ page } = f);
  await fakeBoard(page, { meta: () => META, tasks: () => [], counts: {} });
  await page.goto(`${ORIGIN}/kanban/?project=pb`);
  const board = await sessions(page);
  await page.waitForSelector("#navnew", { state: "visible" });
  f.say({ t: "projects", items: PROJECTS, order: [] });
  const own = await until(() => page.evaluate(() => !!document.getElementById("adhocsession"))) && await page.evaluate(() => !!document.getElementById("adhocsession").offsetWidth);
  check(`AC-142 ${at} the list's own [+ Session] is not shown beside the head's on a computer`, !own, String(own));
  await page.click("#navnew");
  check(`AC-142 ${at} the head's key starts a session in the one project shown`, await until(() => board.length === 1) && board[0].project === "pb", JSON.stringify(board));
  check(`no page error ${at}`, f.errors.length === 0, f.errors.join("; "));
  await f.close();

  // a phone: no head key
  at = `(${name} 390×844)`;
  runningIn(`${name} 390×844`);
  f = await openPage(browser, { device: PHONE });
  await drawn(f.page);
  check(`AC-142 ${at} a phone has no head key`, !(await key(f.page)).shown, "");
  await f.close();
}
