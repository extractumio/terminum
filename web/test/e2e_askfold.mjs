// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum-kanban.md AC-117 "two counters on a touch screen" against a fake terminumd (fake.mjs): where the buttons
// float (a phone, a tablet) the tasks' lines fold into a red and a yellow counter left of ⟨…⟩, the terminal's rows
// unchanged; a counter opens its lines as a sheet at most half the visible height that scrolls; Esc, the shade, a
// line pressed or its last line going close it; a line not looked at rings its counter; a computer keeps the lines.
import { check, runningIn, until } from "./check.mjs";
import { drawn, group, item, openPage, PHONE, TABLET, WIDE } from "./fake.mjs";

const line = (id, failed, words) => ({ id, failed, words, key: `1|${words}` });
const FAILED = Array.from({ length: 14 }, (_, i) => line(`T-${i + 10}`, true, `T-${i + 10} failed: ${"a long reason that wraps on a phone ".repeat(2)}`));
const WAITING = [line("T-1", false, "T-1 waits for you"), line("T-2", false, "T-2 waits for you")];
const marks = (lines) => ({ t: "taskmarks", marks: {}, counts: {}, waiting: 0, failed: 0, lines });

const state = (page) => page.evaluate(() => {
  const $ = (id) => document.getElementById(id), r = (el) => el.getBoundingClientRect();
  const btn = (k) => { const b = document.querySelector(`#askcount [data-kind="${k}"]`); return { shown: !b.hidden && r(b).width > 0, n: b.textContent, label: b.getAttribute("aria-label"), fresh: b.classList.contains("fresh") }; };
  const asks = $("asks"), a = r(asks), fold = r($("foldfab")), cnt = r($("askcount"));
  return { failed: btn("failed"), waiting: btn("waiting"), termH: $("term").clientHeight, asksH: a.height, sheet: "sheet" in asks.dataset,
    rows: [...asks.querySelectorAll(".ask")].filter((c) => c.getBoundingClientRect().height > 0).map((c) => c.querySelector(".what").textContent),
    scrolls: asks.scrollHeight > asks.clientHeight + 1, half: a.height <= parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--app-h") || innerHeight) / 2 + 1,
    whole: [...asks.querySelectorAll(".ask")].filter((c) => c.getBoundingClientRect().height > 0).every((c) => { const w = c.querySelector(".what"); return w.scrollWidth <= w.clientWidth + 1; }),
    beside: Math.abs(cnt.top - fold.top) < 1 && cnt.right <= fold.left - 8 && cnt.width > 0 };
});

