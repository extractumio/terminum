// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Docked Files' width against a fake terminumd (fake.mjs), on a wide page: the band between the
// terminal and Files is a separator; a drag sets Files' width, the terminal follows at once and tmux
// once the drag rests; kept across a reload; within 260 px and 60% of the column; ← → and a double
// click (terminum-files.md AC-70).

import { check, runningIn } from "./check.mjs";
import { drawn, openPage, pause, WIDE } from "./fake.mjs";

export async function webDock(browser, name) {
  runningIn(`${name} 1400×900`);
  const f = await openPage(browser, { device: WIDE });
  const { page, sent } = f;
  await drawn(page);
  await page.waitForFunction(() => document.body.classList.contains("docked"));
  await pause(page, 500);
  const state = () => page.evaluate(() => {
    const $ = (id) => document.getElementById(id);
    return { files: $("files").getBoundingClientRect().width, term: $("term").getBoundingClientRect().width,
      main: $("main").clientWidth, now: Number($("dockgrip").getAttribute("aria-valuenow")),
      role: $("dockgrip").getAttribute("role"), stored: localStorage.getItem("sideWidth") };
  });
  const grip = async () => { const b = await page.locator("#dockgrip").boundingBox(); return { x: b.x + b.width / 2, y: b.y + b.height / 2 }; };
  const drag = async (dx, steps = 12) => {
    const g = await grip();
    await page.mouse.move(g.x, g.y);
    await page.mouse.down();
    for (let i = 1; i <= steps; i++) { await page.mouse.move(g.x + (dx * i) / steps, g.y); await pause(page, 20); }
    await page.mouse.up();
  };
  const sizes = (from) => sent.slice(from).filter((m) => m.t === "size").map((m) => m.cols);

  const s0 = await state();
  const from = sent.length;
  await drag(-120);
  const s1 = await state();
  const rightAway = sizes(from).length;
  await pause(page, 700);
  const after = sizes(from);
  check("AC-70 the band between the terminal and Files is a separator, at 380 px at first",
    s0.role === "separator" && s0.now === 380 && Math.abs(s0.files - 380) <= 1, JSON.stringify(s0));
  check("AC-70 dragging the separator left widens Files and narrows the terminal at once",
    s1.now >= 490 && s1.now <= 510 && s1.files > s0.files + 100 && s1.term < s0.term - 100, JSON.stringify({ s0, s1 }));
  check("AC-70 tmux gets the new width once the drag rests: one \"size\", not one per move",
    rightAway === 0 && after.length === 1, JSON.stringify({ rightAway, after }));

  await page.reload();
  await drawn(page);
  await page.waitForFunction(() => document.body.classList.contains("docked"));
  await pause(page, 300);
  const s2 = await state();
  check("AC-70 the width is kept in this browser across a reload", s2.now === s1.now && Math.abs(s2.files - s1.files) <= 1, JSON.stringify({ s1, s2 }));

  await drag(600);
  const lo = await state();
  await drag(-900);
  const hi = await state();
  const cols = await page.evaluate(() => {   // the terminal's columns at the chosen size, as sizing.js sends them
    const t = document.getElementById("term"), cs = getComputedStyle(t);
    const w = t.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
    const probe = Object.assign(document.createElement("span"), { textContent: "M".repeat(100) });
    t.append(probe); const c = probe.getBoundingClientRect().width / 100; probe.remove();
    return Math.floor(w / c);
  });
  check("AC-70 Files is never narrower than 260 px", lo.now === 260 && Math.abs(lo.files - 260) <= 1, JSON.stringify(lo));
  check("AC-70 Files is never wider than 60% of the column, the terminal never under 45 columns",
    hi.now <= Math.round(hi.main * 0.6) && hi.now > 500 && cols >= 45, JSON.stringify({ hi, cols }));

  await page.locator("#dockgrip").dblclick();
  const d = await state();
  check("AC-70 a double click puts back 380 px, kept", d.now === 380 && d.stored === "380", JSON.stringify(d));

  await page.locator("#dockgrip").focus();
  await page.keyboard.press("ArrowLeft");
  const k1 = await state();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  const k2 = await state();
  check("AC-70 ← → move the separator 16 px (← widens Files), kept", k1.now === 396 && k2.now === 364 && k2.stored === "364",
    JSON.stringify({ k1, k2 }));

  // a narrower window holds it within its bounds; the chosen width comes back when it widens
  await page.setViewportSize({ width: 1700, height: 900 });
  await pause(page, 200);
  await drag(-900);
  const chosen = await state();
  await page.setViewportSize({ width: 1400, height: 900 });
  await pause(page, 200);
  const narrow = await state();
  await page.setViewportSize({ width: 1700, height: 900 });
  await pause(page, 200);
  const back = await state();
  check("AC-70 a narrower window clamps Files again; the chosen width comes back when it widens",
    narrow.now < chosen.now && narrow.now <= Math.round(narrow.main * 0.6) && back.now === chosen.now, JSON.stringify({ chosen, narrow, back }));
  check("AC-70 no page errors (dock)", !f.errors.length, f.errors.join(" | "));
  await f.close();
}
