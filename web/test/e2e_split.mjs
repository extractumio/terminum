// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The view split (AC-88), a working session's pulse and the text size (AC-12) against a fake
// terminumd (fake.mjs) on a computer: ⌘D asks for the pane's quick shell on the right, ⌘⇧D and
// the button under the magnifier below, before the browser's own ⌘D; a quick shell on the right
// stands right of the terminal; a working session's bar pulses also with Reduce Motion; the
// text is the theme's size, − and + a step on it, also when an older page kept a size of its own.

import { check, runningIn } from "./check.mjs";
import { drawn, group, item, openPage, pause, PHONE, THEME, WIDE } from "./fake.mjs";

const SID = "tmux:local:terminum:%1", QUICK = "tmux:local:terminum:%9";
const LAYOUT = [group("main", [item(SID, "api", { agent: "claude", shell: false, state: "working", sub: ["claude", "~/work/api"] }),
  item("tmux:local:terminum:%2", "docs", { index: "2" })])];

export async function webSplit(browser, name) {
  runningIn(`${name} 1400×900`);
  let size = 15, shown = null;
  const f = await openPage(browser, { device: WIDE, layout: LAYOUT,
    hook: (m, say) => { if (m.t === "sub") { shown ??= m.id; say({ t: "theme", sid: m.id, theme: { ...THEME, size } }); } } });
  const { page, sent } = f;
  await page.evaluate(() => localStorage.setItem("size", "13"));   // what an older page kept: a size of its own
  await page.reload();
  await drawn(page);
  await page.waitForFunction(() => document.getElementById("sizeval").textContent === "15", null, { timeout: 3000 }).catch(() => {});

  // ⌘D, ⌘⇧D: the quick shell on the right, below; the browser's own ⌘D does not come
  // read once the event went its way (the page stops it where it takes it)
  await page.evaluate(() => { window.prevented = []; addEventListener("keydown", (e) => { if (e.code === "KeyD") setTimeout(() => window.prevented.push(e.defaultPrevented)); }, true); });
  const asked = async (keys) => { const n = sent.length; await page.keyboard.press(keys); await pause(page, 100); return sent.slice(n).filter((m) => m.t === "quick").map((m) => m.side); };
  await page.focus("#kbd");
  const right = await asked("Meta+d"), below = await asked("Meta+Shift+d");
  const n = sent.length;
  await page.click("#splitfab");
  await pause(page, 100);
  const button = sent.slice(n).filter((m) => m.t === "quick").map((m) => m.side);
  const prevented = await page.evaluate(() => window.prevented);
  const typed = sent.filter((m) => m.t === "in" && /d/i.test(m.data)).length;
  check("split: ⌘D asks for the quick shell on the right, ⌘⇧D and the button under the magnifier below; the browser's ⌘D is prevented, the terminal gets no d",
    right.join() === "right" && below.join() === "bottom" && button.join() === "bottom" && prevented.length === 2 && prevented.every(Boolean) && !typed,
    JSON.stringify({ right, below, button, prevented, typed }));
  const under = await page.evaluate(() => { const s = document.getElementById("splitfab").getBoundingClientRect(), m = document.getElementById("findfab").getBoundingClientRect();
    return { below: s.top >= m.bottom, aligned: Math.abs(s.left - m.left) < 1, round: !getComputedStyle(document.getElementById("splitfab")).borderRadius.startsWith("50%") }; });
  check("split: its button stands under the magnifier, a blob as it", under.below && under.aligned && under.round, JSON.stringify(under));

  // (before a quick shell shows: the fake's say() goes to the page connected last, its frame's)
  // the text size: the theme's, not the size an older page kept; + a step on it, which a new theme size keeps
  const first = await page.evaluate(() => document.getElementById("sizeval").textContent);
  await page.click("#menubtn");
  await page.click("#bigger");
  await page.keyboard.press("Escape");
  const stepped = await page.evaluate(() => [document.getElementById("sizeval").textContent, localStorage.getItem("sizeStep")]);
  size = 17;
  f.say({ t: "theme", sid: shown, theme: { ...THEME, size } });
  await pause(page, 200);
  const themed = await page.evaluate(() => getComputedStyle(document.getElementById("term")).fontSize);
  check("text size: the theme's (15), not an older page's own; + is a step on it, kept when the theme's size changes",
    first === "15" && stepped[0] === "16" && stepped[1] === "1" && themed === "18px", JSON.stringify({ first, stepped, themed }));
  // a quick shell on the right: right of the terminal, its edge between them
  f.say({ t: "layout", groups: [group("main", [{ ...LAYOUT[0].items[0], quick: QUICK, quickSide: "right" }, LAYOUT[0].items[1],
    item(QUICK, "quick shell", { index: "3", quickOf: SID, sub: ["zsh", "~/work/api", "quick shell"] })])] });
  await page.waitForFunction(() => !document.getElementById("quick").hidden, null, { timeout: 3000 }).catch(() => {});
  const side = await page.evaluate(() => { const t = document.getElementById("term").getBoundingClientRect(), q = document.getElementById("quick").getBoundingClientRect();
    return { right: document.getElementById("termrow").classList.contains("right"), beside: q.left >= t.right - 1 && Math.abs(q.top - t.top) < 2 }; });
  check("split: a quick shell on the right stands right of the terminal", side.right && side.beside, JSON.stringify(side));

  // a working session's bar pulses, also with Reduce Motion (the checks run with it): a fade, nothing moves
  const pulse = await page.evaluate(() => [...document.querySelectorAll("#sessions .pbar.busy, #curbar .pbar.busy")].map((b) => getComputedStyle(b).animationName));
  check("a working session's bar pulses (in the list and the header), with Reduce Motion too", pulse.length >= 2 && pulse.every((a) => a === "pulse-bar"), JSON.stringify(pulse));

  check("split: no page errors", !f.errors.length, f.errors.join(" | "));
  await f.close();

  // a phone: no quick shell beside the terminal; the button under the magnifier shows it alone, and on it leads back
  runningIn(`${name} 390×844`);
  const withQuick = [group("main", [{ ...LAYOUT[0].items[0], quick: QUICK }, LAYOUT[0].items[1],
    item(QUICK, "quick shell", { index: "3", quickOf: SID, sub: ["zsh", "~/work/api", "quick shell"] })])];
  const p = await openPage(browser, { device: PHONE, layout: withQuick });
  await drawn(p.page);
  await p.page.click("#sessions .item[data-id='" + SID + "']").catch(() => {});      // the drawer may be closed: pick the pane
  if (await p.page.evaluate((id) => !document.querySelector(`#sessions .item.on[data-id="${id}"]`), SID)) {
    await p.page.click("#navbtn"); await pause(p.page, 300); await p.page.click(`#sessions .item[data-id="${SID}"]`); }
  await pause(p.page, 300);
  const fab = await p.page.evaluate(() => { const b = document.getElementById("splitfab"), r = b.getBoundingClientRect(); return { shown: !b.hidden && r.height >= 44, label: b.getAttribute("aria-label") }; });
  const from = p.sent.length;
  await p.page.click("#splitfab");
  await pause(p.page, 200);
  const opened = p.sent.slice(from).filter((m) => m.t === "quick").map((m) => m.id);
  check("split on a phone: the button under the magnifier (finger-sized) opens the pane's quick shell, shown alone",
    fab.shown && /Quick shell/.test(fab.label) && opened.join() === SID, JSON.stringify({ fab, opened }));
  p.say({ t: "quickmade", id: QUICK });                                   // terminumd made it (or found it): the phone shows it
  await p.page.waitForFunction((id) => document.querySelector(`#sessions .item.on[data-id="${id}"]`), QUICK, { timeout: 3000 }).catch(() => {});
  const onQuick = await p.page.evaluate(() => document.getElementById("splitfab").getAttribute("aria-label"));
  await p.page.click("#splitfab");
  await p.page.waitForFunction((id) => document.querySelector(`#sessions .item.on[data-id="${id}"]`), SID, { timeout: 3000 }).catch(() => {});
  const back = await p.page.evaluate(() => document.querySelector("#sessions .item.on")?.dataset.id);
  check("split on a phone: on the quick shell the button leads back to its pane", onQuick === "Back to its pane" && back === SID, JSON.stringify({ onQuick, back }));
  check("split on a phone: no page errors", !p.errors.length, p.errors.join(" | "));
  await p.close();
  runningIn(name);
}
