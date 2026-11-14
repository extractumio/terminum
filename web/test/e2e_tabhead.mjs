// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum.md AC-18 "on a tablet the header is laid out as a computer's with a finger's keys" against a fake terminumd
// (fake.mjs), at an iPad's landscape (its portrait, 820 px, has the phone's header), the list shown and hidden: every key in the header one height, their
// middles on one line, none touching the header's edge; ✎ and Copy beside the title, in that order, right of its text;
// the folder line whole where it has room.
import { check, runningIn } from "./check.mjs";
import { drawn, group, item, openPage, pause, TABLET } from "./fake.mjs";

const SID = "tmux:local:terminum:%1";
const LAYOUT = [group("main", [item(SID, "api", { sub: ["claude", "~/work/api", "Fixing the tests"], index: "3", dir: "/Users/alex/work/api" })])];

const measure = (page) => page.evaluate(() => {
  const bar = document.getElementById("bar").getBoundingClientRect();
  const seen = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && getComputedStyle(e).visibility !== "hidden" ? r : null; };
  const keys = [...document.querySelectorAll("#bar :is(#navbtn, .sections, #rename, #curplan, .copydir, #quickbtn, #views, #menubtn)")]
    .map((e) => [e.id || e.className, seen(e)]).filter(([, r]) => r).map(([k, r]) => ({ k, h: Math.round(r.height), mid: Math.round(r.top + r.height / 2), top: Math.round(r.top - bar.top), left: r.left, right: r.right }));
  const title = document.querySelector("#current .t").getBoundingClientRect(), sub = document.getElementById("cursub");
  const cut = [...sub.children].some((e) => e.scrollWidth > e.clientWidth + 1);
  return { keys, bar: Math.round(bar.height), titleRight: title.right, titleMid: Math.round(title.top + title.height / 2), cut };
});

export async function webTabHead(browser, name) {
  for (const [size, device] of [["1180×820 touch", TABLET]]) {
    for (const hidden of [false, true]) {
      const at = `(${name} ${size}${hidden ? ", list hidden" : ""})`;
      runningIn(`${name} ${size}`);
      const f = await openPage(browser, { device, layout: LAYOUT });
      const { page } = f;
      await drawn(page);
      if (hidden) await page.click("#navclose");
      await pause(page, 200);
      const m = await measure(page);
      const hs = new Set(m.keys.map((k) => k.h)), mids = m.keys.map((k) => k.mid);
      check(`AC-18 ${at} every header key one height (44 px)`, hs.size === 1 && hs.has(44) && m.keys.length >= (hidden ? 7 : 5), JSON.stringify(m.keys));
      check(`AC-18 ${at} their middles on one line, the title's too`, Math.max(...mids) - Math.min(...mids) <= 1 && Math.abs(m.titleMid - mids[0]) <= 1, JSON.stringify({ mids, title: m.titleMid }));
      check(`AC-18 ${at} none touching the header's edge`, m.keys.every((k) => k.top >= 4 && k.top + k.h <= m.bar - 4), JSON.stringify({ bar: m.bar, keys: m.keys }));
      const pen = m.keys.find((k) => k.k === "rename"), copy = m.keys.find((k) => /copydir/.test(k.k));
      check(`AC-18 ${at} ✎ and Copy together right of the title`, pen && copy && pen.left >= m.titleRight && copy.left - pen.right <= 12 && copy.left > pen.right, JSON.stringify({ pen, copy, title: m.titleRight }));
      check(`AC-18 ${at} the folder line whole where it has room`, !m.cut, JSON.stringify(m));
      check(`no page error ${at}`, f.errors.length === 0, f.errors.join("; "));
      await f.context.close();
    }
  }
}
