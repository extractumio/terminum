// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum-kanban.md AC-203 against a fake terminumd (fake.mjs): the board's three sections named on rails at their
// left, one column, the words turned to read bottom to top, 2 px under the board's words, a step over the cards'
// surface; Agent Pool's and Routines' rails fold them to one strip with a line of what they hold and their add key,
// remembered on a reload; each block's add tile right after its last card; Task Management's rail a plain head; a
// phone's sheet names the two flat. terminum-schedules.md AC-163: "Routines" in the board's words.
import { check, runningIn, until } from "./check.mjs";
import { fakeBoard, ORIGIN, openPage, PHONE, shot, THEME } from "./fake.mjs";

const META = { projects: [{ id: "pa", name: "API", color: 0, dir: "/w/api" }], users: [], sessions: [], hosts: [], last_agents: {},
  agents: [{ id: "a1", project: "pa", name: "Opus reviewer", kind: "claude", color: 0, line: "claude", at_once: 3 },
    { id: "a2", project: "pa", name: "Codex", kind: "codex", color: 2, line: "codex", at_once: 1 }] };
const SCHED = [1, 2, 3].map((n) => ({ id: `s${n}`, key: `SCH-${n}`, project: "pa", title: `Routine ${n}`, severity: "P2", agent: "a1",
  words: "every day", next: [2000000000 - n], next_words: [`today 0${n}:00`], paused: false }));
const BIG = { viewport: { width: 1900, height: 1000 } };

async function board(browser, device, schedules = SCHED, meta = META, tasks = []) {
  const f = await openPage(browser, { device, url: "/kanban/?project=pa" });
  await fakeBoard(f.page, { meta: () => meta, tasks: (status) => status === "TODO" ? tasks : [], counts: { TODO: tasks.length }, schedules });
  await f.page.goto(`${ORIGIN}/kanban/?project=pa`);
  await f.page.waitForSelector("#agentbar .agentcard", { state: "attached" });
  return f;
}

const look = (page) => page.evaluate(() => {
  const box = (e) => { const r = e?.getBoundingClientRect(); return r && r.width ? { l: Math.round(r.left), r: Math.round(r.right), t: Math.round(r.top), b: Math.round(r.bottom), w: Math.round(r.width), h: Math.round(r.height) } : null; };
  const q = (s) => document.querySelector(s), all = (s) => [...document.querySelectorAll(s)];
  const rails = ["#agentbar > .rail", "#schedbar > .rail", "#railboard"].map((s) => { const r = q(s), w = r?.querySelector(".railword"), cs = r && getComputedStyle(r);
    return { ...box(r), word: w?.textContent, wordBox: box(w), size: cs && parseFloat(cs.fontSize), bg: cs?.backgroundColor, ink: cs?.color, tag: r?.tagName,
      key: !!r?.querySelector("button.railkey"), chev: !!r?.querySelector(".chev"), cursor: cs?.cursor }; });
  const tiles = (sec) => { const items = all(`${sec} .trackitems > *`).filter((e) => e.offsetWidth), last = items.at(-1), card = items.at(-2);
    return { last: last?.id, n: items.length, tile: box(last), card: box(card) }; };
  return { rails, page: getComputedStyle(document.body).backgroundColor, card: getComputedStyle(q("#agentbar .agentcard")).backgroundColor, body: parseFloat(getComputedStyle(q(".colhead")).fontSize),
    pool: tiles("#agentbar"), sched: tiles("#schedbar"), folded: ["#agentbar", "#schedbar"].map((s) => ({ folded: q(s).classList.contains("folded"), h: box(q(`${s} > .rail`))?.h,
      expanded: q(`${s} .railkey`).getAttribute("aria-expanded"), sum: q(`${s} .railsum`).textContent, plus: !!q(`${s} .railadd`)?.offsetWidth, body: !!q(`${s} .tracks`)?.offsetWidth })),
    words: document.body.innerText };
});
/** WCAG contrast of two rgb() colors. */
function contrast(a, b) {
  const l = (c) => { const k = c.startsWith("color(") ? 1 : 255;
    const [r, g, bl] = (c.match(/[\d.]+/g) ?? []).slice(0, 3).map((v) => { v /= k; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }); return 0.2126 * r + 0.7152 * g + 0.0722 * bl; };
  const [x, y] = [l(a), l(b)].sort((m, n) => n - m);
  return (x + 0.05) / (y + 0.05);
}

