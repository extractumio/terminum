// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The header's second line (AC-01, header.js) against a fake terminumd (fake.mjs), at both sizes:
// the folder with its Copy, tmux's session:window, then what runs there; Copy copies the full path;
// the folder its coding agent works in when it moved (a worktree), and Files' root, unless the menu's
// "Follow the agent's folder" is off (AC-51); so does its row in the list.
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
      const inline = parts.filter((e) => getComputedStyle(e).position !== "absolute");   // a phone's Copy stands in a column of its own
      const boxes = inline.map((e) => e.getBoundingClientRect()), overlap = boxes.some((b, i) => i && b.left < boxes[i - 1].right - 0.5);
      return { parts: parts.map((e) => e.className + ":" + e.textContent), overlap };
    });
    check("AC-01 the header's second line: the folder and its Copy, then session:window, then what runs there; no part over another",
      JSON.stringify(line.parts) === JSON.stringify(["dir:~/work/api", "copydir:", "where:main:3", "set:claude·local·Fixing the tests"]) && !line.overlap, JSON.stringify(line));
    const target = await page.evaluate(() => {      // the Copy's finger target (its ::after), the title's text, the header
      document.querySelector("#current .t").textContent = "A long session title that runs past the phone's width and ends in an ellipsis";
      const b = document.querySelector("#cursub .copydir"), r = b.getBoundingClientRect(), a = getComputedStyle(b, "::after");
      const t = document.querySelector("#current .t").getBoundingClientRect(), bar = document.getElementById("bar").getBoundingClientRect();
      const box = { top: r.top + parseFloat(a.top), bottom: r.bottom - parseFloat(a.bottom), left: r.left + parseFloat(a.left), right: r.right - parseFloat(a.right) };
      const mid = document.elementFromPoint(t.left + t.width / 2, t.top + t.height / 2);
      return { w: Math.round(box.right - box.left), h: Math.round(box.bottom - box.top),
        overTitle: box.left < t.right && box.right > t.left && box.top < t.bottom && box.bottom > t.top || !!mid?.closest(".copydir"),
        inBar: box.top >= bar.top && box.bottom <= bar.bottom && box.left >= bar.left && box.right <= bar.right,
        clear: [document.getElementById("rename"), document.getElementById("menubtn"), getComputedStyle(b).position === "absolute" && document.querySelector("#cursub .dir")].filter(Boolean).every((e) => {   // ✎, ⋯ and, in a column of its own, the folder
          const o = e.getBoundingClientRect(); return !o.width || o.right <= box.left + 0.5 || o.left >= box.right - 0.5; }) };
    });
    check("AC-01 the folder's Copy: its target never over the title's text (a long title's middle opens the list), inside the header, clear of ✎ and ⋯ (and the folder on a phone), a finger's 44 × 44 on a phone",
      !target.overTitle && target.inBar && target.clear && (!size.startsWith("390") || (target.w >= 44 && target.h >= 44)), JSON.stringify(target));
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
    const tap = async (sel, top) => {               // a tap at an element's top centre (1 px in) or its centre; what it did
      const [x, y] = await page.evaluate(([s, top]) => { const r = document.querySelector(s).getBoundingClientRect(); return [r.left + r.width / 2, top ? r.top + 1 : r.top + r.height / 2]; }, [sel, top]);
      const n = await page.evaluate(() => window.copied.length);
      await page.mouse.click(x, y);
      await pause(page, 300);
      return page.evaluate((n) => ({ copied: window.copied.length > n, files: document.querySelector('#views [data-view="files"]').ariaSelected, app: document.getElementById("app").className }), n);
    };
    if (size.startsWith("390")) {
      const files = await tap('#views [data-view="files"]', true);
      check("AC-01 a phone: a tap at the Files tab's top centre opens Files, not the folder's Copy", files.files === "true" && !files.copied, JSON.stringify(files));
      const before = await page.evaluate(() => document.getElementById("app").className);
      const title = await tap("#current .t", false);
      check("AC-01 a phone: a tap on a long title's middle opens the list, not the folder's Copy", !title.copied && title.app !== before, JSON.stringify({ ...title, before }));
    }
    check("AC-01 no page errors (header)", !f.errors.length, f.errors.join(" | "));
    await f.close();

    // its Claude Code works in a worktree below the pane's folder (its hooks said so)
    const AGENT = "/Users/alex/work/api/worktrees/fix";
    const moved = [group("main", [item(SID, "api", { sub: ["claude", "~/work/api"], index: "3", dir: "/Users/alex/work/api", agentDir: AGENT, agentFolder: "~/work/api/worktrees/fix" })])];
    const g = await openPage(browser, { device, layout: moved, hook: (m, say) => {
      if (m.t === "files") say({ t: "files", sid: SID, key: SID, cwd: "/Users/alex/work/api", home: "/Users/alex", host: null, agent: AGENT });
    } });
    await drawn(g.page);
    await pause(g.page, 300);
    const wide = !size.startsWith("390");
    const seen = () => g.page.evaluate(() => ({ dir: document.querySelector("#cursub .dir")?.textContent, copy: document.querySelector("#cursub .copydir")?.title,
      row: document.querySelector("#sessions .item .s .last")?.textContent, bar: document.querySelector("#sessions .item .pbar")?.title,
      root: new URLSearchParams(new URL(document.getElementById("files").getAttribute("src") ?? "", location.href).search).get("cwd") }));
    const on = await seen();
    const filter = (q) => g.page.evaluate((q) => { const f = document.getElementById("filter"); f.value = q; f.dispatchEvent(new Event("input")); }, q);
    await filter("worktrees");
    await pause(g.page, 200);
    const found = await g.page.evaluate(() => document.querySelectorAll("#sessions .item").length);
    await filter("");
    await pause(g.page, 200);
    await g.page.evaluate(() => { const b = document.getElementById("followagent"); b.checked = false; b.dispatchEvent(new Event("change")); });
    await pause(g.page, 300);
    const off = await seen();
    check("AC-51 following the agent (the default): the header's folder and Files' root are where the agent works; off in the menu, the pane's",
      on.dir === "~/work/api/worktrees/fix" && on.copy === `Copy ${AGENT}` && (!wide || on.root === AGENT)
        && off.dir === "~/work/api" && off.copy === "Copy /Users/alex/work/api" && (!wide || off.root === "/Users/alex/work/api"), JSON.stringify({ on, off }));
    check("AC-01 following the agent: the row names the agent's folder, its bar that folder's, the filter finds it by it; off, the pane's",
      on.row === "~/work/api/worktrees/fix" && on.bar === "Folder ~/work/api/worktrees/fix" && found === 1
        && off.row === "~/work/api" && off.bar === "Folder ~/work/api", JSON.stringify({ on, off, found }));
    check("AC-51 no page errors (following the agent)", !g.errors.length, g.errors.join(" | "));
    await g.close();
  }
  runningIn(name);
}