export async function webAskFold(browser, name) {
  for (const [size, device] of [["390×844", PHONE], ["1180×820 touch", TABLET]]) {
    const at = `(${name} ${size})`;
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device });
    const { page } = f;
    await drawn(page);
    const posts = [];
    await page.route("**/api/tasks/*/seen", (route) => { posts.push(new URL(route.request().url()).pathname); return route.fulfill({ contentType: "application/json", body: "{\"ok\":true}" }); });
    const before = await state(page);
    check(`AC-117 ${at} no line: no counter`, !before.failed.shown && !before.waiting.shown, JSON.stringify(before));

    f.say(marks([...FAILED.slice(0, 4), line("T-9", true, "T-9 failed"), ...WAITING]));
    await until(async () => (await state(page)).failed.n === "5");
    const folded = await state(page);
    check(`AC-117 ${at} the lines fold into a red and a yellow counter, said in words`, folded.failed.shown && folded.failed.n === "5" && folded.failed.label === "5 failed"
      && folded.waiting.shown && folded.waiting.n === "2" && folded.waiting.label === "2 wait for you", JSON.stringify(folded));
    check(`AC-117 ${at} no line above the terminal, its rows unchanged`, folded.asksH === 0 && folded.termH === before.termH, JSON.stringify({ before: before.termH, folded }));
    check(`AC-117 ${at} the counters stand left of ⟨…⟩ on its row`, folded.beside, JSON.stringify(folded));
    check(`AC-117 ${at} lines not looked at ring both counters`, folded.failed.fresh && folded.waiting.fresh, JSON.stringify(folded));

    await page.click('#askcount [data-kind="failed"]');
    await until(async () => (await state(page)).sheet);
    const open = await state(page);
    check(`AC-117 ${at} a counter opens its own lines only, as a sheet`, open.sheet && open.rows.length === 5 && open.rows.every((w) => w.includes("failed")), JSON.stringify(open.rows));
    check(`AC-117 ${at} a line without a reason has no hanging colon`, open.rows.includes("T-9 failed"), JSON.stringify(open.rows));
    check(`AC-117 ${at} opened: its ring goes, the other's stays`, !open.failed.fresh && open.waiting.fresh, JSON.stringify(open));
    await page.keyboard.press("Escape");
    check(`AC-117 ${at} Esc closes the sheet`, await until(async () => !(await state(page)).sheet && (await state(page)).asksH === 0), JSON.stringify(await state(page)));

    f.say(marks([...FAILED, ...WAITING]));
    await until(async () => (await state(page)).failed.n === "14");
    check(`AC-117 ${at} a new line rings its counter again`, (await state(page)).failed.fresh, JSON.stringify(await state(page)));
    await page.click('#askcount [data-kind="failed"]');
    await until(async () => (await state(page)).sheet);
    const many = await state(page);
    check(`AC-117 ${at} many lines: at most half the visible height, scrolling inside, words whole`, many.half && many.scrolls && many.whole, JSON.stringify({ ...many, rows: many.rows.length }));
    await page.mouse.click(8, 8);                  // the shade, above the sheet
    check(`AC-117 ${at} a tap outside closes it`, await until(async () => !(await state(page)).sheet), JSON.stringify(await state(page)));

    await page.click('#askcount [data-kind="waiting"]');
    await until(async () => (await state(page)).sheet);
    await page.click("#asks .ask:not(:has(.state.failed)) .x");
    check(`AC-117 ${at} a line's × in the sheet closes it in terminumd`, await until(() => posts.length === 1) && posts[0] === "/api/tasks/T-1/seen", JSON.stringify(posts));
    f.say(marks([...FAILED, WAITING[1]]));
    await until(async () => (await state(page)).waiting.n === "1");
    f.say(marks(FAILED));
    check(`AC-117 ${at} its last line going closes the sheet and hides its counter`, await until(async () => { const s = await state(page); return !s.sheet && !s.waiting.shown && s.failed.shown; }), JSON.stringify(await state(page)));
    f.say(marks([]));
    check(`AC-117 ${at} none left: no counter`, await until(async () => { const s = await state(page); return !s.failed.shown && !s.waiting.shown; }), JSON.stringify(await state(page)));
    // the agents that wait in panes not shown (terminum-agents.md AC-39) fold into the same counter, every one listed
    const asking = (t) => item(`tmux:local:terminum:%${t}`, `asking ${t}`, { index: String(t), state: "waiting" });
    f.say({ t: "layout", groups: [group("main", [item("tmux:local:terminum:%1", "api"), asking(4), asking(5)])] });
    check(`AC-39 ${at} waiting agents count on the yellow counter, each in its list`, await until(async () => (await state(page)).waiting.n === "2", 6000), JSON.stringify(await state(page)));
    await page.click('#askcount [data-kind="waiting"]');
    await until(async () => (await state(page)).sheet);
    check(`AC-39 ${at} its list holds every waiting agent, none cut to "+N more"`, JSON.stringify((await state(page)).rows) === JSON.stringify(["asking 4 waits for your answer", "asking 5 waits for your answer"]), JSON.stringify((await state(page)).rows));
    await page.keyboard.press("Escape");
    check(`no page error ${at}`, f.errors.length === 0, f.errors.join("; "));
    await f.context.close();
  }
  // a computer with a mouse keeps the lines above the terminal, no counter
  runningIn(`${name} 1400×900`);
  const f = await openPage(browser, { device: WIDE });
  await drawn(f.page);
  f.say(marks([FAILED[0], WAITING[0]]));
  await until(async () => (await state(f.page)).rows.length === 2);
  const wide = await state(f.page);
  check(`AC-117 (${name} 1400×900) a computer keeps its lines, no counter`, wide.rows.length === 2 && wide.asksH > 0 && !wide.failed.shown && !wide.waiting.shown, JSON.stringify(wide));
  check(`no page error (${name} 1400×900)`, f.errors.length === 0, f.errors.join("; "));
  await f.context.close();
}
