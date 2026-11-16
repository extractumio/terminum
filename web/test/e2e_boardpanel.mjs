// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The board wider than a phone (terminum-kanban.md AC-105 "TODO and the panel", "a tablet"; AC-106's + New task) against
// a fake terminumd: TODO a column of its own, the other columns one at a time in a panel under their tabs, each with its
// count, WAIT FOR HUMAN's and FAILED's pulsing while they hold tasks; the board's tracks by its width; on an iPad the head
// one row of finger-sized keys, nothing cut, every left edge on the gutter or a track.
import fs from "node:fs";
import { check, runningIn, until } from "./check.mjs";
import { drawn, openPage, ORIGIN, pause, shot, THEME } from "./fake.mjs";

const NOW = 1_800_000_000;
const task = (status, i, extra = {}) => ({ id: `API-${status.length}${i}`, title: `${status} task ${i}: a title long enough to be read whole on a tablet`, project: "api", status,
  severity: ["P1", "P2", "P0"][i % 3], owner: "human", owner_name: "Alex", updated_at: NOW - i, parent: null, ...extra });
const CARDS = { TODO: 9, "IN PROGRESS": 4, "WAIT FOR HUMAN": 2, INTEGRATION: 1, FAILED: 1, DONE: 12 };
const tasks = Object.fromEntries(Object.entries(CARDS).map(([s, n]) => [s, Array.from({ length: n }, (_, i) => task(s, i))]));
const counts = { ...CARDS };
const agents = ["Opus", "Sonnet", "Fable", "Codex"].map((name, i) => ({ id: `a${i}`, project: "api", name, kind: i === 3 ? "codex" : "claude", at_once: 2 }));
const meta = (integration) => ({ projects: [{ id: "api", name: "API", color: 0, integration }], users: [], sessions: [], hosts: [], theme: THEME, agents });

/** Where the board's parts stand. */
const look = (page) => page.evaluate(() => {
  const box = (e) => { const r = e?.getBoundingClientRect(); return r && r.width > 0 ? { l: Math.round(r.left), r: Math.round(r.right), t: Math.round(r.top), b: Math.round(r.bottom), w: Math.round(r.width), h: Math.round(r.height) } : null; };
  const q = (s) => document.querySelector(s), all = (s) => [...document.querySelectorAll(s)];
  const tabs = all("#colswitch button").filter((b) => b.offsetWidth).map((b) => ({ s: b.dataset.status, words: b.textContent, alarm: b.classList.contains("alarm"), on: b.getAttribute("aria-selected") === "true",
    pulse: getComputedStyle(b.querySelector(".tabn"), "::after").animationName, ...box(b) }));
  const head = all("#navbtn, #bar > .sections, #fproject summary, #filters .search, #filtersbtn, .askn, #autoswitch, #autocog, #newtask").map((e) => ({ id: e.id || e.className, ...box(e) })).filter((b) => b.w);
  return { win: innerWidth, docW: document.documentElement.scrollWidth, bar: box(q("#bar")), board: box(q("#board")), todo: box(q('.col[data-status="TODO"]')), panel: box(q(".colpanel")),
    todoHead: box(q('.col[data-status="TODO"] .colhead')), tabRow: box(q("#colswitch")), tabs, keys: box(q(".colpanel .colhead")),
    todoCard: box(q('.col[data-status="TODO"] .card')), panelCards: all(".colpanel .col.shown .card").map(box), shown: q(".colpanel .col.shown")?.dataset.status,
    agent: box(q("#agentbar .agentcard")), newKey: box(q('.col[data-status="TODO"] .colhead .new')), newWords: q('.col[data-status="TODO"] .colhead .new')?.textContent,
    sortKeys: all('.col[data-status="TODO"] .colsort .icon').map(box), todoTitle: box(q('.col[data-status="TODO"] .card .ctitle')), gutter: parseFloat(getComputedStyle(q("#board")).paddingLeft),
    head };
});

async function board(browser, device, { integration = { on: true }, motion = "reduce" } = {}) {
  const f = await openPage(browser, { device: { ...device, reducedMotion: motion }, secret: "a".repeat(64), stored: "a".repeat(64) });
  const { page } = f;
  await drawn(page);
  const json = (r, data) => r.fulfill({ contentType: "application/json", body: JSON.stringify(data) });
  await page.route(`${ORIGIN}/kanban/**`, (r) => r.fulfill({ contentType: "text/html", body: fs.readFileSync(new URL("../static/kanban.html", import.meta.url), "utf8").replace("__BUILD__", "test") }));
  await page.route("**/api/tasks/meta", (r) => json(r, meta(integration)));
  await page.route("**/api/schedules", (r) => json(r, { schedules: [] }));
  await page.route("**/api/integration/**", (r) => json(r, { batches: [], projects: [] }));
  await page.route("**/api/tasks?*", (r) => json(r, { tasks: tasks[new URL(r.request().url()).searchParams.get("status")] ?? [], counts, slots: { agents: {}, full: [], full_projects: [] } }));
  const go = async () => {
    await page.goto(`${ORIGIN}/kanban/?project=api`);
    await page.waitForSelector(".col .card");
    await page.waitForFunction(() => document.body.dataset.socket === "open");
    await page.evaluate(() => { if (!document.getElementById("app").classList.contains("nav-hidden") && innerWidth > 860) document.getElementById("navclose").click(); });
    await pause(page, 100);
  };
  return { f, page, go };
}

