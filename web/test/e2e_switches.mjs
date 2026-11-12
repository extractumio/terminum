// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// One switch and one tick across the pages (docs/UI-GUIDELINES.md "A switch and a tick") against a fake terminumd
// (fake.mjs): what turns a feature on or off (Settings' rows, Auto-Dispatch's On, a schedule's Paused, the board's
// Auto-Dispatch key) is the same drawn track and knob, its knob at the right in the accent when on; what is
// included (Auto-Dispatch's severities, a filter's rows) stays a drawn tick.

import { check, runningIn } from "./check.mjs";
import { drawn, openPage, SIZES } from "./fake.mjs";

/** Each switch's and tick's drawn look, by selector. */
const looks = (page, sels) => page.evaluate((sels) => Object.fromEntries(sels.map((s) => {
  const e = document.querySelector(s), cs = getComputedStyle(e), knob = getComputedStyle(e, "::after");
  return [s, { look: [cs.width, cs.height, cs.borderRadius, cs.appearance || cs.webkitAppearance].join(" "), fill: cs.backgroundColor, knob: knob.transform }];
})), sels);

export async function webSwitches(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device });
    await drawn(f.page);
    const menu = ["#followagent", "#sidemarks", "#tipson", "#appcur", "#bracket", "#insclip"];
    const page = await looks(f.page, menu);
    const b = await openPage(browser, { device, url: "/kanban.html" });
    await b.page.evaluate(() => { document.getElementById("autoswitch").setAttribute("aria-checked", "true"); document.getElementById("autoon").checked = true; });
    const board = await looks(b.page, ["#autoswitch .track", "#autoon", "#schedpaused", "#autoseverities input"]);
    const all = [...Object.values(page), board["#autoswitch .track"], board["#autoon"], board["#schedpaused"]];
    check("every switch is drawn alike: a 26 × 14 px track, no platform look", all.every((l) => l.look === "26px 14px 7px none"), JSON.stringify({ page, board }));
    // Chromium draws an input's knob but does not say where to a script: the key's knob is read, the inputs' tracks compared
    check("on, a switch's track takes the accent and its knob goes right; off, neither", board["#autoon"].fill === board["#autoswitch .track"].fill && board["#autoswitch .track"].knob !== "none"
      && board["#schedpaused"].fill !== board["#autoon"].fill, JSON.stringify(board));
    check("what is included stays a tick: 16 px, square, drawn", board["#autoseverities input"].look === "16px 16px 2px none", JSON.stringify(board["#autoseverities input"]));
    check("no page errors", !f.errors.length && !b.errors.length, [...f.errors, ...b.errors].join(" | "));
    await f.close();
    await b.close();
  }
}
