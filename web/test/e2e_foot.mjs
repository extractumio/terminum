// The list's foot (AC-07, foot.css, foot.js) against a fake terminumd (fake.mjs): on a computer one
// slim row of its buttons' icons with ▸, a toggle: a click (or Enter, Space) unfolds it upward (▾)
// and it stays until clicked again, the pointer changing nothing; folded, its buttons are out of the
// keyboard's way; folding closes a form of theirs; the state is remembered; moving as it unfolds;
// on a phone the row of blobs, never folded.
import { check, runningIn } from "./check.mjs";
import { drawn, openPage, pause, PHONE, WIDE, pressFoot } from "./fake.mjs";

const BUTTONS = ["newsessbtn", "marksbtn", "addhostbtn", "addtmuxbtn"];

/** The foot's height, its menu's, the toggle row's, how much of each button shows inside the menu,
 *  the toggle's state and its triangle's turn. */
const measure = (page) => page.evaluate((ids) => {
  const h = (sel) => Math.round(document.querySelector(sel).getBoundingClientRect().height);
  const menu = document.querySelector(".footmenu").getBoundingClientRect(), t = document.getElementById("foottoggle");
  const shown = ids.map((id) => {
    const b = document.getElementById(id).getBoundingClientRect();
    return Math.max(0, Math.min(b.bottom, menu.bottom) - Math.max(b.top, menu.top));
  });
  return { foot: h(".nav-foot"), menu: h(".footmenu"), peek: h(".footpeek"), shown, expanded: t.getAttribute("aria-expanded"),
    tri: getComputedStyle(t.querySelector(".tri")).transform, inert: document.getElementById("footmenu").inert };
}, BUTTONS);

const away = (page) => page.mouse.move(1000, 450);   // over the terminal
const folded = (m) => m.menu === 0 && m.shown.every((s) => s === 0) && m.peek >= 24 && m.foot <= 50 && m.expanded === "false" && m.inert && m.tri === "none";
const open = (m) => m.peek >= 24 && m.shown.every((s) => s >= 30) && m.expanded === "true" && !m.inert && m.tri !== "none";