const near = (a, b, d = 1) => Math.abs(a - b) <= d;

/** A computer's window: TODO two tracks of six from 1500 px of board, one of three under it. */
async function computer(browser, name) {
  for (const [w, h, tracks] of [[1900, 1000, 6], [1400, 900, 3]]) {
    runningIn(`${name} ${w}×${h}`);
    const { f, page, go } = await board(browser, { viewport: { width: w, height: h } }, { motion: "no-preference" });
    try {
      await go();
      const g = await look(page);
      const track = (g.board.w - 2 * g.gutter - (tracks - 1) * 8) / tracks, todoTracks = tracks === 6 ? 2 : 1;
      check(`AC-105 ${w} px: TODO is a column of its own, ${todoTracks} of the board's ${tracks} tracks; the panel takes the rest beside it`,
        near(g.todo.w, todoTracks * track + (todoTracks - 1) * 8, 2) && near(g.panel.l, g.todo.r + 8, 1) && near(g.panel.r, g.board.r - g.gutter, 1) && near(g.panel.t, g.todo.t), JSON.stringify({ todo: g.todo, panel: g.panel, track }));
      check(`AC-105 ${w} px: the panel's tabs name IN PROGRESS, WAIT FOR HUMAN, INTEGRATION, FAILED and DONE with their counts, IN PROGRESS shown`,
        g.tabs.map((t) => t.words).join("|") === "IN PROGRESS4|WAIT FOR HUMAN2|INTEGRATION1|FAILED1|DONE12" && g.shown === "IN PROGRESS" && g.tabs[0].on, JSON.stringify(g.tabs));
      check(`AC-105 ${w} px: WAIT FOR HUMAN's and FAILED's counts are lit and pulse; the others not`,
        g.tabs.filter((t) => t.alarm).map((t) => t.s).join() === "WAIT FOR HUMAN,FAILED" && g.tabs.every((t) => (t.pulse === "askpulse") === t.alarm), JSON.stringify(g.tabs));
      check(`AC-105 ${w} px: the panel's cards are a grid of ${tracks === 6 ? 4 : 2} columns on the tracks`,
        new Set(g.panelCards.map((c) => c.l)).size === (tracks === 6 ? 4 : 2) && g.panelCards.every((c) => near(c.w, g.panelCards[0].w)), JSON.stringify(g.panelCards));
      check(`AC-105 ${w} px: TODO's head and the tabs' row stand on one line, their first cards on one line`,
        near(g.todoHead.t, g.panel.t) && near(g.todoCard.t, g.panelCards[0].t, 2) && g.keys.t >= g.panel.t && g.keys.b <= g.tabRow.b + 12, JSON.stringify({ head: g.todoHead, tabs: g.tabRow, keys: g.keys, card: g.todoCard, first: g.panelCards[0] }));
      if (tracks === 6) check("AC-138 an agent is a track wide, the first on TODO's left edge", near(g.agent.l, g.todo.l) && near(g.agent.w, track, 2), JSON.stringify({ agent: g.agent, track }));
      check(`AC-106 ${w} px: TODO's + New task is a text key as tall as the sort keys, on their line, before them`,
        g.newWords === "New task" && g.sortKeys.every((k) => near(k.h, g.newKey.h) && near(k.t, g.newKey.t)) && g.newKey.r <= g.sortKeys[0].l, JSON.stringify({ key: g.newKey, sort: g.sortKeys }));
      await shot(page, `boardpanel-${name}-${w}`);

      // a tab: its column in the panel, kept in this browser
      await page.click('#colswitch button[data-status="DONE"]');
      check(`AC-105 ${w} px: DONE's tab shows DONE's cards in the panel, IN PROGRESS's go`,
        await until(() => page.evaluate(() => document.querySelector(".colpanel .col.shown")?.dataset.status === "DONE" && !document.querySelector('.col[data-status="IN PROGRESS"] .card')?.offsetWidth)));
      await go();
      check(`AC-105 ${w} px: after a reload this browser shows DONE first`, (await look(page)).shown === "DONE");
      if (tracks === 3) {
        await page.click('#colswitch button[data-status="INTEGRATION"]');
        await page.route("**/api/tasks/meta", (r) => r.fulfill({ contentType: "application/json", body: JSON.stringify(meta(null)) }));
        counts.INTEGRATION = 0;
        tasks.INTEGRATION = [];
        await go();
        const after = await look(page);
        check("AC-105 INTEGRATION's tab gone, the panel shows IN PROGRESS", after.shown === "IN PROGRESS" && !after.tabs.some((t) => t.s === "INTEGRATION"), JSON.stringify(after.tabs));
        counts.INTEGRATION = CARDS.INTEGRATION;
        tasks.INTEGRATION = [task("INTEGRATION", 0)];
      }
      await page.click('.col[data-status="TODO"] .colhead .new');
      check(`AC-106 ${w} px: + New task opens the new task's window`, await until(() => page.evaluate(() => document.getElementById("newbox").open)));
    } finally { await f.close(); }
  }
  // Reduce Motion: the counts are lit, still
  const { f, page, go } = await board(browser, { viewport: { width: 1400, height: 900 } });
  try {
    await go();
    const g = await look(page);
    check("AC-105 with Reduce Motion the lit counts stand still", g.tabs.some((t) => t.alarm) && g.tabs.every((t) => t.pulse === "none"), JSON.stringify(g.tabs));
  } finally { await f.close(); }
}