export async function webBoardRail(browser, name) {
  let at = `(${name} 1900×1000)`;
  runningIn(`${name} 1900×1000`);
  let f = await board(browser, BIG);
  let { page } = f;
  let g = await look(page);
  check(`AC-163 AC-203 ${at} Routines folded by default: one 28 px strip "Routines", its count and next run, its + at the end; no "Scheduled", no "+ Schedule"`,
    g.folded[1].folded && g.folded[1].h === 28 && g.folded[1].expanded === "false" && g.folded[1].sum === "3 · next SCH-3 today 03:00" && g.folded[1].plus && !g.folded[1].body
    && !/Scheduled|\+ Schedule/.test(g.words), JSON.stringify(g.folded[1]));
  check(`AC-203 ${at} the pool's add tile right after its second agent, as tall, in the third track's place`,
    g.pool.last === "addagent" && g.pool.tile.l > g.pool.card.r && g.pool.tile.l - g.pool.card.r <= 10 && g.pool.tile.h === g.pool.card.h && g.pool.tile.w === 44, JSON.stringify(g.pool));
  await page.click("#railsched");
  await until(async () => (await look(page)).folded[1].body);
  g = await look(page);
  check(`AC-163 AC-203 ${at} unfolded: the routines' + tile right after the third card, as tall; the rail as tall as the block`,
    g.sched.last === "newschedule" && g.sched.tile.l > g.sched.card.r && g.sched.tile.h === g.sched.card.h && g.folded[1].expanded === "true" && !g.folded[1].plus && g.rails[1].h >= g.sched.card.h, JSON.stringify({ sched: g.sched, fold: g.folded[1] }));
  g = await look(page);
  const [pool, , tasks] = g.rails;
  check(`AC-203 ${at} three rails in one column at the left, 24 px wide: Agent Pool, Routines, Task Management`,
    g.rails.every((r) => r.l === pool.l && r.w === 24) && JSON.stringify(g.rails.map((r) => r.word)) === '["Agent Pool","Routines","Task Management"]', JSON.stringify(g.rails));
  check(`AC-203 ${at} the words turned (taller than wide), 2 px under the board's, on their own surface, 4.5:1`,
    g.rails.every((r) => r.wordBox.h > r.wordBox.w && r.size === g.body - 2 && r.bg !== g.page && contrast(r.ink, r.bg) >= 4.5), JSON.stringify({ rails: g.rails, body: g.body, page: g.page }));
  check(`AC-203 ${at} Task Management's rail is a plain head: an h2, no key, no chevron, no pointer`,
    tasks.tag === "H2" && !tasks.key && !tasks.chev && tasks.cursor === "default" && pool.key && pool.chev, JSON.stringify(tasks));
  await page.click("#railpool");
  await until(async () => (await look(page)).folded[0].folded);
  await page.reload();
  await page.waitForSelector("#agentbar .agentcard", { state: "attached" });
  await until(async () => (await look(page)).folded[0].sum !== "");
  g = await look(page);
  check(`AC-203 ${at} the pool folded and Routines open stay so after a reload; the pool's strip says what it holds`,
    g.folded[0].folded && g.folded[0].h === 28 && g.folded[0].sum === "2 agents" && g.folded[0].plus && !g.folded[1].folded, JSON.stringify(g.folded));
  await page.click("#agentbar .railadd");
  check(`AC-203 ${at} the folded strip's + adds an agent, the strip stays folded`, await until(() => page.isVisible("#agentbox[open]")) && (await look(page)).folded[0].folded, "");
  await page.keyboard.press("Escape");
  await page.click("#railpool");
  await page.click("#railsched");
  check(`no page error ${at}`, f.errors.length === 0, f.errors.join("; "));
  await f.close();

  // no routine: the strip says none, its + opens New routine
  f = await board(browser, BIG, []);
  ({ page } = f);
  await until(async () => (await look(page)).folded[1].sum === "none");
  await page.click("#schedbar .railadd");
  check(`AC-163 ${at} no routine, folded: "Routines none", its + opens New routine`,
    await until(() => page.isVisible("#schedbox[open]")) && await page.textContent("#schedhead") === "New routine", await page.textContent("#schedhead"));
  await f.close();

  at = `(${name} 390×844)`;
  runningIn(`${name} 390×844`);
  f = await board(browser, PHONE);
  ({ page } = f);
  await page.click("#poolline");
  await until(() => page.evaluate(() => "sheet" in document.getElementById("pool").dataset));
  const phone = await page.evaluate(() => ["#agentbar > .rail", "#schedbar > .rail", "#railboard"].map((s) => { const r = document.querySelector(s).getBoundingClientRect(), w = document.querySelector(`${s} .railword`).getBoundingClientRect();
    return { w: Math.round(r.width), h: Math.round(r.height), flat: w.width > w.height, text: document.querySelector(s).textContent.trim() }; }));
  check(`AC-203 ${at} the sheet names Agent Pool and Routines as flat 44 px heads; Task Management's is for screen readers alone`,
    phone[0].h === 44 && phone[0].flat && phone[1].h === 44 && phone[1].flat && phone[2].w <= 1 && phone[2].text === "Task Management", JSON.stringify(phone));
  await page.click("#railsched");
  check(`AC-203 ${at} its fold works in the sheet`, await until(() => page.evaluate(() => document.getElementById("railsched").getAttribute("aria-expanded") === "true")), "");
  await shot(page, `boardrail-${name}-phone`);
  check(`no page error ${at}`, f.errors.length === 0, f.errors.join("; "));
  await f.close();
  await fullWidth(browser, name);
}

