// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// More checks for e2e_panel.mjs: the page's View (`/fb/?view=`), the panel inside a page that
// receives its files as messages (AC-70), and a host's files offer (AC 83).

import { check, within } from "./e2e_harness.mjs";

/** AC-70 the View alone: renders the file, no tree, no secret in its URL, a path bar. */
export async function viewerTools(context, base, SB, errors) {
  const vp = await context.newPage();
  await vp.setViewportSize({ width: 1200, height: 800 });
  vp.on("pageerror", (e) => errors.push("viewer: " + e.message));
  await vp.goto(`${base}/fb/?view=${encodeURIComponent(SB + "/README.md")}`);
  await within("AC-70 the View renders the file", (ms) => vp.waitForSelector(".md-body h1", { timeout: ms }));
  check("AC-70 the View has no tree", !(await vp.isVisible("#tree")));
  const barText = () => vp.textContent("#pathbar .path");
  check("AC-70 its path bar shows the full path", (await barText()) === `${SB}/README.md`, await barText());
  check("AC-70 its title is the full path", (await vp.title()) === `${SB}/README.md`, await vp.title());
  await vp.click('.md-body a:text("guide")');
  await within("AC-70 the path bar follows the active tab", (ms) => vp.waitForFunction((p) => document.querySelector("#pathbar .path")?.textContent === p, `${SB}/docs/guide.md`, { timeout: ms }));
  await context.grantPermissions(["clipboard-read", "clipboard-write"], { origin: base });
  await vp.click("#pathbar button");
  await vp.waitForTimeout(200);
  const clip = await vp.evaluate(() => navigator.clipboard.readText());
  check("AC-70 copy puts the full path on the clipboard", clip === `${SB}/docs/guide.md`, clip);
  check("AC-70 copy says so", (await vp.textContent("#toast")) === `Copied ${SB}/docs/guide.md`, await vp.textContent("#toast"));
  await vp.close();
}

/** AC-70 in a page around it (same origin), a file opens in the page's View: the panel posts
 *  `fb-open` (with `window` for ⌘-click) and opens no tab of its own. */
export async function embedTools(context, url, row) {
  const ep = await context.newPage();
  await ep.goto(new URL("/custom.css", url).href);   // any same-origin document will do
  await ep.evaluate((src) => {
    window.__msgs = [];
    addEventListener("message", (e) => window.__msgs.push(e.data));
    document.body.append(Object.assign(document.createElement("iframe"), { src, id: "files", style: "width:500px;height:800px" }));
  }, url);
  const frame = ep.frameLocator("#files");
  await frame.locator(row("README.md")).waitFor();
  await ep.waitForTimeout(500);                       // the pane's saved tabs are restored
  const tabs = await frame.locator(".tab").count();
  await frame.locator(row("README.md")).click();
  await frame.locator(row("README.md")).click({ modifiers: ["Meta"] });
  await ep.waitForTimeout(300);
  const msgs = await ep.evaluate(() => window.__msgs);
  const path = new URL(url).searchParams.get("cwd") + "/README.md";
  check("AC-70 a click asks the page to open the file in its View", msgs.some((m) => m.type === "fb-open" && m.path === path && m.host === null && !m.window), JSON.stringify(msgs));
  check("AC-70 ⌘-click asks for a window of its own", msgs.some((m) => m.type === "fb-open" && m.path === path && m.window === true), JSON.stringify(msgs));
  check("AC-70 … and the embedded panel opens no tab itself", (await frame.locator(".tab").count()) === tabs, `${tabs} tabs before`);
  await ep.close();
}

/** AC 83 a host's pane: Files offers to enable its files; a failure is loud. */
export async function hostOffer(context, url) {
  const hp = await context.newPage();
  await hp.goto(url);
  await within("AC 83 a host's pane offers to browse its files", (ms) => hp.waitForSelector('#offer.on button[data-a="enable"]', { timeout: ms }));
  check("AC 83 … naming the host", (await hp.textContent("#offer b")) === "devbox.example is a remote host.", await hp.textContent("#offer b"));
  check("AC-51 the badge says remote", (await hp.textContent("#mode")) === "remote");
  await within("AC-51 a host that is not connected says so", (ms) => hp.waitForFunction(() => document.getElementById("tree").textContent.includes("devbox.example is not connected"), null, { timeout: ms }));
  await hp.click('#offer button[data-a="enable"]');
  await within("AC 83 Enable on an unknown host fails loud", (ms) => hp.waitForFunction(() => document.getElementById("toast").textContent.includes("no host named devbox.example"), null, { timeout: ms }));
  await hp.click("#tree", { button: "right", position: { x: 40, y: 200 } });
  await hp.waitForSelector(".ctx .mi");
  check("AC 83 the menu can enable it later", !!(await hp.$('.ctx .mi:has-text("Browse Files of devbox.example")')));
  await hp.keyboard.press("Escape");
  await hp.close();
}
