// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The header's second line (AC-01, header.js) against a fake terminumd (fake.mjs), at both sizes:
// the folder with its Copy, tmux's session:window, then what runs there; Copy copies the full path;
// the folder its coding agent works in when it moved (a worktree), and Files' root, unless the menu's
// "Follow the agent's folder" is off (AC-51).
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
      root: new URLSearchParams(new URL(document.getElementById("files").getAttribute("src") ?? "", location.href).search).get("cwd") }));
    const on = await seen();
    await g.page.evaluate(() => { const b = document.getElementById("followagent"); b.checked = false; b.dispatchEvent(new Event("change")); });
    await pause(g.page, 300);
    const off = await seen();
    check("AC-51 following the agent (the default): the header's folder and Files' root are where the agent works; off in the menu, the pane's",
      on.dir === "~/work/api/worktrees/fix" && on.copy === `Copy ${AGENT}` && (!wide || on.root === AGENT)
        && off.dir === "~/work/api" && off.copy === "Copy /Users/alex/work/api" && (!wide || off.root === "/Users/alex/work/api"), JSON.stringify({ on, off }));
    check("AC-51 no page errors (following the agent)", !g.errors.length, g.errors.join(" | "));
    await g.close();
  }
  runningIn(name);
}
