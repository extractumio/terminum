// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The header's second line (AC-01, header.js) against a fake terminumd (fake.mjs), at both sizes:
// the folder with its Copy, tmux's session:window, then what runs there; Copy copies the full path.
import { check, runningIn } from "./check.mjs";
import { drawn, group, item, openPage, pause, SIZES } from "./fake.mjs";

const SID = "tmux:local:terminum:%1";
const LAYOUT = [group("main", [item(SID, "api", { sub: ["claude", "~/work/api", "Fixing the tests"], index: "3", dir: "/Users/alex/work/api", profile: "local" })])];

export async function webHeader(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device, layout: LAYOUT });
    const { page } = f;
    await drawn(page);
    await pause(page, 200);
    const line = await page.evaluate(() => {
      const parts = [...document.getElementById("cursub").children].filter((e) => !e.matches(".sep"));
      const boxes = parts.map((e) => e.getBoundingClientRect()), overlap = boxes.some((b, i) => i && b.left < boxes[i - 1].right - 0.5);
      return { parts: parts.map((e) => e.className + ":" + e.textContent), overlap };
    });
    check("AC-01 the header's second line: the folder and its Copy, then session:window, then what runs there; no part over another",
      JSON.stringify(line.parts) === JSON.stringify(["dir:~/work/api", "copydir:", "where:main:3", "set:claude·local·Fixing the tests"]) && !line.overlap, JSON.stringify(line));
    await page.evaluate(() => {                     // what was copied: by the clipboard API or the older copy command
      window.copied = [];
      document.addEventListener("copy", () => window.copied.push(String(getSelection())), true);
      if (navigator.clipboard) navigator.clipboard.writeText = async (t) => { window.copied.push(t); };
    });
    const drawer = await page.evaluate(() => document.getElementById("app").className);
    await page.click("#cursub .copydir");
    await pause(page, 200);
    const got = await page.evaluate(() => ({ copied: window.copied.at(-1) ?? null, app: document.getElementById("app").className }));
    check("AC-01 the header's Copy copies the folder's full path and opens nothing else",
      got.copied === "/Users/alex/work/api" && got.app === drawer, JSON.stringify({ ...got, drawer }));
    check("AC-01 no page errors (header)", !f.errors.length, f.errors.join(" | "));
    await f.close();
  }
  runningIn(name);
}