/** AC-138, AC-163, AC-203: both sections use all six tracks, including overflow to a new row. */
async function fullWidth(browser, name) {
  for (const n of [5, 6, 7]) {
    runningIn(`${name} 1900×1000 ${n} cards`);
    const meta = { ...META, agents: Array.from({ length: n }, (_, i) => ({ ...META.agents[0], id: `a${i + 1}`, name: `Agent ${i + 1}` })) };
    const schedules = Array.from({ length: n }, (_, i) => ({ ...SCHED[0], id: `s${i + 1}`, key: `SCH-${i + 1}` }));
    const tasks = [{ id: "API-1", title: "Task to select", project: "pa", status: "TODO", severity: "P2", updated_at: 1, parent: null }];
    const f = await board(browser, BIG, schedules, meta, tasks), { page } = f;
    try {
      await page.click("#railsched");
      for (const section of ["#agentbar", "#schedbar"]) {
        const g = await page.$eval(`${section} .tracks`, (tracks) => {
          const box = (e) => { const r = e.getBoundingClientRect(); return { l: r.left, r: r.right, t: r.top, b: r.bottom, w: r.width }; };
          const list = tracks.querySelector(".trackitems"), cards = [...list.querySelectorAll(".item")].map(box);
          return { tracks: box(tracks), list: box(list), cards, add: box(list.querySelector(".addtile")), side: tracks.querySelector(".trackside")?.getBoundingClientRect().height ?? 0 };
        });
        check(`AC-138 AC-163 AC-203 ${section}: ${n} cards fill six tracks before wrapping`,
          Math.abs(g.list.w - g.tracks.w) <= 1 && g.cards.length === n && g.cards.slice(0, 6).every((c, i) =>
            Math.abs(c.l - (g.tracks.l + i * (g.cards[0].w + 8))) <= 1 && Math.abs(c.t - g.cards[0].t) <= 1)
          && (n !== 7 || g.cards[6].t > g.cards[0].t && Math.abs(g.cards[6].l - g.tracks.l) <= 1)
          && g.side === 0, JSON.stringify(g));
        const last = g.cards.at(-1);
        check(`AC-203 ${section}: add tile follows ${n} cards`, g.add.w === 44 && (n === 6
          ? g.add.t > last.t && Math.abs(g.add.l - g.tracks.l) <= 1
          : Math.abs(g.add.t - last.t) <= 1 && Math.abs(g.add.l - last.r - 8) <= 1), JSON.stringify(g));
      }
      if (n === 5) {
        await shot(page, `boardrail-${name}-wide-dark`);
        await page.click('.card[data-id="API-1"] .ctitle', { modifiers: ["Meta"] });
        check("AC-203 selection hint occupies its own full-width row under the pool", await until(() => page.$eval(".trackside", (e) => {
          const r = e.getBoundingClientRect(), list = e.previousElementSibling.getBoundingClientRect();
          return r.height > 0 && r.top >= list.bottom && Math.abs(r.width - list.width) <= 1 && /1 chosen/.test(e.textContent);
        })));
        await page.keyboard.press("Escape");
        check("AC-203 clearing selection removes the hint's row", await until(() => page.$eval(".trackside", (e) => !e.getBoundingClientRect().height)));
        meta.theme = { ...THEME, bg: "#ffffff", fg: "#1f2328", dark: false };
        await page.reload();
        await page.waitForSelector("#agentbar .agentcard");
        await shot(page, `boardrail-${name}-wide-light`);
        await page.setViewportSize({ width: 1400, height: 900 });
        await shot(page, `boardrail-${name}-tablet-light`);
      }
      check("full-width cards: no page error", f.errors.length === 0, f.errors.join("; "));
    } finally { await f.close(); }
  }
}
