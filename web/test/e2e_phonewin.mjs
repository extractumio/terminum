// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A phone's windows (terminum.md AC-173 "a phone's windows cover the screen", AC-184's phone case) against a fake
// terminumd: at 600 px or narrower a window to type in, with fields, a list or a document covers what is in sight,
// its ways on in a bar at its top with × and the title, what is seldom pressed in a row under its fields, a text
// field with no full-window key; a question stays a sheet; 601 px and wider keeps the sheets, and a window resized
// between the two gets its keys back in their order; a row of keys filled anew (a task's) fills the bar anew.
import fs from "node:fs";
import path from "node:path";
import { check, runningIn } from "./check.mjs";
import { drawn, openPage, pause, PHONE, pressFoot } from "./fake.mjs";

/** Opens dialog `id` alone and measures it: its box, its head's parts, its bar's keys and what is left at its foot. */
const open = (page, id) => page.evaluate(async (id) => {
  (await import("/taskboxes.js")).taskBoxes();
  for (const d of document.querySelectorAll("dialog[open]")) d.close();
  const d = document.getElementById(id);
  d.showModal();
  await new Promise((r) => setTimeout(r, 60));
  return window.measure(id);
}, id);
const MEASURE = () => {
  window.measure = (id) => {
    const d = document.getElementById(id);
    const box = (e) => { const r = e.getBoundingClientRect(); return { l: Math.round(r.left), r: Math.round(r.right), t: Math.round(r.top), b: Math.round(r.bottom), h: Math.round(r.height), shown: r.width > 0 && r.height > 0 }; };
    const seen = (e) => box(e).shown, words = (e) => e.textContent.trim();
    const foot = d.querySelector(":scope > form > .actions:not(.barkeys), :scope > .actions:not(.barkeys)");
    return { win: [innerWidth, Math.round(visualViewport.height)], box: box(d), head: box(d.querySelector(".dhead")), x: box(d.querySelector(".dhead .x")), title: box(d.querySelector(".dhead h2")),
      bar: [...d.querySelectorAll(".dhead > .barkeys > button")].filter(seen).map((k) => ({ words: words(k), ...box(k) })),
      foot: foot ? [...foot.children].filter(seen).map(words) : [], footTop: foot && seen(foot) ? box(foot).t : null,
      text: d.querySelector("textarea") ? box(d.querySelector("textarea")) : null, radius: getComputedStyle(d).borderTopLeftRadius };
  };
};
const covers = (m) => m.box.l === 0 && m.box.t === 0 && m.box.r === m.win[0] && m.box.b === m.win[1] && m.radius === "0px";
/** The bar: at the top, 60 px, × first, then the title, then the keys on its middle, each a finger's, none past the screen's edge. */
const barred = (m) => m.head.t === 0 && m.x.l >= 0 && m.x.r <= m.title.l && m.bar.length > 0 && m.bar[0].l >= m.title.r && m.bar.every((k) => k.h >= 44 && k.r <= m.win[0] && Math.abs(k.t + k.b - m.head.t - m.head.b) <= 2) && m.head.h === 60 && m.x.h >= 44;
const shots = (page, name) => process.env.TM_TEST_SHOTS ? (fs.mkdirSync(process.env.TM_TEST_SHOTS, { recursive: true }), page.screenshot({ path: path.join(process.env.TM_TEST_SHOTS, `phonewin-${name}.png`) })) : null;

