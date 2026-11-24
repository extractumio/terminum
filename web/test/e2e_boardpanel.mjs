// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The board wider than a phone (terminum-kanban.md AC-105 "TODO and the panel", "a tablet"; AC-106's + New task) against
// a fake terminumd: TODO a column of its own, the other columns one at a time in a panel under their tabs, each with its
// count, WAIT FOR HUMAN's and FAILED's pulsing while they hold tasks; the board's tracks by its width; on an iPad the head
// one row of finger-sized keys, nothing cut, every left edge on the gutter or a track.
import { check, runningIn, until } from "./check.mjs";
import { drawn, fakeBoard, openPage, ORIGIN, pause, shot, TABLET, THEME } from "./fake.mjs";

const NOW = 1_800_000_000;
const task = (status, i, extra = {}) => ({ id: `API-${status.length}${i}`, title: `${status} task ${i}: a title long enough to be read whole on a tablet`, project: "api", status,
  severity: ["P1", "P2", "P0"][i % 3], owner: "human", owner_name: "Alex", updated_at: NOW - i, parent: null, ...extra });
const CARDS = { TODO: 9, "IN PROGRESS": 4, "WAIT FOR HUMAN": 2, INTEGRATION: 1, FAILED: 1, DONE: 12 };
const tasks = Object.fromEntries(Object.entries(CARDS).map(([s, n]) => [s, Array.from({ length: n }, (_, i) => task(s, i))]));
tasks["IN PROGRESS"][1].title = "Short";            // a row's cards as tall as its tallest (AC-105)
const counts = { ...CARDS };
const agents = ["Opus", "Sonnet", "Fable", "Codex"].map((name, i) => ({ id: `a${i}`, project: "api", name, kind: i === 3 ? "codex" : "claude", at_once: 2 }));
const meta = (integration) => ({ projects: [{ id: "api", name: "API", color: 0, integration }], users: [], sessions: [], hosts: [], theme: THEME, agents });

/** Find's place, the tabs' counts against their names' baselines, a card's date against its ID (AC-105, AC-111). */
const details = (page) => page.evaluate(() => {
  const mark = (e) => { const m = document.createElement("span"); m.style.cssText = "display:inline-block;width:0;height:0"; e.append(m); const y = m.getBoundingClientRect().top; m.remove(); return Math.round(y * 10) / 10; };
  const find = document.getElementById("find"), head = document.querySelector('.col[data-status="TODO"] .colhead'), card = document.querySelector('.col[data-status="TODO"] .card');
  const type = (e) => { const s = getComputedStyle(e); return `${s.fontSize} ${s.color}`; };
  return { findIn: find.closest(".colhead") === head && find.closest(".search").previousElementSibling?.classList.contains("colname"), words: find.placeholder,
    baselines: [...document.querySelectorAll("#colswitch button")].filter((b) => b.offsetWidth).map((b) => [mark(b.querySelector(".tabname")), mark(b.querySelector(".tabn"))]),
    when: card.querySelector(".cwhen")?.textContent ?? "", whenType: card.querySelector(".cwhen") && type(card.querySelector(".cwhen")), idType: type(card.querySelector(".cid")),
    run: [...document.querySelectorAll(".card")].filter((c) => c.querySelector(".crun")).map((c) => c.closest(".col").dataset.status),
    foldedRun: card.matches(".compact:not(.open, .onagent)") && getComputedStyle(card.querySelector(".crun")).display };
});

/** Where the board's parts stand. */
const look = (page) => page.evaluate(() => {
  const box = (e) => { const r = e?.getBoundingClientRect(); return r && r.width > 0 ? { l: Math.round(r.left), r: Math.round(r.right), t: Math.round(r.top), b: Math.round(r.bottom), w: Math.round(r.width), h: Math.round(r.height) } : null; };
  const q = (s) => document.querySelector(s), all = (s) => [...document.querySelectorAll(s)];
  const tabs = all("#colswitch button").filter((b) => b.offsetWidth).map((b) => ({ s: b.dataset.status, words: b.textContent, alarm: b.classList.contains("alarm"), on: b.getAttribute("aria-selected") === "true",
    pulse: getComputedStyle(b.querySelector(".tabn"), "::after").animationName, ...box(b) }));
  const head = all("#navbtn, #bar > .sections, #fproject summary, #filters .search, #filtersbtn, .askn, #autoswitch, #projectcog").map((e) => ({ id: e.id || e.className, ...box(e) })).filter((b) => b.w);
  return { win: innerWidth, docW: document.documentElement.scrollWidth, bar: box(q("#bar")), board: box(q("#board")), todo: box(q('.col[data-status="TODO"]')), panel: box(q(".colpanel")),
    todoHead: box(q('.col[data-status="TODO"] .colhead')), tabRow: box(q("#colswitch")), tabs, keys: box(q(".colpanel .colhead")),
    todoCard: box(q('.col[data-status="TODO"] .card')), panelCards: all(".colpanel .col.shown .card").map(box), shown: q(".colpanel .col.shown")?.dataset.status,
    agent: box(q("#agentbar .agentcard")), newKey: box(q('.col[data-status="TODO"] .colhead .new')), newWords: q('.col[data-status="TODO"] .colhead .new')?.textContent,
    sortKeys: all('.col[data-status="TODO"] .colsort button').map(box), todoTitle: box(q('.col[data-status="TODO"] .card .ctitle')), gutter: parseFloat(getComputedStyle(q("#board")).paddingLeft),
    rail: box(q("#railboard")), poolRail: box(q("#agentbar > .rail")), head };
});

