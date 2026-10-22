// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A phone's floating buttons folded into ⟨…⟩ (AC-18, fold.js) against a fake terminumd (fake.mjs):
// a tap unfolds them, 3 s later or a press elsewhere folds them again, find's field opens at the
// top right, ⟨…⟩ carries the bookmarks' count, every floating button hides while a finger scrolls
// the terminal; a computer keeps its column.
import { check, runningIn } from "./check.mjs";
import { drawn, line, openPage, pause, PHONE, WIDE } from "./fake.mjs";

const SID = "tmux:local:terminum:%1";
const COLUMN = ["findfab", "splitfab", "sentfab", "attachfab", "marksfab"];
const MARKS = [{ id: "B1", sid: SID, key: null, n: 1, text: "line 3", note: "", title: "api", host: null, at: 1792000000 },
  { id: "B2", sid: SID, key: null, n: 5, text: "line 5", note: "", title: "api", host: null, at: 1792000001 }];
const seen = (page, ids) => page.evaluate((ids) => Object.fromEntries(ids.map((id) => {
  const e = document.getElementById(id);
  return [id, !e.hidden && getComputedStyle(e).visibility === "visible" && getComputedStyle(e).opacity !== "0"];
})), ids);

export async function webFold(browser, name) {
  runningIn(`${name} 390×844`);
  const f = await openPage(browser, { device: PHONE, screen: Array.from({ length: 80 }, (_, i) => line(`line ${i} of a long answer`)) });
  const { page } = f;
  await drawn(page);
  f.say({ t: "marks", items: MARKS, error: null });
  await pause(page, 400);
  const folded = await seen(page, ["foldfab", ...COLUMN]);
  const badge = await page.evaluate(() => document.getElementById("foldfabn").textContent);
  check("AC-18 a phone: folded, only ⟨…⟩ shows of the column, with the bookmarks' count", folded.foldfab && COLUMN.every((id) => !folded[id]) && badge === "2",
    JSON.stringify({ folded, badge }));

  // a reminder falls due: folded, ⟨…⟩ turns pink with a pink ring at once (fold.js watches the bell), blinking nowhere
  f.say({ t: "marks", items: [...MARKS, { id: "R1", sid: SID, key: SID, n: 2, text: "line 2", note: "check", title: "api", host: null, at: 1792000002, remind: true, due: true }], error: null });
  await pause(page, 300);
  const due = await page.evaluate(() => { const b = document.getElementById("foldfab"); return { remind: b.classList.contains("remind"), due: b.classList.contains("due"), blinks: /remind/.test(getComputedStyle(b).animationName) }; });
  check("AC-18 AC-91 a phone: a reminder due shows on the folded ⟨…⟩ (pink, a ring, no blink) with no other update", due.remind && due.due && !due.blinks, JSON.stringify(due));

  await page.tap("#foldfab");
  await pause(page, 150);
  const open = await seen(page, COLUMN);
  await pause(page, 3300);
  const later = await seen(page, COLUMN);
  check("AC-18 a phone: a tap on ⟨…⟩ shows the column; 3 s later it folds again", COLUMN.every((id) => open[id] && !later[id]), JSON.stringify({ open, later }));

  await page.tap("#foldfab");
  await pause(page, 150);
  await page.touchscreen.tap(60, 500);              // the terminal
  await pause(page, 150);
  const away = await seen(page, COLUMN);
  check("AC-18 a phone: a press elsewhere folds the column at once", COLUMN.every((id) => !away[id]), JSON.stringify(away));

  await page.tap("#foldfab");
  await pause(page, 150);
  await page.tap("#findfab");
  await pause(page, 250);
  const find = await page.evaluate(() => {
    const b = document.getElementById("findbox"), r = b.getBoundingClientRect(), fold = document.getElementById("foldfab").getBoundingClientRect();
    return { open: !b.hidden, top: Math.round(r.top), below: r.top >= fold.bottom - 1, right: Math.round(innerWidth - r.right), folded: !document.body.classList.contains("unfolded") };
  });
  check("AC-18 AC-58 a phone: find from the column opens its field at the top right, by ⟨…⟩, and the column folds", find.open && find.below && find.top < 260 && find.right <= 20 && find.folded,
    JSON.stringify(find));
  await page.keyboard.press("Escape");
  await pause(page, 150);

  // a finger scrolling the terminal: every floating button out of sight until it lifts (Chromium: a CDP touch screen)
  if (name === "chromium") {
    const cdp = await page.context().newCDPSession(page);
    const at = (type, y) => cdp.send("Input.dispatchTouchEvent", { type, touchPoints: type === "touchEnd" ? [] : [{ x: 120, y }] });
    await page.evaluate(() => { document.getElementById("term").scrollTop = 0; });   // up in the history: "To the bottom" shows
    await pause(page, 300);
    await at("touchStart", 600);
    for (let k = 1; k <= 5; k++) await at("touchMove", 600 - k * 30);
    await pause(page, 200);
    const during = await seen(page, ["foldfab", "fabs", "bottom"]);
    await at("touchEnd");
    await pause(page, 600);
    const after = await seen(page, ["foldfab", "fabs", "bottom"]);
    check("AC-18 a phone: while a finger scrolls the terminal every floating button (To the bottom too) is out of sight, back once it lifted and the scroll stopped",
      !during.foldfab && !during.fabs && !during.bottom && after.foldfab && after.fabs && after.bottom, JSON.stringify({ during, after }));
  }
  check("AC-18 no page errors (fold, phone)", !f.errors.length, f.errors.join(" | "));
  await f.close();

  runningIn(`${name} 1400×900`);
  const w = await openPage(browser, { device: WIDE });
  await drawn(w.page);
  await pause(w.page, 300);
  const wide = await seen(w.page, ["foldfab", "findfab", "sentfab"]);
  check("AC-18 a computer: no ⟨…⟩, the column as before", !wide.foldfab && wide.findfab && wide.sentfab, JSON.stringify(wide));
  await w.close();
  runningIn(name);
}
