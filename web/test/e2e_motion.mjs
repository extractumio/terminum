// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Motion (docs/UI-GUIDELINES.md §2.8) against a fake terminumd (fake.mjs), with motion on: a
// popover comes in and is in place within 300 ms, and goes at once; a press ripples; the
// selected row settles once; nothing moves in the terminal; with reduced motion nothing moves.
// The other checks run with reduced motion, so they measure boxes at their place.

import { check, runningIn } from "./check.mjs";
import { drawn, openPage, pause, WIDE } from "./fake.mjs";

/** The running animations whose target is under `sel`, as "tag#id.class:name". */
const moving = (page, sel) => page.evaluate((sel) => document.getAnimations()
  .filter((a) => a.playState === "running" && a.effect?.target?.closest?.(sel) && a.effect.getTiming().iterations !== Infinity)
  .map((a) => { const t = a.effect.target; return `${t.tagName}#${t.id}.${t.className}:${a.animationName ?? a.transitionProperty}`; }), sel);

export async function webMotion(browser, name) {
  runningIn(`${name} 1400×900`);
  const f = await openPage(browser, { device: { ...WIDE, reducedMotion: "no-preference" } });
  const { page } = f;
  await drawn(page);

  await page.click("#menubtn");
  const opening = await page.evaluate(() => getComputedStyle(document.getElementById("menu")).opacity);
  // its transition is the token's 200 ms; a slow browser's frames may take longer to show the end
  const lasts = await page.evaluate(() => getComputedStyle(document.getElementById("menu")).transitionDuration);
  const atRest = () => { const m = getComputedStyle(document.getElementById("menu")); const y = Math.abs(parseFloat(m.translate.split(" ")[1] ?? "0") || 0);
    return m.opacity === "1" && ["1", "none"].includes(m.scale) && y < 0.5; };
  await page.waitForFunction(atRest, null, { timeout: 2000 }).catch(() => {});
  const settled = await page.evaluate(atRest);
  const opened = await page.evaluate(() => { const s = getComputedStyle(document.getElementById("menu")); return [s.opacity, s.scale, s.translate]; });
  await page.keyboard.press("Escape");
  const closed = await page.evaluate(() => getComputedStyle(document.getElementById("menu")).display);
  check("motion: a popover comes in (it starts faded, over 200 ms) and comes to its place; it goes at once",
    Number(opening) < 1 && lasts.split(", ").every((d) => d === "0.2s") && settled && closed === "none",
    JSON.stringify({ opening, lasts, opened, closed }));

  const box = await page.$("#newsessbtn").then((b) => b.boundingBox());
  await page.mouse.move(box.x + 20, box.y + 10);
  await page.mouse.down();
  await page.waitForFunction(() => Number(getComputedStyle(document.getElementById("newsessbtn")).scale) < 1, null, { timeout: 1000 }).catch(() => {});
  const press = await page.evaluate(() => { const b = document.getElementById("newsessbtn"); return [b.dataset.press, b.style.getPropertyValue("--px"), getComputedStyle(b).scale]; });
  await page.mouse.up();
  const ripples = await page.evaluate(() => document.getAnimations().filter((a) => a.effect?.pseudoElement === "::before" && a.effect.target?.id === "newsessbtn").length);
  check("motion: a press shrinks the button and ripples from where it was pressed", ["a", "b"].includes(press[0]) && press[1] === "20px" && Number(press[2]) < 1 && ripples === 1,
    JSON.stringify({ press, ripples }));
  await page.click("#newsesscancel");

  await page.click("#sessions .item[data-id='tmux:local:terminum:%2']");
  const settle = await moving(page, "#sessions .item.on");
  check("motion: the newly selected row settles once (its fill and its bar)", settle.length >= 1, JSON.stringify(settle));
  await pause(page, 400);

  await page.click("#term");
  await page.keyboard.type("ls -la");
  await page.mouse.click(300, 300);
  const inTerm = await moving(page, "#term, #kbd, .key");
  check("motion: nothing moves in the terminal, while typing or on a click there", !inTerm.length, JSON.stringify(inTerm));
  await f.close();

  const calm = await openPage(browser, { device: WIDE });
  await drawn(calm.page);
  await calm.page.click("#menubtn");
  await calm.page.keyboard.press("Escape");
  await calm.page.click("#newsessbtn");
  const still = await moving(calm.page, "body");
  check("motion: with reduced motion nothing moves: no fade, no ripple, no settle", !still.length, JSON.stringify(still));
  check("motion: no page errors", !f.errors.length && !calm.errors.length, [...f.errors, ...calm.errors].join(" | "));
  await calm.close();
}
