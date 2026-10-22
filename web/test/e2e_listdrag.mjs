// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Dragging in the session list (AC-07, AC-92) against a fake terminumd: no grips; a mouse drags
// a row by its body once it moves (a press that does not still selects), its slot dashed and
// a copy under the pointer, Esc puts it back; a finger rests 350 ms to lift a row, then drags
// it, or lifts for its menu, and moving sooner drags nothing. Playwright cannot reproduce iOS
// Safari's pan arbitration, so the finger's part is synthetic pointer events.
// Alone: node web/test/e2e_all.mjs webListDrag.

import { check, runningIn } from "./check.mjs";
import { drawn, group, item, openPage, pause, PHONE, WIDE } from "./fake.mjs";

const ID = (n) => `tmux:local:terminum:%${n}`;
const LAYOUT = [group("main", [item(ID(1), "api", { index: "1" }), item(ID(2), "docs", { index: "2" }), item(ID(3), "tests", { index: "3" })],
  { wid: "local/main", new: "New window" }), group("notes", [item(ID(4), "notes", { index: "1" })], { wid: "local/notes", new: "New window" })];
const order = (page) => page.evaluate(() => [...document.querySelectorAll("#sessions li > .item")].map((b) => b.dataset.id.split("%")[1]).join(""));
const mid = (page, n) => page.evaluate((n) => { const r = document.querySelector(`#sessions .item[data-id$="%${n}"]`).getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2, h: r.height }; }, n);
const reorders = (sent) => sent.filter((m) => m.t === "reorder");
const touch = (page, type, n, dy = 0) => page.evaluate(([type, n, dy]) => {
  const el = document.querySelector(`#sessions .item[data-id$="%${n}"]`), r = el.getBoundingClientRect();
  el.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerType: "touch", pointerId: 7, clientX: r.left + 40, clientY: r.top + 10 + dy }));
}, [type, n, dy]);
const midDrag = (page) => page.evaluate(() => {
  const slot = document.querySelector("#sessions .dragging"), ghost = document.querySelector("#sessions .dragghost");
  return { slot: !!slot, dashed: slot && getComputedStyle(slot).outlineStyle, hidden: slot && getComputedStyle(slot.querySelector(".item")).visibility,
    ghost: !!ghost, aria: ghost?.getAttribute("aria-hidden"), inert: !!ghost?.inert, ids: ghost ? ghost.querySelectorAll("[id]").length + (ghost.id ? 1 : 0) : -1,
    pos: ghost && getComputedStyle(ghost).position };
});

