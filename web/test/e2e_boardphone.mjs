// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The board on a phone (terminum-kanban.md AC-105 "a phone's one-row head", AC-117's counters on the board) against
// a fake terminumd: at 600 px or narrower one row of keys, Find and the filters under it by the Find key, lit while
// the board is narrowed; Auto-Dispatch, the agents and the schedules behind one line that opens them as a sheet; a
// column's keys scrolling with its cards; the lines of tasks that need the human folded into the head's counters,
// a fresh one pulsing; a wider window keeps its two rows.
import { check, runningIn, until } from "./check.mjs";
import { drawn, fakeBoard, openPage, ORIGIN, pause, PHONE, shot, THEME } from "./fake.mjs";

const tasks = Array.from({ length: 30 }, (_, i) => ({ id: `API-${i + 1}`, title: `Task ${i + 1} with a title long enough to be cut`, project: "api", status: "TODO",
  severity: i ? "P2" : "P0", owner: "human", owner_name: "Alex", updated_at: i, parent: null }));
const counts = { TODO: 30, "IN PROGRESS": 0, "WAIT FOR HUMAN": 2, FAILED: 1, DONE: 0 };
const META = { projects: [{ id: "api", name: "API service with a long name", color: 0 }], users: [], sessions: [], hosts: [], theme: THEME,
  agents: [{ id: "a1", project: "api", name: "Claude Code" }, { id: "a2", project: "api", name: "Codex" }] };
const LINES = { t: "taskmarks", waiting: 2, failed: 1, lines: [{ id: "API-7", key: "w7", words: "API-7 waits for you" },
  { id: "API-9", key: "w9", words: "API-9 waits for you" }, { id: "API-3", key: "f3", words: "API-3 failed", failed: true }] };
const HEAD = ["#navbtn", "#fproject summary", ".askn.failed", ".askn.waiting", "#findkey", "#projectcog"];

/** Where the head's keys stand, and what else is in sight. */
const look = (page, head = HEAD) => page.evaluate((head) => {
  const box = (sel) => { const e = document.querySelector(sel), r = e?.getBoundingClientRect(); return r && r.width > 0 && getComputedStyle(e).visibility !== "hidden" ? { l: Math.round(r.left), r: Math.round(r.right), t: Math.round(r.top), b: Math.round(r.bottom), w: Math.round(r.width), h: Math.round(r.height) } : null; };
  return { win: innerWidth, keys: head.map(box), find: box("#find"), picks: box("#picks"), agents: box("#agentbar"), line: box("#poolline"), asks: box("#asks"),
    card: box('.col[data-status="TODO"] .card'), colhead: box('.col[data-status="TODO"] .colhead'), auto: document.getElementById("autokeys").parentElement.id,
    lineWords: document.getElementById("poolline").textContent };
}, head);

async function board(browser, device, motion = "reduce") {
  const f = await openPage(browser, { device: { ...device, reducedMotion: motion }, secret: "a".repeat(64), stored: "a".repeat(64) });
  const { page } = f;
  await drawn(page);
  await fakeBoard(page, { meta: () => META, tasks: (status) => (status === "TODO" ? tasks : []), counts });
  const go = async (query = "?project=api") => {
    await page.goto(`${ORIGIN}/kanban/${query}`);
    await page.waitForSelector(".col .card");
    await page.waitForFunction(() => document.body.dataset.socket === "open");
  };
  return { f, page, go };
}

