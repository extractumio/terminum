// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-35 against a real private terminumd and tmux (e2e_live.mjs): a link a program printed with a
// text of its own (OSC 8, as Claude Code prints "⧉ Attack and Compromise Detection — MR !318")
// reaches the page with its address, is underlined and opens that address with ⌘ (Ctrl elsewhere);
// a javascript: one stays plain text.
import fs from "node:fs";
import path from "node:path";
import { check } from "./check.mjs";

const MR = "https://git.example/a/-/merge_requests/318";
const LONG = Array.from({ length: 40 }, (_, i) => `part${i}`).join(" ");   // wider than the pane: tmux wraps it

export async function liveLinks(page, { work }) {
  const file = path.join(work, "links.txt");
  fs.writeFileSync(file, `see \x1b]8;id=a1;${MR}\x1b\\Attack and Compromise Detection — MR !318\x1b]8;;\x1b\\ and \x1b]8;;javascript:alert(1)\x1b\\evil\x1b]8;;\x1b\\ end\n\x1b]8;;${MR}\x1b\\${LONG}\x1b]8;;\x1b\\ after\n`);
  await page.evaluate(() => { window.opened = []; window.open = (u) => { window.opened.push(u); return null; }; });
  await page.click("#term");
  await page.keyboard.type(`clear; cat ${file}`);
  await page.keyboard.press("Enter");
  const link = await page.waitForFunction(() => {
    const e = [...document.querySelectorAll("#term .ln .olink")].find((s) => s.textContent.includes("MR !318"));
    if (!e) return null;
    const b = e.getBoundingClientRect();
    return { text: e.textContent, href: e.dataset.href, title: e.title, x: b.left + b.width / 2, y: b.top + b.height / 2 };
  }, null, { timeout: 5000 }).then((h) => h.jsonValue(), () => null);
  const halves = await page.evaluate((mr) => [...document.querySelectorAll("#term .ln .olink")].filter((s) => s.dataset.href === mr).map((s) => s.textContent), MR);
  const evil = await page.evaluate(() => [...document.querySelectorAll("#term .ln span")].filter((s) => s.textContent === "evil").map((s) => s.classList.contains("olink") || !!s.title));
  const mod = process.platform === "darwin" ? "Meta" : "Control";
  if (link) { await page.keyboard.down(mod); await page.mouse.click(link.x, link.y); await page.keyboard.up(mod); }
  const opened = await page.evaluate(() => window.opened);
  check("AC-35 a program's OSC 8 link through tmux: its text underlined with its address on hover, ⌘-click opens that address; a javascript: one is plain text",
    link?.text === "Attack and Compromise Detection — MR !318" && link.href === MR && link.title === MR && JSON.stringify(opened) === JSON.stringify([MR])
      && JSON.stringify(evil) === "[false]", JSON.stringify({ link, opened, evil }));
  // tmux 3.6a ends a link at a row's end and does not open it again on the next (AS-03): its first row only
  check("AC-35 an OSC 8 link wider than the pane is a link on its first row; the text after it is not",
    halves.length === 2 && LONG.startsWith(halves[1]) && halves[1].length > 20 && !halves.some((t) => t.includes("after")), JSON.stringify(halves));
  fs.rmSync(file, { force: true });
}
