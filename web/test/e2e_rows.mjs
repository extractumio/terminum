// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The session list's rows and headers (AC-01, AC-57, AC-92), in Chromium and WebKit at both
// sizes: every row's folder bar at one place, the bookmarks' and reminders' waves beside it
// (thin, gentle); each group's count in words ("3 sessions": its rows, not the quick shells under
// them), the counts in one column whatever buttons follow; a terminal's icon white on a dark gray
// tile; the shown session's row see-through (a project's tint under it); no hourglass; every
// group tinted, a hue apart from its neighbor's.
// Alone: node web/test/e2e_all.mjs webRows.

import { check, runningIn } from "./check.mjs";
import { drawn, group, item, openPage, SIZES } from "./fake.mjs";

const ID = (n) => `tmux:local:terminum:%${n}`, OWN = (n) => `tmux:local:default:%${n}`;
const DIR = "/Users/alex/work/api";
const LAYOUT = [group("local", [
  item(ID(1), "api", { index: "0", agent: "claude", shell: false, sub: ["2.1", "~/work/api"], dir: DIR }),
  item(ID(2), "web", { index: "2", agent: "codex", shell: false, sub: ["node", "~/work/web"], dir: "/Users/alex/work/web" }),
  item(ID(5), "quick", { index: "4", sub: ["quick shell", "bash", "~/work/web"], quickOf: ID(2), dir: "/Users/alex/work/web" }),
  item(ID(6), "slicer", { index: "11", agent: "claude", shell: false, sub: ["2.1", "~/work/slicer"], dir: "/Users/alex/work/slicer", state: "working" }),
  item(ID(7), "notes", { index: "13", sub: ["bash", "~/notes"], dir: "/Users/alex/notes" })]),
  group("main", [item(OWN(1), "mr", { index: "1", sub: ["zsh", "~/work/mr"], dir: "/Users/alex/work/mr" })],
    { session: "work", attached: true, attach: "tmux attach -t work", wid: "own/main" })];
const PROJECT = { id: "p1", name: "API", color: 2, dir: DIR, members: [ID(1)] };
const MARKS = [{ id: "M1", sid: ID(1), key: ID(1), n: 1, text: "x", note: "n", title: "api", host: null, dir: DIR, at: 1792000000 },
  { id: "R1", sid: ID(1), key: ID(1), n: 2, text: "y", note: "n", title: "api", host: null, dir: DIR, at: 1792000000, remind: true }];

export async function webRows(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device, layout: LAYOUT });
    const { page } = f;
    await drawn(page);
    f.say({ t: "projects", items: [PROJECT] });
    f.say({ t: "marks", items: MARKS });
    if (device.hasTouch) await page.click("#navbtn");
    await page.waitForTimeout(400);
    const got = await page.evaluate(() => {
      const left = (e) => Math.round(e.getBoundingClientRect().left);
      const tops = [...document.querySelectorAll("#sessions li > .item:not(.child)")];
      const waves = [...document.querySelectorAll("#sessions .wave")];
      const bar = tops[0].querySelector(".pbar"), text = tops[0].querySelector(".txt");
      const tile = document.querySelector('#sessions .item[data-id$="%7"] .agmark'), cs = getComputedStyle(tile);
      const counts = [...document.querySelectorAll("#sessions h2 .count")].map((c) => [c.textContent, Math.round(c.getBoundingClientRect().right)]);
      const on = document.querySelector('#sessions .item[data-id$="%1"]');
      return { bars: tops.map((t) => left(t.querySelector(".pbar"))), waves: waves.map((w) => [Math.round(w.getBoundingClientRect().width), left(w) >= bar.getBoundingClientRect().right, w.getBoundingClientRect().right <= text.getBoundingClientRect().left]),
        counts, tile: [cs.backgroundColor, getComputedStyle(tile.querySelector("svg")).color, parseFloat(cs.borderTopLeftRadius) > 0],
        hourglass: !!document.querySelector("#sessions .state.working, #curstate .state"), busy: !!document.querySelector('#sessions .item[data-id$="%6"] .pbar.busy'),
        on: [on.classList.contains("on"), getComputedStyle(on).backgroundColor],
        groups: [...document.querySelectorAll("#sessions section.grp")].map((s) => [Number(/(\d+)\)$/.exec(s.style.getPropertyValue("--proj"))?.[1]), getComputedStyle(s).backgroundColor]) };
    });
    check("AC-01 every row's folder bar stands at one place, with or without the waves beside it", new Set(got.bars).size === 1, JSON.stringify(got.bars));
    check("AC-57 the bookmarks' and reminders' waves: 4 px, right of the bar, clear of the text", got.waves.length === 2 && got.waves.every(([w, r, c]) => w === 4 && r && c), JSON.stringify(got.waves));
    check("AC-92 each group's count says its sessions in words (its rows, not the quick shell under one), the counts in one column",
      JSON.stringify(got.counts.map(([t]) => t)) === JSON.stringify(["1 session", "3 sessions", "1 session"]) && new Set(got.counts.map(([, r]) => r)).size === 1, JSON.stringify(got.counts));
    check("AC-01 a terminal's icon is white on a dark gray tile with small round corners", got.tile[0] === "rgb(58, 58, 60)" && got.tile[1] === "rgb(255, 255, 255)" && got.tile[2], JSON.stringify(got.tile));
    check("AC-39 a working agent has no hourglass: its bar pulses", !got.hourglass && got.busy, JSON.stringify(got));
    check("AC-92 the shown session's row in a project is see-through (the project's tint), not a box of its own",
      got.on[0] && /\/ 0\.1|, 0\.1/.test(got.on[1]), JSON.stringify(got.on));
    const apart = got.groups.slice(1).map(([h], i) => Math.abs(((h - got.groups[i][0] + 540) % 360) - 180));
    check("AC-92 every group is tinted (a project in its color), each in a hue at least 60° from the group above it",
      got.groups.length === 3 && got.groups.every(([h, bg]) => Number.isFinite(h) && !/^rgba\(0, 0, 0, 0\)$|transparent/.test(bg)) && apart.every((d) => d >= 60),
      JSON.stringify({ groups: got.groups, apart }));
    await f.close();
  }
}