export async function webBoardPhone(browser, name) {
  for (const width of [390, 320]) {
    runningIn(`${name} ${width}×700`);
    const { f, page, go } = await board(browser, { ...PHONE, viewport: { width, height: 700 } });
    try {
      await go();
      f.say(LINES);
      await page.waitForSelector(".askn.waiting:not([hidden])");
      let g = await look(page);
      check("AC-105 phone: the head is one row of keys, each a finger's size, none past the screen's edge",
        g.keys.every((k) => k && k.t === g.keys[0].t && k.w >= 44 && k.h >= 44 && k.l >= 0 && k.r <= g.win) && g.keys.every((k, i) => !i || k.l >= g.keys[i - 1].r), JSON.stringify(g.keys));
      check("AC-105 phone: Find, the filters and the agents take no room; the first card stands under the switch and the line", !g.find && !g.picks && !g.agents && g.line && g.card.t < 240, JSON.stringify(g));
      check("AC-117 board: the lines fold into the head's counters, a single kind too; no line stands over the board", !g.asks && await page.textContent(".askn.waiting") === "2" && await page.textContent(".askn.failed") === "1", JSON.stringify(g.asks));
      check("AC-105 phone: the line says Auto-Dispatch's state first, then the agents and the routines", /^Auto OFF\s*Agents 2\s*Routines 0$/.test(g.lineWords.trim()), g.lineWords);
      await shot(page, `boardphone-${name}-${width}`);

      // a counter: its lines as a sheet, the board not moved
      const cardTop = g.card.t;
      await page.click(".askn.waiting");
      await pause(page, 200);
      const sheet = await page.evaluate(() => { const r = document.getElementById("asks").getBoundingClientRect(); return { h: r.height, half: visualViewport.height / 2, lines: [...document.querySelectorAll("#asks .ask")].filter((a) => a.offsetHeight > 0).length }; });
      check("AC-117 board: a counter opens its kind's lines as a sheet at most half the screen", sheet.lines === 2 && sheet.h > 0 && sheet.h <= sheet.half + 1, JSON.stringify(sheet));
      await page.keyboard.press("Escape");
      await pause(page, 200);
      g = await look(page);
      check("AC-117 board: Esc closes the sheet; the cards never moved for the lines", !g.asks && g.card.t === cardTop, JSON.stringify({ asks: g.asks, card: g.card, cardTop }));

      // Find and the filters, by the Find key
      await page.click("#findkey");
      g = await look(page);
      check("AC-105 phone: the Find key shows Find and the filters on rows under the head, the field taking the keys",
        g.find && g.picks && g.find.t >= g.keys[0].b && g.picks.t >= g.find.b && g.find.r <= g.win && await page.evaluate(() => document.activeElement.id === "find" && document.getElementById("findkey").getAttribute("aria-expanded") === "true"), JSON.stringify(g));
      check("AC-105 phone: with nothing narrowed the Find key is not lit", !await page.$("#findkey.lit"));
      check("AC-111 phone: Find is the head's, saying it finds tasks", await page.evaluate(() => !document.getElementById("find").closest(".colhead") && document.getElementById("find").placeholder === "Find tasks"));
      check("AC-159 phone: a folded TODO card no agent ran shows no Run key", await page.$eval('.card[data-id="API-1"]', (c) => c.matches(".compact:not(.open, .onagent)") && getComputedStyle(c.querySelector(".crun")).display === "none"));
      await page.fill("#find", "Task 1");
      check("AC-105 phone: a search lights the Find key", await until(() => page.$("#findkey.lit")));
      await page.click("#findkey");
      g = await look(page);
      check("AC-105 phone: a second press puts them away; the key stays lit while the board is narrowed", !g.find && !g.picks && !!await page.$("#findkey.lit"), JSON.stringify(g));
      await go("?project=api&severity=P0");
      check("AC-105 phone: a filter in the address lights the Find key at once", await until(() => page.$("#findkey.lit")));
      await go();

      // Auto-Dispatch, the agents and the schedules: one sheet
      await page.click("#poolline");
      await pause(page, 200);
      const pool = await page.evaluate(() => { const p = document.getElementById("pool"), r = p.getBoundingClientRect(), seen = (id) => document.getElementById(id).getBoundingClientRect().height > 0;
        return { l: Math.round(r.left), r: Math.round(innerWidth - r.right), t: r.top, b: visualViewport.height - r.bottom, parts: ["autoswitch", "agentbar", "schedbar"].map(seen), agents: p.querySelectorAll(".agentcard").length,
          shaded: document.elementFromPoint(4, 4)?.id }; });
      check("AC-105 phone: the line opens Auto-Dispatch, the agents and the schedules as one centered sheet over a shade",
        pool.l === 16 && pool.r === 16 && pool.t >= 0 && pool.b >= 0 && pool.parts.every(Boolean) && pool.agents === 2 && pool.shaded === "sheetscrim", JSON.stringify(pool));
      await shot(page, `boardphone-${name}-${width}-pool`);
      await page.keyboard.press("Escape");
      await pause(page, 200);
      check("AC-105 phone: Esc closes it", !(await look(page)).agents && await page.getAttribute("#poolline", "aria-expanded") === "false");

      // New task's × closes it and keeps what was typed (kanbanform.js's draft)
      await page.click("#newtask");
      await page.waitForSelector("#newbox[open]");
      await page.fill("#newtitle", "kept as a draft");
      await page.click("#newbox .dhead .x");
      const closed = await until(() => page.evaluate(() => !document.getElementById("newbox").open));
      await page.click("#newtask");
      await page.waitForSelector("#newbox[open]");
      check("AC-173 phone: × closes New task and keeps its draft", closed && await page.inputValue("#newtitle") === "kept as a draft", await page.inputValue("#newtitle"));
      await page.fill("#newtitle", "");
      await page.click("#newbox .dhead .x");
      await until(() => page.evaluate(() => !document.getElementById("newbox").open));

      // a column's own keys go with its cards
      const before = (await look(page)).colhead;
      await page.$eval('.col[data-status="TODO"]', (c) => { c.scrollTop = 150; });
      const after = (await look(page)).colhead;
      check("AC-105 phone: a column's keys scroll away with its cards", before.t - after.t === 150, JSON.stringify({ before, after }));
      f.say({ ...LINES, waiting: 3 });                // any news redraws the board's counts; a redraw keeps the scroll
      await pause(page, 300);
      check("no page errors", !f.errors.length, f.errors.join(" | "));
    } finally { await f.close(); }
  }

  // the counter's blink, with motion; still with reduced motion (the other checks)
  runningIn(`${name} 390×700 motion`);
  const m = await board(browser, { ...PHONE, viewport: { width: 390, height: 700 } }, "no-preference");
  try {
    await m.go();
    m.f.say(LINES);
    await m.page.waitForSelector(".askn.waiting.fresh");
    const anim = (sel) => m.page.$eval(sel, (b) => { const cs = getComputedStyle(b, "::after"); return `${cs.animationName} ${cs.animationIterationCount}`; });   // the glow round its ring
    check("AC-117 a counter with a line not looked at pulses, on and on", await anim(".askn.waiting") === "askpulse infinite", await anim(".askn.waiting"));
    await m.page.click(".askn.waiting");
    await pause(m.page, 300);
    await m.page.keyboard.press("Escape");
    await pause(m.page, 300);
    check("AC-117 its lines opened, it rests; the other still pulses", await anim(".askn.waiting") === "none 1" && await anim(".askn.failed") === "askpulse infinite", `${await anim(".askn.waiting")} | ${await anim(".askn.failed")}`);
  } finally { await m.f.close(); }
  runningIn(`${name} 390×700 reduced motion`);
  const still = await board(browser, { ...PHONE, viewport: { width: 390, height: 700 } });
  try {
    await still.go();
    still.f.say(LINES);
    await still.page.waitForSelector(".askn.waiting.fresh");
    check("AC-117 with reduced motion the ring is still", await still.page.$eval(".askn.waiting", (b) => getComputedStyle(b, "::after").animationName === "none" && getComputedStyle(b).boxShadow !== "none"));
  } finally { await still.f.close(); }

  // 601 px and wider: the two rows, the agents over the columns, Auto-Dispatch in the head; a mouse's wide window keeps its lines
  runningIn(`${name} 700×700`);
  const t = await board(browser, { ...PHONE, viewport: { width: 700, height: 700 } });
  try {
    await t.go();
    t.f.say(LINES);
    await t.page.waitForSelector(".askn.waiting:not([hidden])");
    const g = await look(t.page, ["#navbtn", "#find", "#agentbar"]);
    check("AC-105 a window 601 px or wider keeps Find in the head, the agents over the columns and Auto-Dispatch in the head; no Find key, no line",
      g.keys.every(Boolean) && !g.line && g.auto === "boardacts" && !await t.page.isVisible("#findkey"), JSON.stringify(g));
    check("AC-117 board: where the buttons float the counters stand in the head there too", !g.asks && await t.page.isVisible(".askn.waiting"));
    await t.page.setViewportSize({ width: 390, height: 700 });
    await pause(t.page, 200);
    const narrow = await look(t.page);
    await t.page.click("#poolline");                 // its sheet open while the window grows
    await t.page.waitForSelector("#pool[data-sheet]");
    await t.page.setViewportSize({ width: 700, height: 700 });
    await pause(t.page, 500);
    const back = await look(t.page);
    const left = await t.page.evaluate(() => ({ sheet: "sheet" in document.getElementById("pool").dataset, shade: document.elementFromPoint(350, 600)?.id }));
    check("AC-105 a window made narrow, then wide again with the sheet open: Auto-Dispatch's keys go to the sheet and come back to the head, no sheet and no shade left",
      narrow.auto === "pool" && back.auto === "boardacts" && !!back.agents && !left.sheet && left.shade !== "sheetscrim", JSON.stringify({ narrow: narrow.auto, back: back.auto, left }));
  } finally { await t.f.close(); }
  runningIn(`${name} 1400×900`);
  const w = await board(browser, { viewport: { width: 1400, height: 900 } });
  try {
    await w.go();
    w.f.say(LINES);
    await w.page.waitForSelector("#asks .ask");
    check("AC-117 a computer keeps its lines over the board, no counter", await w.page.isVisible("#asks .ask") && !await w.page.isVisible("#askcount"));
  } finally { await w.f.close(); }
}
