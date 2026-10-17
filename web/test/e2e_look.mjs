// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Layout guards (docs/UI-GUIDELINES.md §6) against a fake terminumd (fake.mjs), at a wide and a
// phone size: controls of one kind look alike, a primary button stands out from Cancel, a
// phone's targets are finger-sized, a long toast stays on the screen, waiting cards never cover
// the banners. They check computed styles and boxes, which behavior checks never look at.

import { check, runningIn } from "./check.mjs";
import { drawn, group, item, LAYOUT, openPage, pause, SIZES } from "./fake.mjs";

const HOST = group("devbox", [item("tmux:devbox:default:%1", "build", { host: "devbox" })],
  { host: "devbox", session: "terminum", state: "attached", others: 2, server: "devbox:default", attach: "ssh -t -- devbox 'tmux attach -t =terminum'" });
const ASKING = group("asks", [item("tmux:local:terminum:%7", "claude", { agent: "claude", shell: false, state: "waiting" })]);

/** Every visible button under `sel` smaller than `least` px tall: "id-or-class:height". */
const small = (page, sel, least) => page.evaluate(([sel, least]) => [...document.querySelectorAll(sel)]
  .filter((b) => b.offsetParent && b.getClientRects().length)
  .map((b) => [b.id || b.className || b.textContent.trim().slice(0, 20), Math.round(b.getBoundingClientRect().height)])
  .filter(([, h]) => h < least).map(([n, h]) => `${n}:${h}`), [sel, least]);

export async function webLook(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const phone = size.startsWith("390");
    const f = await openPage(browser, { device, layout: [...LAYOUT, HOST] });
    const { page } = f;
    await drawn(page);

    const dialogs = await page.evaluate(() => [...document.querySelectorAll("dialog")].filter((d) => d.querySelector(".actions .primary")).map((d) => {
      const fill = (b) => getComputedStyle(b).backgroundColor;
      const primary = d.querySelector(".actions .primary"), other = d.querySelector(".actions button:not(.primary)");
      return [d.id, fill(primary), other && fill(other)];
    }));
    check("look: every dialog's primary button is filled apart from its Cancel", dialogs.length >= 3 && dialogs.every(([, p, c]) => c && p !== c),
      JSON.stringify(dialogs));

    if (phone) { await page.click("#navbtn"); await pause(page, 300); }
    const head = await page.evaluate(() => [...document.querySelectorAll('#sessions .grp[data-wid="devbox/terminum"] h2 > button:not(.toggle)')].map((b) => {
      const s = getComputedStyle(b);
      return [b.className, `${s.width} ${s.height} ${s.borderRadius} ${s.color}`];
    }));
    check("look: a group header's buttons (copy, import, +, ⋯) share one size, radius and color",
      head.length === 4 && new Set(head.map(([, l]) => l)).size === 1, JSON.stringify(head));
    if (phone) {
      const list = await small(page, "#nav button:not(.item):not(.grip)", 40);
      await page.click("#addhostbtn");
      const form = await small(page, "#addhost button", 40);
      await page.click("#addhostcancel");
      await page.click("#drawerclose");
      await pause(page, 300);
      const bar = await small(page, "#bar button:not(.seg button)", 40);
      await page.click("#menubtn");
      const menu = await small(page, "#menu .row, #menu button:not(.seg button)", 40);
      const segs = await small(page, ".seg button", 34);
      await page.keyboard.press("Escape");
      check("look: on a phone every button of the list, a form, the header and the menu is at least 40px tall (a segment 34)",
        !list.length && !form.length && !bar.length && !menu.length && !segs.length, JSON.stringify({ list, form, bar, menu, segs }));
    }

    const toast = await page.evaluate(() => {
      const t = document.getElementById("toast");
      t.textContent = "A long message from terminumd: ".repeat(5);
      t.hidden = false;
      const r = t.getBoundingClientRect();
      t.hidden = true;
      return { left: Math.round(r.left), right: Math.round(r.right), width: innerWidth };
    });
    check("look: a long toast wraps inside the screen", toast.left >= 0 && toast.right <= toast.width, JSON.stringify(toast));

    if (!phone) {
      f.say({ t: "layout", groups: [...LAYOUT, HOST, ASKING] });
      f.say({ t: "restore", report: { text: "Restored 2 sessions, 4 windows. ".repeat(3), sessions: 2, windows: 4, programs: [], errors: [], notes: [] } });
      await page.waitForSelector("#asks .ask:not([hidden])", { timeout: 6000 }).catch(() => {});
      const boxes = await page.evaluate(() => {
        const r = (id) => document.getElementById(id)?.getBoundingClientRect();
        const ask = document.querySelector("#asks .ask:not([hidden])")?.getBoundingClientRect(), banner = r("restored"), fab = r("findfab");
        const meets = (a, b) => !!(a && b && b.width && a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom);
        return { ask: !!ask, banner: meets(ask, banner), fab: meets(ask, fab) };
      });
      check("look: a waiting card covers neither the banners nor the find button", boxes.ask && !boxes.banner && !boxes.fab, JSON.stringify(boxes));
    }
    await f.close();
  }
}
