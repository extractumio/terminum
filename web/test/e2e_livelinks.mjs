// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-35 against a real private terminumd and tmux (e2e_live.mjs): a link a program printed with a
// text of its own (OSC 8, as Claude Code prints "⧉ Attack and Compromise Detection — MR !318")
// reaches the page with its address, is underlined and opens that address with ⌘ (Ctrl elsewhere);
// a javascript: one stays plain text. A folder's printed path opens it in Files, closed or open,
// only where Files can show it.
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
  // where the link rests: the view may still move after the output (the prompt drawn, the view kept at its bottom)
  const at = link && await textPoint(page, link.text);
  if (at) { await page.keyboard.down(mod); await page.mouse.click(at.x, at.y); await page.keyboard.up(mod); }
  const opened = await page.evaluate(() => window.opened);
  check("AC-35 a program's OSC 8 link through tmux: its text underlined with its address on hover, ⌘-click opens that address; a javascript: one is plain text",
    link?.text === "Attack and Compromise Detection — MR !318" && link.href === MR && link.title === MR && JSON.stringify(opened) === JSON.stringify([MR])
      && JSON.stringify(evil) === "[false]", JSON.stringify({ link, at, opened, evil }));
  // tmux 3.6a ends a link at a row's end and does not open it again on the next (AS-03): its first row only
  check("AC-35 an OSC 8 link wider than the pane is a link on its first row; the text after it is not",
    halves.length === 2 && LONG.startsWith(halves[1]) && halves[1].length > 20 && !halves.some((t) => t.includes("after")), JSON.stringify(halves));
  fs.rmSync(file, { force: true });
  await liveFolderLink(page, { work, mod });
}

/** A visible text's center once its position settles: output, the prompt and a resized pane
 *  can still scroll it after it first appears. A ready cursor at an old point is not a ready click. */
async function textPoint(page, text) {
  await page.evaluate(() => { window.linkWas = null; });
  return page.waitForFunction((want) => {
    const view = document.getElementById("term").getBoundingClientRect();
    for (const row of document.querySelectorAll("#term .ln")) {
      const walk = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
      for (let t = walk.nextNode(); t; t = walk.nextNode()) {
        const i = t.data.indexOf(want);
        if (i < 0) continue;
        const r = document.createRange();
        r.setStart(t, i); r.setEnd(t, i + want.length);
        const b = r.getBoundingClientRect();
        if (!b.width || b.top < view.top || b.bottom > view.bottom) continue;   // clear left an earlier command in history
        const now = `${b.left},${b.top},${b.width},${b.height}`;
        if (now !== window.linkWas) { window.linkWas = now; window.linkSince = performance.now(); return null; }
        return performance.now() - window.linkSince > 300 ? { x: b.left + b.width / 2, y: b.top + b.height / 2 } : null;
      }
    }
    window.linkWas = null;
    return null;
  }, text, { timeout: 5000, polling: 50 }).then((h) => h.jsonValue(), () => null);
}

/** ⌘-click (Ctrl elsewhere) in the middle of `text`; with ready, wait for its link cursor first. */
async function clickText(page, text, mod, ready = false) {
  const at = await textPoint(page, text);
  if (!at) return false;
  await page.keyboard.down(mod);
  await page.mouse.move(at.x + 1, at.y); await page.mouse.move(at.x, at.y);
  const linked = !ready || await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => resolve(document.getElementById("term").classList.contains("onlink")))));
  if (linked) await page.mouse.click(at.x, at.y);
  await page.keyboard.up(mod);
  return linked;
}

async function liveFolderLink(page, { work, mod }) {
  const outside = fs.mkdtempSync(path.join(path.dirname(work), "tmlo-"));
  fs.mkdirSync(path.join(work, "shots/ipad"), { recursive: true });
  fs.writeFileSync(path.join(work, "shots/ipad/coach.png"), "");
  try {
    await page.click("#term");
    await page.keyboard.type(`clear; echo in shots/ipad/ not ${outside}/`);
    await page.keyboard.press("Enter");
    const hidden = () => page.evaluate(() => document.getElementById("files").hidden);
    const wasOpen = !(await hidden());
    // The Files button works on every runner; the page's IDE shortcut is Meta+B even on Linux.
    const toggleFiles = () => page.click('button[data-view="files"]');
    if (wasOpen) await toggleFiles();              // Files closed: the frame keeps its pane, hidden
    // A path is a link once its folder was listed. Wait for its cursor, then click once:
    // repeated clicks can leave several asynchronous opens pending (paneInfo()).
    const tree = await (await page.$("#files")).contentFrame();
    const shownIn = () => tree.waitForFunction(() => {
      const names = [...document.querySelectorAll("#tree .row")].map((e) => e.querySelector(".name")?.textContent ?? e.textContent);
      const sel = document.querySelector("#tree .row.sel");
      return sel && /ipad/.test(sel.textContent) && names.some((n) => /coach\.png/.test(n)) ? names : null;
    }, null, { timeout: 1500 }).then((h) => h.jsonValue(), () => null);
    let shown = null;
    for (let i = 0; i < 15 && !shown; i++) {
      if (i) await page.waitForTimeout(400);       // the listing that makes it a link is on its way
      if (!(await clickText(page, "shots/ipad/", mod, true))) continue;
      shown = await page.waitForSelector("#files:not([hidden])", { timeout: 8000 }).then(shownIn, () => null);
      break;
    }
    // Check the outside folder separately, after the wanted click finished and listings settled.
    // Its asynchronous opening must not be mistaken for an outside click opening Files.
    if (!(await hidden())) await toggleFiles();
    const other = await clickText(page, `${path.basename(outside)}/`, mod);
    const closed = await page.waitForSelector("#files:not([hidden])", { timeout: 1500 }).then(() => false, () => true);
    check("AC-35 a folder's printed path opens Files with the folder selected and open; a folder Files cannot show is no link",
      closed && other && !!shown, JSON.stringify({ closed, other, shown }));
    const isOpen = !(await hidden());
    if (isOpen !== wasOpen) await toggleFiles();
  } finally {
    fs.rmSync(outside, { recursive: true, force: true });
    fs.rmSync(path.join(work, "shots"), { recursive: true, force: true });
  }
}