export async function webPhoneWin(browser, name) {
  for (const width of [390, 320]) {
    runningIn(`${name} ${width}×700`);
    const f = await openPage(browser, { device: { ...PHONE, viewport: { width, height: 700 } } });
    const { page } = f;
    await drawn(page);
    await page.evaluate(MEASURE);

    let m = await open(page, "runbox");
    check("AC-173 phone: a window to type in covers what is in sight, edge to edge", covers(m), JSON.stringify(m));
    check("AC-173 phone: one bar at its top: ×, the title, then its way on; no Cancel, no foot when nothing else is left", barred(m) && m.bar.map((k) => k.words).join() === "Run" && !m.foot.length && m.footTop === null, JSON.stringify(m));
    check("AC-173 phone: its text field takes the room its other fields leave", m.text.h > m.win[1] / 2, JSON.stringify(m.text));
    await shots(page, `${name}-${width}-run`);

    m = await open(page, "markbox");
    check("AC-173 phone: what is seldom pressed (New task from selection) stays a row under the fields, Save in the bar",
      covers(m) && barred(m) && m.bar.map((k) => k.words).join() === "Save" && m.foot.join() === "New task from selection" && m.footTop >= m.text.b, JSON.stringify(m));

    m = await open(page, "askbox");
    check("AC-173 phone: two ways on stand in the bar, the primary last, the title giving way", covers(m) && barred(m) && m.bar.map((k) => k.words).join() === "Send to Agent,Add to Chat", JSON.stringify(m));
    await shots(page, `${name}-${width}-ask`);

    const keys = await page.evaluate(() => { document.getElementById("asktext").textContent = "a quoted line";
      const seen = (sel) => document.querySelector(sel).getBoundingClientRect().width > 0;
      return { typed: seen("#askbox .xbox:has(#askq) .xkey"), read: seen("#askbox .xbox:has(#asktext) .xkey") }; });
    check("AC-184 phone: a text field to type in has no full-window key; a text shown to read keeps it", !keys.typed && keys.read, JSON.stringify(keys));

    m = await open(page, "pastebox");
    await page.fill("#pastetext", "echo one");
    await page.click("#pastebox .barkeys .primary");
    check("AC-173 phone: the bar's primary is the window's own key: it sends and closes", await page.evaluate(() => !document.getElementById("pastebox").open && document.getElementById("pastetext").value === ""));

    m = await open(page, "confirmbox");
    check("AC-173 phone: a question stays a centered sheet, its buttons at its foot", m.box.l === 16 && m.win[0] - m.box.r === 16 && m.box.h < 300 && !m.bar.length && m.foot.length === 2, JSON.stringify(m));
    await page.keyboard.press("Escape");

    // a short screen (the keyboard up): the bar stays while the fields scroll under it
    await page.setViewportSize({ width, height: 300 });
    await open(page, "convertbox");
    const stuck = await page.evaluate(() => { const d = document.getElementById("convertbox"); d.scrollTop = 400;
      const h = d.querySelector(".dhead").getBoundingClientRect(), at = document.elementFromPoint(innerWidth / 2, h.top + h.height / 2);
      return { scrolled: d.scrollTop, top: Math.round(h.top), over: !!at?.closest(".dhead"), box: Math.round(d.getBoundingClientRect().height), win: Math.round(visualViewport.height) }; });
    check("AC-173 phone: the window is as tall as what is in sight; its bar stays at its top, over what scrolls", stuck.box === stuck.win && stuck.top === 0 && stuck.over && (stuck.scrolled > 0 || stuck.box >= 300), JSON.stringify(stuck));
    await page.keyboard.press("Escape");
    await page.setViewportSize({ width, height: 700 });

    // a row of keys filled anew by its window, as a task's is by what the task allows
    const refilled = await page.evaluate(async () => {
      (await import("/taskboxes.js")).taskBoxes();
      const d = document.getElementById("taskbox"), acts = document.getElementById("taskacts"), key = (words, cls = "") => { const b = document.createElement("button"); b.type = "button"; b.className = cls; b.textContent = words; return b; };
      const save = key("Save", "fromright"), run = key("Run"), names = () => ({ bar: [...d.querySelectorAll(".barkeys > button")].map((b) => b.textContent), foot: [...acts.children].map((b) => b.textContent) });
      acts.replaceChildren(key("Archive"), save, run);
      d.showModal();
      await new Promise((r) => setTimeout(r, 60));
      const first = names();
      acts.replaceChildren(key("Bring back"));         // kanbanwin.js's paint: the seldom keys anew, Save and Run put back
      acts.append(save); save.after(run);
      await new Promise((r) => setTimeout(r, 60));
      const second = names();
      d.close();
      return { first, second };
    });
    check("AC-173 phone: a task's keys: Save and Run in the bar, Archive under the task; filled anew, the bar is too, no key twice",
      JSON.stringify(refilled) === JSON.stringify({ first: { bar: ["Save", "Run"], foot: ["Archive"] }, second: { bar: ["Save", "Run"], foot: ["Bring back"] } }), JSON.stringify(refilled));

    // a list foot's form
    if (!await page.evaluate(() => document.getElementById("app").classList.contains("nav-open"))) await page.click("#navbtn");
    await pause(page, 300);
    await pressFoot(page, "#newsessbtn");
    await pause(page, 200);
    m = await page.evaluate(() => window.measure("newsess"));
    check("AC-173 phone: a list's form (New session) covers the screen, Create in its bar", covers(m) && barred(m) && m.bar.map((k) => k.words).join() === "Create" && !m.foot.length, JSON.stringify(m));
    await shots(page, `${name}-${width}-newsession`);
    await page.click("#newsess .dhead .x");
    check("no page errors", !f.errors.length, f.errors.join(" | "));
    await f.close();

    // the board's New task: Create and Run in the bar, Add files and Discard under the fields
    const b = await openPage(browser, { device: { ...PHONE, viewport: { width, height: 700 } }, url: "/kanban.html" });
    await b.page.evaluate(MEASURE);
    m = await b.page.evaluate(async () => { document.getElementById("newbox").showModal(); await new Promise((r) => setTimeout(r, 60)); return window.measure("newbox"); });
    check("AC-173 phone: New task: Create and Run in the bar, Add files and Discard a row under its fields", covers(m) && barred(m) && m.bar.map((k) => k.words).join() === "Create,Run" && m.foot.join() === "Add files…,Discard" && m.footTop > m.text.b, JSON.stringify(m));
    await shots(b.page, `${name}-${width}-newtask`);
    await b.close();
  }

  // 601 px and wider: a sheet; resized between the two with a window open, its keys move and come back in their order
  runningIn(`${name} 700×700`);
  const f = await openPage(browser, { device: { ...PHONE, viewport: { width: 700, height: 700 } } });
  await drawn(f.page);
  await f.page.evaluate(MEASURE);
  let m = await open(f.page, "runbox");
  const order = () => f.page.evaluate(() => ({ foot: [...document.querySelectorAll("#runbox > form > .actions:not(.barkeys) > button")].map((b) => b.id || b.textContent), bar: [...document.querySelectorAll("#runbox .barkeys > button")].map((b) => b.id) }));
  const wide = await order();
  check("AC-173 a window 601 px or wider keeps the sheet: 16 px from the sides, its keys at its foot, none in its head", m.box.l === 16 && m.win[0] - m.box.r === 16 && !m.bar.length && m.foot.includes("Run") && m.foot.includes("Cancel"), JSON.stringify(m));
  await f.page.setViewportSize({ width: 390, height: 700 });
  await pause(f.page, 200);
  m = await f.page.evaluate(() => window.measure("runbox"));
  const narrow = await order();
  await f.page.setViewportSize({ width: 700, height: 700 });
  await pause(f.page, 200);
  const back = await order();
  check("AC-173 a window made narrow while open covers the screen, its way on in the bar; wide again, its keys stand at its foot in their order",
    covers(m) && narrow.bar.join() === "runqueue,rungo" && narrow.foot.join() === "runadd,Cancel" && JSON.stringify(back) === JSON.stringify(wide) && !back.bar.length, JSON.stringify({ wide, narrow, back }));
  check("no page errors", !f.errors.length, f.errors.join(" | "));
  await f.close();
}