async function board(browser, device, { motion = "reduce" } = {}) {
  const f = await openPage(browser, { device: { ...device, reducedMotion: motion }, secret: "a".repeat(64), stored: "a".repeat(64) });
  const { page } = f;
  await drawn(page);
  let integration = { on: true };
  await fakeBoard(page, { meta: () => meta(integration), tasks: (status) => tasks[status] ?? [], counts });
  const go = async () => {
    await page.goto(`${ORIGIN}/kanban/?project=api`);
    await page.waitForSelector(".col .card");
    await page.waitForFunction(() => document.body.dataset.socket === "open");
    await page.evaluate(() => { if (!document.getElementById("app").classList.contains("nav-hidden") && innerWidth > 860) document.getElementById("navclose").click(); });
    await pause(page, 100);
  };
  return { f, page, go, integrate: (on) => { integration = on ? { on: true } : null; } };
}

const near = (a, b, d = 1) => Math.abs(a - b) <= d;

/** A computer's window: TODO two tracks of six from 1500 px of board, one of three under it. */
async function computer(browser, name) {
  for (const [w, h, tracks] of [[1900, 1000, 6], [1400, 900, 3]]) {
    runningIn(`${name} ${w}×${h}`);
    const { f, page, go, integrate } = await board(browser, { viewport: { width: w, height: h } }, { motion: "no-preference" });
    try {
      await go();
      const g = await look(page);
      // Task Management's rail first, on the gutter, then the tracks (terminum-kanban.md AC-203)
      const track = (g.board.w - 2 * g.gutter - g.rail.w - 8 - (tracks - 1) * 8) / tracks, todoTracks = tracks === 6 ? 2 : 1;
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
      const rows = new Map();
      for (const c of g.panelCards) rows.set(c.t, [...(rows.get(c.t) ?? []), c.h]);
      check(`AC-105 ${w} px: a row's cards are as tall as its tallest`, g.panelCards.length > 2 && [...rows.values()].every((hs) => hs.every((h) => near(h, hs[0]))), JSON.stringify(g.panelCards));
      const d = await details(page);
      check(`AC-105 ${w} px: each tab's count stands on its name's baseline`, d.baselines.length >= 5 && d.baselines.every(([a, b]) => near(a, b)), JSON.stringify(d.baselines));
      check(`AC-105 ${w} px: a card says when it last changed, in its ID's type`, /\d/.test(d.when) && d.whenType === d.idType, JSON.stringify(d));
      check(`AC-122 ${w} px: only TODO's cards have a Run key`, d.run.length > 0 && d.run.every((s) => s === "TODO"), JSON.stringify(d.run));
      check(`AC-159 ${w} px: a folded TODO card no agent ran shows no Run key`, d.foldedRun === "none", JSON.stringify(d.foldedRun));
      check(`AC-111 ${w} px: Find stands in TODO's head after its name, saying it filters every column; the head one line`,
        d.findIn && d.words === "Filter all columns" && g.todoHead.h <= 40, JSON.stringify({ d, head: g.todoHead }));
      await page.fill("#find", "retry");
      check(`AC-111 ${w} px: typing in it narrows the board by the URL's text`, await until(() => page.evaluate(() => new URLSearchParams(location.search).get("q") === "retry")));
      await page.fill("#find", "");
      await until(() => page.evaluate(() => !new URLSearchParams(location.search).has("q")));
      check("AC-138 the responsive pool starts on TODO's left edge", near(g.agent.l, g.todo.l) && g.agent.w >= 240, JSON.stringify({ agent: g.agent, todo: g.todo }));
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
        integrate(false);
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
    const { f, page, go } = await board(browser, { ...TABLET, viewport: { width: w, height: h } });
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
      // the rails on the gutter, one column (terminum-kanban.md AC-203); TODO over the panel: the rail beside TODO alone,
      // the panel the board's whole width, on the gutter
      const start = g.gutter + g.rail?.w + 8, stacked = g.panel.t >= g.todo.b;
      check(`AC-105 AC-203 tablet ${w}×${h}: the rails on the gutter, one column; the agents and TODO start after them, the panel ${stacked ? "on the gutter under TODO and its rail" : "beside TODO"}`,
        near(g.rail?.l, g.gutter) && near(g.poolRail?.l, g.gutter) && near(g.poolRail?.w, g.rail?.w) && near(g.agent.l, start) && near(g.todo.l, start)
        && (stacked ? near(g.panel.l, g.gutter) && g.rail.b <= g.panel.t : near(g.panel.l, g.todo.r + 8)),
        JSON.stringify({ rail: g.rail, poolRail: g.poolRail, agent: g.agent, todo: g.todo, panel: g.panel, gutter: g.gutter }));
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
