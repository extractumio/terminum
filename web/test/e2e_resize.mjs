// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Checks of a view that changes width (AC-04) against a fake terminumd (fake.mjs), at a wide and
// a phone size (docked Files opened, a narrower window, a bigger text size): the normal screen
// re-flows to the new width at once at the chosen text size and is drawn as laid out once tmux's
// new width comes; a full-screen program keeps the chosen text size, clipped, until that width
// comes, and is drawn scaled when it does not come within 2 s.

import { check, runningIn } from "./check.mjs";
import { drawn, line, openPage, pause, SIZES } from "./fake.mjs";

const SID = "tmux:local:terminum:%1";

export async function webResize(browser, name) {
  for (const [size, device] of SIZES) {
    for (const alt of [false, true]) {
      runningIn(`${name} ${size}${alt ? " full-screen" : ""}`);
      await resize(browser, { phone: size.startsWith("390"), device, alt });
    }
  }
  runningIn(name);
}

async function resize(browser, { phone, device, alt }) {
  const f = await openPage(browser, { device: phone ? { ...device, viewport: { width: 700, height: 844 } } : device });
  const { page, sent } = f;
  await drawn(page);
  const sizes = () => sent.filter((m) => m.t === "size");
  const settled = async () => { for (let n = -1; n !== sizes().length;) { n = sizes().length; await pause(page, 600); } return sizes().at(-1); };
  // tmux's pane at `cols`, its rows full
  const frame = (cols) => f.say({ t: "screen", sid: SID, full: true, n: 6, cols, rows: 6, alt,
    ch: Array.from({ length: 6 }, (_, i) => [i, line(`${i}`.padEnd(cols, "x"))]) });
  const look = () => page.evaluate(() => {
    const t = document.getElementById("term"), cs = getComputedStyle(t);
    return { px: parseFloat(cs.fontSize), size: parseFloat(cs.getPropertyValue("--size")), holding: t.classList.contains("holding"),
      reflow: t.classList.contains("reflow"), sideways: cs.overflowX !== "hidden" && t.scrollWidth > t.clientWidth + 1 };
  });
  const docked = () => page.evaluate(() => document.body.classList.contains("docked"));
  const at = alt ? "AC-04 a full-screen program:" : "AC-04 the normal screen:";

  // the pane at this view's width, Files closed (wide) or a wide phone
  if (!phone && await docked()) await page.keyboard.press("Meta+KeyB");
  const s0 = await settled();
  frame(s0.cols);
  await pause(page);
  const before = await look();

  // the view narrows: docked Files opened, a narrower phone
  if (phone) await page.setViewportSize({ width: 500, height: 844 });
  else await page.keyboard.press("Meta+KeyB");
  await pause(page, 50);
  const narrowed = await look();
  check(alt ? `${at} the view narrows: the text stays at the chosen size, clipped, while the new width is on its way`
    : `${at} the view narrows: its lines re-flow to the new width at once, at the chosen size`,
  before.px === before.size && !before.holding && !before.reflow && narrowed.px === narrowed.size && !narrowed.sideways
      && (alt ? narrowed.holding && !narrowed.reflow : narrowed.reflow && !narrowed.holding),
  JSON.stringify({ before, narrowed, docked: await docked() }));

  const s1 = await settled();
  frame(s1.cols);
  await pause(page);
  const resized = await look();
  check(`${at} tmux's new width comes: the pane is drawn as laid out at the same text size`,
    s1.cols < s0.cols && resized.px === resized.size && resized.px === before.px && !resized.holding && !resized.reflow, JSON.stringify({ s0, s1, resized }));

  // a bigger text size whose width never comes (the fake does not answer)
  await page.click("#menubtn");
  await page.evaluate(() => { document.getElementById("bigger").click(); document.getElementById("bigger").click(); });
  await page.keyboard.press("Escape");
  await pause(page, 50);
  const bigger = await look();
  await pause(page, 2400);
  const gaveUp = await look();
  if (alt) {
    check(`${at} a bigger text size shows at once, clipped; with no new width within 2 s the pane is drawn scaled`,
      bigger.px === bigger.size && bigger.size === before.size + 2 && bigger.holding
        && gaveUp.px < gaveUp.size && !gaveUp.holding && !gaveUp.sideways, JSON.stringify({ bigger, gaveUp }));
  } else {
    check(`${at} a bigger text size shows at once, re-flowed; with no new width it stays so, never scaled`,
      bigger.px === bigger.size && bigger.size === before.size + 2 && bigger.reflow && !bigger.holding
        && gaveUp.px === gaveUp.size && gaveUp.reflow && !gaveUp.sideways, JSON.stringify({ bigger, gaveUp }));
  }
  check(`${at} no page errors (resize)`, !f.errors.length, f.errors.join(" | "));
  await f.close();
}