export async function webFoot(browser, name) {
  runningIn(`${name} 1400×900`);
  const f = await openPage(browser, { device: WIDE });
  const { page } = f;
  await drawn(page);
  await away(page);
  await pause(page, 500);
  const rest = await measure(page);
  const icons = await page.evaluate(() => [...document.querySelector(".footpeek .icons").children].map((e) => e.tagName.toLowerCase() === "span" ? [...e.children].map((c) => c.id || c.tagName.toLowerCase()).join("+") : e.tagName.toLowerCase()));
  const named = await page.evaluate(() => { const t = document.getElementById("foottoggle"); return [t.tagName, t.getAttribute("aria-label"), t.getAttribute("aria-controls")]; });
  check("AC-07 the foot at rest on a computer: one slim toggle row of its four buttons' icons (the bookmarks' count after theirs) with ▸, the buttons folded away",
    folded(rest) && JSON.stringify(icons) === JSON.stringify(["svg", "svg+markpeek", "svg", "svg"]) && JSON.stringify(named) === JSON.stringify(["BUTTON", "Session tools", "footmenu"]),
    JSON.stringify({ rest, icons, named }));

  await page.hover(".footpeek");
  await pause(page, 500);
  check("AC-07 the pointer over the row unfolds nothing", folded(await measure(page)));
  await page.click("#foottoggle");
  await pause(page, 400);
  const clicked = await measure(page);
  await away(page);
  await pause(page, 600);
  const stays = await measure(page);
  check("AC-07 a click unfolds New session, Bookmarks / Reminders, Add host and Attach tmux sessions (▾), and they stay with the pointer gone",
    open(clicked) && clicked.foot > rest.foot + 100 && open(stays), JSON.stringify({ clicked, stays }));

  await page.reload();
  await drawn(page);
  await pause(page, 300);
  const reloaded = await measure(page);
  check("AC-07 the page remembers the foot unfolded for the next load", open(reloaded), JSON.stringify(reloaded));

  await page.click("#addhostbtn");
  await pause(page, 300);
  const formOpen = await page.evaluate(() => !document.getElementById("addhost").hidden);
  await page.click("#foottoggle");
  await pause(page, 500);
  const closing = { ...(await measure(page)), form: await page.evaluate(() => !document.getElementById("addhost").hidden) };
  check("AC-07 a click on the row while a form of theirs is open folds the foot and closes the form",
    formOpen && folded(closing) && !closing.form, JSON.stringify(closing));

  await page.focus("#foottoggle");
  await page.keyboard.press("Enter");
  await pause(page, 400);
  const entered = await measure(page);
  await page.keyboard.press("Space");
  await pause(page, 400);
  const spaced = await measure(page);
  await page.focus("#foottoggle");
  await page.keyboard.press("Tab");
  const next = await page.evaluate(() => document.activeElement?.id ?? document.activeElement?.tagName);
  check("AC-07 Enter and Space on the row toggle it; folded, Tab passes its buttons by",
    open(entered) && folded(spaced) && !["newsessbtn", "marksbtn", "addhostbtn", "addtmuxbtn"].includes(next), JSON.stringify({ entered, spaced, next }));
  await page.evaluate(() => document.activeElement?.blur());

  // the foot's New session (a tmux session with a shell): its form above the foot; the list head's [+ Session] starts
  // an agent's session instead (terminum-dispatcher.md AC-142: e2e_navsession.mjs) and never opens this form
  const where = () => page.evaluate(() => {
    const r = (id) => document.getElementById(id).getBoundingClientRect(), form = document.getElementById("newsess");
    return { open: !form.hidden, aboveFoot: !form.hidden && r("newsess").bottom <= document.querySelector(".nav-foot").getBoundingClientRect().top + 1 && r("newsess").top >= r("sessions").bottom - 1,
      focus: document.activeElement?.id };
  });
  await pressFoot(page, "#newsessbtn");
  await pause(page, 300);
  const foot = await where();
  await page.fill("#newsessname", "scratch");
  const sentAt = f.sent.length;
  await page.keyboard.press("Enter");
  await pause(page, 200);
  const made = f.sent.slice(sentAt).find((m) => m.t === "newsession");
  await page.keyboard.press("Escape");
  await pause(page, 400);
  const shut = await where();
  check("AC-07 the foot's New session opens its form above the foot, its field in focus; Create asks terminumd; Esc closes it",
    foot.open && foot.aboveFoot && foot.focus === "newsessname" && made?.name === "scratch" && !shut.open, JSON.stringify({ foot, made, shut }));
  const counts = await page.evaluate(() => [document.getElementById("markcount").textContent, document.getElementById("markpeek").textContent]);
  check("AC-07 the folded row's count is the bookmarks button's", counts[0] === counts[1], JSON.stringify(counts));
  check("AC-07 no page errors (the foot)", !f.errors.length, f.errors.join(" | "));
  await f.close();

  // motion on: it moves as it unfolds, not at once. Its transition (CSS) is paused and stepped through
  // every 16 ms once it starts, so a busy browser's stalled frames never hide the heights between
  const g = await openPage(browser, { device: { ...WIDE, reducedMotion: "no-preference" } });
  await drawn(g.page);
  await away(g.page);
  await pause(g.page, 500);
  const sampling = g.page.evaluate(() => new Promise((done) => {
    const foot = document.querySelector(".nav-foot"), menu = document.querySelector(".footmenu"), seen = [];
    const watch = new MutationObserver(() => {
      watch.disconnect();
      getComputedStyle(menu).gridTemplateRows;                 // its style flushed: its transition begins
      const all = menu.getAnimations({ subtree: true });
      all.forEach((a) => a.pause());
      for (let t = 0; ; t = Math.min(t + 16, 700)) { all.forEach((a) => { a.currentTime = t; }); seen.push(Math.round(menu.getBoundingClientRect().height)); if (t === 700) break; }
      all.forEach((a) => a.finish());
      done(seen);
    });
    watch.observe(foot, { attributes: true, attributeFilter: ["class"] });
    setTimeout(() => done(seen), 3000);                         // it never opened: the check says so
  }));
  await g.page.click("#foottoggle");
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
