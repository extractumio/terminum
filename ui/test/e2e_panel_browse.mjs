// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-275: Open folder… shows any folder in the pane's place (full, `~/…` from the home the page
// names, or relative to the shown one); what cannot be listed says why in place; the choice
// stays through a cd (the page reloads the frame) and a reload, until Back shows the pane's folder.
import { base, check, PANE_B, SB, within, wsKey } from "./e2e_harness.mjs";

const url = (cwd) => `${base}/fb/?${new URLSearchParams({ key: PANE_B, cwd, home: SB })}`;
const row = (p) => `#tree .row[data-p="${SB}/${p}"]`;

export async function panelBrowse(page) {
  const headers = { "X-TM-Session": await page.evaluate(() => localStorage.getItem("tm.session")) };
  const read = async () => (await page.request.get(`${base}/fb/api/workspace?${new URLSearchParams({ key: wsKey(PANE_B) })}`, { headers })).json();
  const crumbs = () => page.getAttribute("#crumbs", "title");
  const back = () => page.isVisible("#back");
  const go = async (text) => { await page.fill("#goto", text); await page.press("#goto", "Enter"); };

  await page.goto(url(`${SB}/src`));
  await page.waitForSelector(row("src/main.rs"));
  check("AC-275 following the pane: no Back", !(await back()));
  await page.click("#browse");
  check("AC-275 Open folder… shows the field with the shown folder", await page.isVisible("#goto")
    && (await page.inputValue("#goto")) === `${SB}/src/`, await page.inputValue("#goto"));

  await go("~/no-such-folder");
  await within("AC-275 an unreadable path says why in place", (ms) => page.waitForFunction(() => document.getElementById("goto-why").textContent.includes("no-such-folder"), null, { timeout: ms }));
  check("AC-275 … the field stays, the tree unchanged", await page.isVisible("#goto") && !!(await page.$(row("src/main.rs"))) && !(await back()),
    await page.textContent("#goto-why"));
  await page.press("#goto", "Escape");
  check("AC-275 Esc closes the field and its message", !(await page.isVisible("#goto")) && !(await page.isVisible("#goto-why")));

  await page.click("#browse");
  await go("../docs");
  await within("AC-275 a relative path: the tree shows that folder", (ms) => page.waitForSelector(row("docs/guide.md"), { timeout: ms }));
  check("AC-275 … the header names it, Back names the session's folder", (await crumbs()) === `${SB}/docs` && await back()
    && (await page.getAttribute("#back", "title")).endsWith(`${SB}/src`), JSON.stringify([await crumbs(), await page.getAttribute("#back", "title")]));
  await within("AC-275 … kept in the pane's workspace at once", async (ms) => {
    const end = Date.now() + ms;
    while (Date.now() < end) { const w = await read(); if (w.browse === `${SB}/docs` && w.root === `${SB}/src`) return; await page.waitForTimeout(100); }
    throw new Error(JSON.stringify(await read()));
  });

  // a cd: the page reloads the frame with the new folder; the chosen one stays
  await page.goto(url(SB));
  await within("AC-275 a cd in the pane keeps the chosen folder", (ms) => page.waitForSelector(row("docs/guide.md"), { timeout: ms }));
  check("AC-275 … Back now names the pane's new folder", await back() && (await page.getAttribute("#back", "title")).endsWith(`: ${SB}`),
    await page.getAttribute("#back", "title"));

  await page.click("#back");
  await within("AC-275 Back shows the session's folder", (ms) => page.waitForSelector(row("src"), { timeout: ms }));
  await page.waitForTimeout(300);
  const w = await read();
  check("AC-275 … Back hidden, the workspace follows the pane again", !(await back()) && (await crumbs()) === SB && w.browse === "" && w.root === SB, JSON.stringify(w));

  await page.click("#browse");
  await go("~/docs");
  await within("AC-275 ~/… is the pane's home", (ms) => page.waitForSelector(row("docs/guide.md"), { timeout: ms }));
  await page.click("#back");
  await page.waitForSelector(row("src"));
}