/** An iPad, both ways and both sizes: one row of 44 px keys, TODO and the panel whole, nothing past the screen. */
async function tablet(browser, name) {
  for (const [w, h] of [[1180, 820], [1366, 1024], [820, 1180], [744, 1133]]) {
    runningIn(`${name} iPad ${w}×${h}`);
    const { f, page, go } = await board(browser, { viewport: { width: w, height: h }, hasTouch: true });
    try {
      await go();
      const g = await look(page);
      const row = g.head.filter((k) => k.t < g.bar.t + 40);
      check(`AC-105 tablet ${w}×${h}: the head is one row, every key 44 px, none past the screen`,
        g.head.every((k) => near(k.t, row[0].t, 2)) && g.head.every((k) => k.h >= 44 && k.r <= w) && g.docW === w, JSON.stringify({ head: g.head, docW: g.docW }));
      check(`AC-105 tablet ${w}×${h}: TODO and the panel at least 300 px wide, ${w > 1100 ? "side by side" : "TODO over the panel"}`,
        g.todo.w >= 300 && g.panel.w >= 300 && (w > 1100 ? near(g.todo.t, g.panel.t) : g.panel.t >= g.todo.b), JSON.stringify({ todo: g.todo, panel: g.panel }));
      check(`AC-105 tablet ${w}×${h}: TODO's head and the tabs each on one line; a TODO card's title gets room`,
        g.todoHead.h <= 60 && g.tabs.every((t) => near(t.t, g.tabs[0].t)) && g.todoTitle.w >= 150, JSON.stringify({ head: g.todoHead, tabs: g.tabs, title: g.todoTitle }));
      check(`AC-105 tablet ${w}×${h}: the agents, TODO and the panel start on the gutter`,
        near(g.agent.l, g.gutter) && near(g.todo.l, g.gutter) && near(Math.min(g.panel.l, g.todo.l), g.gutter), JSON.stringify({ agent: g.agent, todo: g.todo, panel: g.panel, gutter: g.gutter }));
      await shot(page, `boardpanel-${name}-ipad-${w}x${h}`);
      await page.tap('#colswitch button[data-status="WAIT FOR HUMAN"]');
      check(`AC-105 tablet ${w}×${h}: a tap on a tab shows its column`, await until(() => page.evaluate(() => document.querySelector(".colpanel .col.shown")?.dataset.status === "WAIT FOR HUMAN")));
    } finally { await f.close(); }
  }
}

/** A phone on its side: a touch screen 500 px high or less keeps a phone's board, the cards the room. */
async function sideways(browser, name) {
  runningIn(`${name} phone 844×390`);
  const { f, page, go } = await board(browser, { viewport: { width: 844, height: 390 }, isMobile: true, hasTouch: true });
  try {
    await go();
    const g = await page.evaluate(() => ({ cols: [...document.querySelectorAll(".col")].filter((c) => c.offsetWidth).map((c) => Math.round(c.getBoundingClientRect().width)),
      line: !!document.getElementById("poolline").offsetWidth, agents: !!document.getElementById("agentbar").offsetWidth, find: !!document.getElementById("findkey").offsetWidth,
      card: Math.round(document.querySelector('.col[data-status="TODO"] .card').getBoundingClientRect().top) }));
    check("AC-105 a phone on its side: one column a screen wide, the one-row head's Find key, the agents behind one line, the first card under 240 px as upright",
      g.cols.every((w) => w === 844) && g.line && !g.agents && g.find && g.card < 240, JSON.stringify(g));
  } finally { await f.close(); }
}

export async function webBoardPanel(browser, name) {
  await computer(browser, name);
  await tablet(browser, name);
  await sideways(browser, name);
}
