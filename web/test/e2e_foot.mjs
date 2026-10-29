// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The list's foot (AC-07, foot.css) against a fake terminumd (fake.mjs): on a computer one slim row
// of its buttons' icons, unfolded upward by the pointer (and by the keyboard's focus), kept while the
// pointer is there or a form of theirs is open, folded 0.3 s after the pointer left, moving as it
// unfolds; on a phone the row of blobs, never folded.
import { check, runningIn } from "./check.mjs";
import { drawn, openPage, pause, PHONE, WIDE, pressFoot } from "./fake.mjs";

const BUTTONS = ["newsessbtn", "marksbtn", "addhostbtn", "addtmuxbtn"];

/** The foot's height, its menu's, the peek's, and how much of each button shows inside the menu. */
const measure = (page) => page.evaluate((ids) => {
  const h = (sel) => Math.round(document.querySelector(sel).getBoundingClientRect().height);
  const menu = document.querySelector(".footmenu").getBoundingClientRect();
  const shown = ids.map((id) => {
    const b = document.getElementById(id).getBoundingClientRect();
    return Math.max(0, Math.min(b.bottom, menu.bottom) - Math.max(b.top, menu.top));
  });
  return { foot: h(".nav-foot"), menu: h(".footmenu"), peek: h(".footpeek"), shown };
}, BUTTONS);

const away = (page) => page.mouse.move(1000, 450);   // over the terminal
const folded = (m) => m.menu === 0 && m.shown.every((s) => s === 0) && m.peek >= 24 && m.foot <= 50;
const open = (m) => m.peek === 0 && m.shown.every((s) => s >= 30);

