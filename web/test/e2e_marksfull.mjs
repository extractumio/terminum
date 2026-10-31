// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-98 and AC-91 against a fake terminumd (fake.mjs): the banner when agents' records went to make
// room (its words, Open showing the pane that made room with its pane of that kind, × telling
// terminumd, a pane gone opening the list below the sessions), and the bookmarks' floating button in
// the reminders' look (an orange bookmark on the plain fill, never dark on an orange blob).
import { check, runningIn } from "./check.mjs";
import { drawn, openPage, pause, SIZES } from "./fake.mjs";
import { fullText } from "../static/marksfull.js";

const [P1, P2] = ["tmux:local:terminum:%1", "tmux:local:terminum:%2"];
const at = 1790000000;
const mark = (id, sid, extra = {}) => ({ id, sid, key: sid, n: 3, text: "a.txt", note: `note ${id}`, title: "api", host: null, at, by: "claude", ...extra });
const ITEMS = [mark("B1", P1), mark("R1", P1, { remind: true }), mark("B2", P2), mark("R2", P2, { remind: true })];
const BANNERS = [{ scope: P2, remind: false, most: 50, label: "session docs", sid: P2 },
  { scope: P2, remind: true, most: 20, label: "session docs", sid: P2 }];

export async function webMarksFull(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device });
    const { page } = f;
    await drawn(page);
    f.say({ t: "marks", items: ITEMS, error: null });
    await pause(page, 300);

    const look = await page.evaluate(() => {
      const cs = (id) => getComputedStyle(document.getElementById(id));
      const mark = getComputedStyle(document.documentElement).getPropertyValue("--mark").trim();
      return { fill: cs("marksfab").backgroundColor, remindFill: cs("remindfab").backgroundColor, ink: cs("marksfab").color, mark,
        shown: !document.getElementById("marksfab").hidden };
    });
    check(`AC-91 (${size}) the bookmarks' button: an orange bookmark on the plain buttons' fill, as the bell`,
      look.shown && look.fill === look.remindFill && look.ink === "rgb(255, 159, 26)" && look.mark === "#ff9f1a", JSON.stringify(look));

    f.say({ t: "marksfull", items: BANNERS });
    await pause(page, 200);
    const shown = await page.evaluate(() => {
      const box = document.getElementById("marksfull"), r = box.getBoundingClientRect(), term = document.getElementById("term").getBoundingClientRect();
      return { hidden: box.hidden, lines: [...box.querySelectorAll(".fullline > span")].map((s) => s.textContent),
        links: [...box.querySelectorAll(".link")].map((b) => b.textContent), above: r.bottom <= term.top + 1, wide: r.width > 300 };
    });
    check(`AC-98 (${size}) agents' records that went for room: a line a kind over the terminal, terminumd's numbers, with Open and ×`,
      !shown.hidden && shown.above && JSON.stringify(shown.lines) === JSON.stringify(BANNERS.map(fullText))
      && shown.lines[0] === "Agents' bookmarks of session docs are over 50: the oldest are deleted automatically. Review them."
      && JSON.stringify(shown.links) === JSON.stringify(["Open bookmarks", "Open reminders"]), JSON.stringify(shown));

    const from = f.sent.length;
    await page.click("#marksfull .fullline:nth-child(1) .link");
    await page.waitForFunction(() => !document.getElementById("markpane").hidden, null, { timeout: 3000 }).catch(() => {});
    const opened = await page.evaluate(() => ({ pane: !document.getElementById("markpane").hidden,
      titles: [...document.querySelectorAll("#markpane .mtitle, #markpane .ttl, #markpane [data-id]")].length }));
    const sub = f.sent.slice(from).find((m) => m.t === "sub");
    check(`AC-98 (${size}) Open bookmarks shows the pane that made room and opens its bookmarks' pane`,
      sub?.id === P2 && opened.pane, JSON.stringify({ sub, opened }));
    await page.keyboard.press("Escape");
    await pause(page, 200);

    const before = f.sent.length;
    await page.click("#marksfull .fullline:nth-child(2) .x");
    await pause(page, 200);
    const seen = f.sent.slice(before).find((m) => m.t === "marksfullseen");
    check(`AC-98 (${size}) × tells terminumd which line (scope and kind), which closes it on every page`,
      seen?.scope === P2 && seen?.remind === true, JSON.stringify(seen));
    f.say({ t: "marksfull", items: [] });
    await pause(page, 200);
    check(`AC-98 (${size}) none left: the banner goes`, await page.evaluate(() => document.getElementById("marksfull").hidden));

    f.say({ t: "marksfull", items: [{ ...BANNERS[0], sid: "tmux:local:terminum:%99" }] });
    await pause(page, 200);
    await page.click("#marksfull .link");
    await pause(page, 300);
    check(`AC-98 (${size}) the pane that made room gone: Open shows Bookmarks / Reminders below the sessions`,
      await page.evaluate(() => document.getElementById("marksdlg").open));
    check(`AC-98 (${size}) no page errors (the banner)`, !f.errors.length, f.errors.join(" | "));
    await f.close();
  }
  runningIn(name);
}