export async function webListDrag(browser, name) {
  runningIn(`${name} 1400×900`);
  const f = await openPage(browser, { device: WIDE, layout: LAYOUT });
  const { page, sent } = f;
  await drawn(page);
  check("AC-07 no drag grip in the list; its rows and its session's header can be dragged",
    await page.evaluate(() => !document.querySelector("#sessions .grip") && document.querySelectorAll("#sessions li[data-drag]").length === 4 && !!document.querySelector("#sessions h2[data-drag]")));

  const cur0 = await page.evaluate(() => document.querySelector('#sessions .item[aria-current="true"]')?.dataset.id);
  const a = await mid(page, 2), c = await mid(page, 3);
  await page.mouse.move(a.x, a.y);
  await page.mouse.down();
  await page.mouse.move(a.x, a.y + 4);
  const early = await midDrag(page);
  await page.mouse.move(c.x, c.y + c.h / 4, { steps: 8 });
  const during = await midDrag(page);
  await page.mouse.up();
  await pause(page);
  const moved = reorders(sent).pop();
  check("AC-07 a mouse press that moves 4 px drags nothing yet", !early.slot && !early.ghost, JSON.stringify(early));
  check("AC-07 mid-drag the row's slot is dashed, its content hidden; a copy (no ids, aria-hidden, inert) follows the pointer",
    during.slot && during.dashed === "dashed" && during.hidden === "hidden" && during.ghost && during.aria === "true" && during.inert && during.ids === 0 && during.pos === "fixed", JSON.stringify(during));
  check("AC-07 a row dragged by its body to the end reorders its session's windows",
    moved?.group === "local/main" && moved.ids.map((i) => i.split("%")[1]).join("") === "132" && !(await page.$("#sessions .dragghost")), JSON.stringify(moved));
  const shownAfter = await page.evaluate(() => document.querySelector('#sessions .item[aria-current="true"]')?.dataset.id);

  await page.click('#sessions .item[data-id$="%3"]');
  await pause(page);
  check("AC-07 a click without movement still shows the session (and a drag shows none)",
    (await page.evaluate(() => document.querySelector('#sessions .item[aria-current="true"]')?.dataset.id)) === ID(3) && shownAfter === cur0, `${cur0} ${shownAfter}`);

  const before = await order(page), n = reorders(sent).length, b = await mid(page, 2);
  await page.mouse.move(b.x, b.y);
  await page.mouse.down();
  await page.mouse.move(b.x, b.y + 3 * b.h, { steps: 8 });
  await page.keyboard.press("Escape");
  await page.mouse.up();
  await pause(page);
  check("AC-07 Esc mid-drag puts the row back and sends nothing",
    (await order(page)) === before && reorders(sent).length === n && !(await page.$("#sessions .dragging, #sessions .dragghost")), `${before} ${await order(page)}`);
  const groups = () => page.evaluate(() => [...document.querySelectorAll("#sessions section[data-wid]")].map((s) => `${s.dataset.wid}:${s.querySelector(".toggle").getAttribute("aria-expanded")}`).join());
  const g0 = await groups(), hd = await page.evaluate(() => { const r = document.querySelector('#sessions section[data-wid="local/main"] h2 .toggle').getBoundingClientRect(); return { x: r.left + 40, y: r.top + r.height / 2 }; });
  const last = await page.evaluate(() => { const r = document.querySelector('#sessions section[data-wid="local/notes"]').getBoundingClientRect(); return r.bottom - 6; });
  await page.mouse.move(hd.x, hd.y);
  await page.mouse.down();
  await page.mouse.move(hd.x, last, { steps: 10 });
  await page.mouse.up();
  await pause(page);
  check("AC-52 a session's group dragged by its header moves below the next one, still open (the drag's click dropped)",
    g0 === "local/main:true,local/notes:true" && (await groups()) === "local/notes:true,local/main:true", `${g0} → ${await groups()}`);
  check("no page errors", !f.errors.length, f.errors.join(" | "));
  await f.close();

  runningIn(`${name} 390×844`);
  const p = await openPage(browser, { device: PHONE, layout: LAYOUT });
  await drawn(p.page);
  await p.page.click("#navbtn");
  await pause(p.page, 300);
  const pg = p.page, h = (await mid(pg, 3)).h;
  await touch(pg, "pointerdown", 1);
  await pause(pg, 100);
  await touch(pg, "pointermove", 1, 30);
  await pause(pg, 400);
  const scrolled = await pg.evaluate(() => !!document.querySelector("#sessions .lifted, #sessions .dragging"));
  await touch(pg, "pointerup", 1, 30);
  check("AC-07 a finger that moves before 350 ms lifts nothing (the list scrolls)", !scrolled);

  await touch(pg, "pointerdown", 1);
  await pause(pg, 450);
  const lifted = await pg.evaluate(() => !!document.querySelector("#sessions li.lifted"));
  await touch(pg, "pointermove", 3, h - 14);         // near the last row's bottom
  const fing = await midDrag(pg);
  await touch(pg, "pointerup", 3, h - 14);
  await pause(pg);
  const fmoved = reorders(p.sent).pop();
  check("AC-07 a finger resting 350 ms lifts the row, then drags it: its slot dashed, a copy under the finger",
    lifted && fing.slot && fing.ghost && fing.dashed === "dashed", JSON.stringify({ lifted, fing }));
  check("AC-07 the finger's drag reorders the windows", fmoved?.ids.map((i) => i.split("%")[1]).join("") === "231", JSON.stringify(fmoved));
  check("AC-07 the finger's drag opens no menu", await pg.isHidden("#rowmenu"));

  await touch(pg, "pointerdown", 2);
  await pause(pg, 450);
  await touch(pg, "pointerup", 2);
  await pause(pg, 100);
  check("AC-07 a finger resting and lifted without moving opens the row's menu, the row unmoved",
    (await pg.evaluate(() => document.querySelector("#rowmenu[open] .mhead span")?.textContent)) === "docs" && reorders(p.sent).length === 1);
  check("AC-07 on a phone (no hover) Clone is always there", await pg.evaluate(() => getComputedStyle(document.querySelector("#sessions li > .clone")).opacity === "0.8"));
  check("no page errors", !p.errors.length, p.errors.join(" | "));
  await p.close();
}