export async function webFoot(browser, name) {
  runningIn(`${name} 1400×900`);
  const f = await openPage(browser, { device: WIDE });
  const { page } = f;
  await drawn(page);
  await away(page);
  await pause(page, 500);
  const rest = await measure(page);
  const icons = await page.evaluate(() => [...document.querySelector(".footpeek").children].map((e) => e.tagName.toLowerCase() === "span" ? [...e.children].map((c) => c.id || c.tagName.toLowerCase()).join("+") : e.tagName.toLowerCase()));
  check("AC-07 the foot at rest on a computer: one slim row of its four buttons' icons (the bookmarks' count after theirs), the buttons folded away",
    folded(rest) && JSON.stringify(icons) === JSON.stringify(["svg", "svg+markpeek", "svg", "svg"]), JSON.stringify({ rest, icons }));

  await page.hover(".footpeek");
  await pause(page, 400);
  const hovered = await measure(page);
  check("AC-07 the pointer over it unfolds New session, Bookmarks / Reminders, Add host and Attach tmux sessions",
    open(hovered) && hovered.foot > rest.foot + 100, JSON.stringify(hovered));
  await away(page);
  await pause(page, 150);
  const leaving = await measure(page);
  await pause(page, 450);
  const left = await measure(page);
  check("AC-07 the pointer gone, the foot stays 0.3 s, then folds", open(leaving) && folded(left), JSON.stringify({ leaving, left }));

  await page.hover(".footpeek");
  await pause(page, 400);
  await pressFoot(page, "#addhostbtn");
  await away(page);
  await pause(page, 600);
  const form = await measure(page);
  await page.keyboard.press("Escape");
  await pause(page, 600);
  const closed = await measure(page);
  check("AC-07 a button pressed: its form open keeps the foot unfolded with the pointer gone; closed, it folds",
    open(form) && folded(closed), JSON.stringify({ form, closed }));

  await page.keyboard.press("Shift");               // the keyboard's focus: a focus ring shows
  await page.focus("#addtmuxbtn");
  await pause(page, 400);
  const keyed = await measure(page);
  await page.evaluate(() => document.activeElement.blur());
  await pause(page, 600);
  check("AC-07 the keyboard's focus on one of its buttons unfolds it", open(keyed) && folded(await measure(page)), JSON.stringify(keyed));
  // [+] in the list's head: New session's form under the head, the foot folded; the foot's button: above the foot again
  const where = () => page.evaluate(() => {
    const r = (id) => document.getElementById(id).getBoundingClientRect(), form = document.getElementById("newsess");
    return { open: !form.hidden, underHead: !form.hidden && r("newsess").top >= document.querySelector(".nav-head").getBoundingClientRect().bottom - 1 && r("newsess").bottom <= r("sessions").top + 1,
      aboveFoot: !form.hidden && r("newsess").bottom <= document.querySelector(".nav-foot").getBoundingClientRect().top + 1 && r("newsess").top >= r("sessions").bottom - 1,
      focus: document.activeElement?.id, plus: document.getElementById("navnew").ariaExpanded,
      keys: ["navnew", "navclose"].map((id) => [Math.round(r(id).left), Math.round(r(id).top), Math.round(r(id).width), Math.round(r(id).height)]) };
  });
  await page.click("#navnew");
  await pause(page, 300);
  const head = { ...(await where()), foot: await measure(page) };
  await page.fill("#newsessname", "scratch");
  const sentAt = f.sent.length;
  await page.keyboard.press("Enter");
  await pause(page, 200);
  const made = f.sent.slice(sentAt).find((m) => m.t === "newsession");
  await page.keyboard.press("Escape");
  await pause(page, 200);
  const shut = await where();
  check("AC-07 [+] left of the list's hide button, as large: New session's form under the list's head, its field in focus, the foot folded; Create asks terminumd; Esc closes it",
    head.open && head.underHead && head.focus === "newsessname" && head.plus === "true" && folded(head.foot) && made?.name === "scratch" && !shut.open && shut.plus === "false"
    && head.keys[0][0] < head.keys[1][0] && head.keys[0][1] === head.keys[1][1] && head.keys[0][2] === head.keys[1][2] && head.keys[0][3] === head.keys[1][3], JSON.stringify({ head, made, shut }));
  await page.hover(".footpeek");
  await pause(page, 400);
  await pressFoot(page, "#newsessbtn");
  await pause(page, 300);
  const foot = await where();
  await page.keyboard.press("Escape");
  await pause(page, 400);
  check("AC-07 the foot's New session opens the same form above the foot again", foot.open && foot.aboveFoot && foot.plus === "false", JSON.stringify(foot));
  await away(page);
  await pause(page, 600);
  const counts = await page.evaluate(() => [document.getElementById("markcount").textContent, document.getElementById("markpeek").textContent]);
  check("AC-07 the folded row's count is the bookmarks button's", counts[0] === counts[1], JSON.stringify(counts));
  check("AC-07 no page errors (the foot)", !f.errors.length, f.errors.join(" | "));
  await f.close();

  // motion on: it moves as it unfolds (each frame's height sampled), not at once
  const g = await openPage(browser, { device: { ...WIDE, reducedMotion: "no-preference" } });
  await drawn(g.page);
  await away(g.page);
  await pause(g.page, 500);
  const sampling = g.page.evaluate(() => new Promise((done) => {
    const menu = document.querySelector(".footmenu"), seen = [], t0 = performance.now();
    const tick = () => { seen.push(Math.round(menu.getBoundingClientRect().height)); performance.now() - t0 < 700 ? requestAnimationFrame(tick) : done(seen); };
    requestAnimationFrame(tick);
  }));
  await g.page.hover(".footpeek");
  const seen = await sampling, full = Math.max(...seen);
  check("AC-07 with motion on the foot unfolds over several frames to its full height",
    full > 150 && seen.at(-1) === full && seen.filter((h) => h > 10 && h < full - 10).length >= 2, JSON.stringify(seen));
  check("AC-07 no page errors (the foot's motion)", !g.errors.length, g.errors.join(" | "));
  await g.close();

  runningIn(`${name} 390×844`);
  const p = await openPage(browser, { device: PHONE });
  await drawn(p.page);
  const phone = await p.page.evaluate((ids) => ({
    peek: getComputedStyle(document.querySelector(".footpeek")).display,
    tops: ids.map((id) => Math.round(document.getElementById(id).getBoundingClientRect().top)),
    heights: ids.map((id) => Math.round(document.getElementById(id).getBoundingClientRect().height)),
  }), BUTTONS);
  check("AC-07 on a phone the foot is its row of four blobs, never folded",
    phone.peek === "none" && new Set(phone.tops).size === 1 && phone.heights.every((h) => h >= 44), JSON.stringify(phone));
  await p.close();
  runningIn(name);
}
