// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Panel's own state writes fail loud and are sent again (main.ts saveNow, savePrefs): a refused
// PUT /api/workspace or /api/prefs says so in a toast and the next change sends it again; a write
// whose answer was lost after it landed (stale_rev with its own content) keeps the newer change; a
// write terminumd accepted but could not put on disk is said in the banner until its retry works.
import fs from "node:fs";
import path from "node:path";
import { APP, base, check, PANE_A, within, wsKey } from "./e2e_harness.mjs";

const REFUSED = { status: 500, contentType: "application/json", body: JSON.stringify({ error: "io", message: "disk full (test)" }) };

export async function panelSaves(page) {
  const headers = { "X-TM-Session": await page.evaluate(() => localStorage.getItem("tm.session")) };
  const read = async () => (await page.request.get(`${base}/fb/api/workspace?${new URLSearchParams({ key: wsKey(PANE_A) })}`, { headers })).json();
  // three files of one shown folder (the tree is virtual: the root's own may be out of view); a
  // click selects one and moves no other row
  const rows = await page.$$eval("#tree .row[data-p][data-d='0']", (r) => {
    const by = new Map();
    for (const p of r.map((e) => e.dataset.p)) { const d = p.slice(0, p.lastIndexOf("/")); by.set(d, [...(by.get(d) ?? []), p]); }
    return [...by.values()].find((g) => g.length >= 3)?.slice(0, 3) ?? [];
  });
  const pick = async (p) => { await page.click(`#tree .row[data-p="${p}"]`); await page.waitForTimeout(700); };   // the write is debounced
  const toast = () => page.textContent("#toast");

  // refused once: a toast; the next change goes through
  let refused = 0;
  const once = (route) => { if (route.request().method() !== "PUT" || refused++) return route.fallback(); return route.fulfill(REFUSED); };
  await page.route("**/fb/api/workspace?*", once);
  await pick(rows[0]);
  const said = await toast();
  await pick(rows[1]);
  await page.unroute("**/fb/api/workspace?*", once);
  const saved = await read();
  check("AC-71 a refused workspace write says \"Panel state not saved: …\"; the next change saves", refused >= 2 && said === "Panel state not saved: disk full (test)"
    && JSON.stringify(saved.selected) === JSON.stringify([rows[1]]), JSON.stringify({ refused, said, selected: saved.selected }));

  // the answer lost after the server applied it: the next change meets stale_rev with that same content and is kept
  let lost = 0, done;
  const landed = new Promise((r) => { done = r; });
  const land = async (route) => {
    if (route.request().method() !== "PUT" || lost++) return route.fallback();
    try {
      await route.fetch();                                  // terminumd applies it…
      await route.abort("connectionreset");                 // …and the Panel never hears
    } finally { done(); }
  };
  await page.route("**/fb/api/workspace?*", land);
  await pick(rows[2]);
  await landed;
  await page.unroute("**/fb/api/workspace?*", land);
  const puts = [], seen = async (r) => { if (r.request().method() === "PUT" && r.url().includes("/fb/api/workspace")) puts.push(`${r.status()} ${(await r.text()).slice(0, 400)}`); };
  page.on("response", seen);
  await pick(rows[0]);
  await page.waitForTimeout(700);
  const kept = await read(), shown = await page.$$eval("#tree .row.sel, #tree .row.selected, #tree .row[aria-selected='true']", (r) => r.map((e) => e.dataset.p));
  check("AC-71 a write whose answer was lost after it landed: the newer change is kept and saved, not undone",
    lost >= 1 && JSON.stringify(kept.selected) === JSON.stringify([rows[0]]) && !shown.includes(rows[2]), JSON.stringify({ lost, rows, selected: kept.selected, shown, puts }));
  page.off("response", seen);

  // accepted, but workspaces.json cannot be written (a folder in its place): the banner says so,
  // terminumd tries again by itself, and the banner goes once the write works
  const file = path.join(APP, "workspaces.json");
  fs.rmSync(file, { force: true });
  fs.mkdirSync(path.join(file, "in-the-way"), { recursive: true });
  const banner = () => page.textContent("#notice");
  try {
    await pick(rows[1]);
    await within("AC-71 a workspace write that fails on disk says so in the banner", (ms) =>
      page.waitForFunction(() => document.getElementById("notice").textContent.startsWith("Panel state not saved to disk"), null, { timeout: ms }));
  } finally {
    fs.rmSync(file, { recursive: true, force: true });
  }
  await within("AC-71 the failed write is tried again by itself and the banner goes", (ms) =>
    page.waitForFunction(() => !document.getElementById("notice").textContent.startsWith("Panel state not saved"), null, { timeout: ms }));
  const onDisk = JSON.parse(fs.readFileSync(file, "utf8")).panes?.[wsKey(PANE_A)]?.selected;
  check("AC-71 the retry put what was accepted on disk", JSON.stringify(onDisk) === JSON.stringify([rows[1]]), JSON.stringify({ onDisk, banner: await banner() }));

  // preferences refused once: a toast; the next change sends them again
  let prefs = 0;
  const pOnce = (route) => { if (route.request().method() !== "PUT" || prefs++) return route.fallback(); return route.fulfill(REFUSED); };
  await page.route("**/fb/api/prefs", pOnce);
  await page.dispatchEvent("#splitter", "dblclick");
  await within("AC-52 a refused preferences write says so", (ms) => page.waitForFunction(() => document.getElementById("toast").textContent === "Preferences not saved: disk full (test)", null, { timeout: ms }));
  await page.dispatchEvent("#splitter", "dblclick");
  await page.waitForTimeout(400);
  await page.unroute("**/fb/api/prefs", pOnce);
  check("AC-52 the next preferences change is sent again and saved", prefs >= 2, JSON.stringify({ prefs }));
}
