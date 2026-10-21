// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A session just opened shows its last line however late its layout settles (AC-02): what grows
// after the first frame (widgets, images, a note, the phone's list closing) keeps the view at the
// end, until the reader moves it. Against a fake terminumd (fake.mjs), at a wide and a phone size.

import { check, runningIn } from "./check.mjs";
import { drawn, openPage, pause, SIZES } from "./fake.mjs";

export async function webOpenEnd(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const { page } = await openPage(browser, { device });
    await drawn(page);
    // late growth above the last line, as a widget that finishes after the first frame
    const grow = (px) => page.evaluate((px) => { const d = document.createElement("div"); d.style.height = `${px}px`; document.getElementById("hist").append(d); }, px);
    const gap = () => page.evaluate(() => { const t = document.getElementById("term"); return Math.round(t.scrollHeight - t.scrollTop - t.clientHeight); });
    await grow(2000);
    await pause(page, 100);
    // a change that left the view short of the end (no reader's doing), then another growth
    await page.evaluate(() => { document.getElementById("term").scrollTop -= 600; });
    await grow(40);
    await pause(page, 100);
    const settled = await gap();
    // the reader scrolls: later growth no longer pulls the view down
    await page.evaluate(() => { const t = document.getElementById("term"); t.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -100 })); t.scrollTop = 0; });
    await grow(500);
    await pause(page, 100);
    const kept = await page.evaluate(() => document.getElementById("term").scrollTop);
    check("AC-02 a session just opened keeps to its last line while its layout settles late; once the reader scrolls, it stays where they are",
      settled < 8 && kept < 8, JSON.stringify({ settled, kept }));
    await page.context().close();
  }
}
