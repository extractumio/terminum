// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// An IDE's keys (AC-07) against a fake terminumd (fake.mjs): ⌘[ and ⌘] go back and forward
// through the sessions shown, kept across a reload; ⌘F opens the history's search; ⌘V after a
// copy pastes into the terminal's input again; ⌘B opens
// and closes Files, docked on a computer, in the terminal's place on a phone.

import { check, runningIn } from "./check.mjs";
import { drawn, LAYOUT, openPage, pause, PHONE, WIDE } from "./fake.mjs";

const ID = (n) => `tmux:local:terminum:%${n}`;

export async function webIdeKeys(browser, name) {
  runningIn(`${name} 1400×900`);
  const f = await openPage(browser, { device: WIDE, layout: LAYOUT });
  const { page, sent } = f;
  await drawn(page);
  const shown = () => sent.filter((m) => m.t === "sub").pop()?.id;
  for (const n of [2, 3]) {
    await page.click(`#sessions .item[data-id="${ID(n)}"]`);
    await pause(page);
  }
  const walk = [];
  for (const k of ["BracketLeft", "BracketLeft", "BracketLeft", "BracketRight"]) {
    await page.keyboard.press(`Meta+${k}`);
    await pause(page);
    walk.push(shown().split(":").pop());
  }
  check("AC-07 ⌘[ goes back through the sessions shown, ⌘] forward again; past the first nothing moves",
    walk.join() === "%2,%1,%1,%2", walk.join());
  await page.click(`#sessions .item[data-id="${ID(3)}"]`);
  await page.reload();
  await drawn(page);
  await page.keyboard.press("Meta+BracketLeft");
  await pause(page);
  check("AC-07 the sessions shown are kept across a reload (this browser)", shown() === ID(2), shown());

  await page.click("#term");
  await page.keyboard.press("Meta+KeyF");
  await pause(page);
  check("AC-07 ⌘F opens the search in the history, its field focused",
    await page.evaluate(() => !document.getElementById("findbox").hidden && document.activeElement?.id === "findq"));
  await page.keyboard.press("Escape");

  // ⌘V after a copy from the history: the terminal's input has the keys again, the paste goes there
  await page.evaluate(() => {
    const r = document.createRange();
    r.selectNodeContents(document.querySelector("#screen .ln"));
    getSelection().removeAllRanges();
    getSelection().addRange(r);
    document.activeElement?.blur();
  });
  const off = await page.evaluate(() => document.activeElement?.id || document.activeElement?.tagName);
  await page.keyboard.press("Meta+KeyV");
  await pause(page);
  check("AC-03 ⌘V with the keys off the input (text selected in the history) gives them back to it, the paste with them",
    off !== "kbd" && (await page.evaluate(() => document.activeElement?.id)) === "kbd", off);
  const docked = () => page.evaluate(() => document.body.classList.contains("docked"));
  const before = await docked();
  await page.keyboard.press("Meta+KeyB");
  await pause(page);
  const after = await docked();
  await page.keyboard.press("Meta+KeyB");
  await pause(page);
  check("AC-07 ⌘B closes and opens docked Files", before !== after && (await docked()) === before, JSON.stringify({ before, after }));
  check("AC-07 no page errors", !f.errors.length, f.errors.join(" | "));
  await f.close();

  runningIn(`${name} 390×844`);
  const p = await openPage(browser, { device: PHONE, layout: LAYOUT });
  await drawn(p.page);
  const where = () => p.page.evaluate(() => document.body.dataset.shown);
  await p.page.keyboard.press("Meta+KeyB");
  await pause(p.page);
  const open = await where();
  await p.page.keyboard.press("Meta+KeyB");
  await pause(p.page);
  check("AC-07 ⌘B on a phone shows Files in the terminal's place and the terminal again", open === "files" && (await where()) === "term", open);
  await p.close();
}
