// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-52, AC-53, AC-56, AC-71: a 200 from a program that took terminumd's port must not
// erase edits or workspace revisions. Reads prove the server again and retry only once.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { base, check, cleanup, panelUrl, results, SB, signIn, waitFor, within, wsKey } from "./e2e_harness.mjs";
import { NO_SIGNALS } from "./process-stop.mjs";

const FOREIGN = { status: 200, contentType: "application/json", body: "{}" };
const WHY = "the answer on terminumd's port is not terminumd's";

export async function panelForeign(context, errors) {
  let serial = 0;
  const fresh = async () => {
    const page = await context.newPage(), key = `foreign-${serial++}`;
    page.setDefaultTimeout(5000);
    page.on("pageerror", (e) => errors.push(`foreign: ${e.message}`));
    return { page, key, url: panelUrl(key) };
  };
  const row = (name) => `#tree .row[data-p="${SB}/${name}"]`;
  const said = (page, text) => page.waitForFunction((t) => document.getElementById("toast").textContent === t, text);
  const screenshot = async (page, name) => {
    if (!process.env.TM_TEST_FOREIGN_SHOTS) return;
    fs.mkdirSync(process.env.TM_TEST_FOREIGN_SHOTS, { recursive: true });
    await page.screenshot({ path: path.join(process.env.TM_TEST_FOREIGN_SHOTS, `${name}.png`) });
  };

  // The workspace GET already guards its shape (TERM-253), including a body cut off by WebKit.
  for (const body of ["{}", '{"rev":1,"ro', "twice"]) {
    const { page, url } = await fresh();
    try {
      let reads = 0;
      await page.route("**/fb/api/workspace?*", (r) => r.request().method() === "GET" && reads++ < (body === "twice" ? Infinity : 1)
        ? r.fulfill({ ...FOREIGN, body: body === "twice" ? "{}" : body }) : r.fallback());
      await page.goto(url);
      if (body === "twice") await said(page, `Panel state not read: ${WHY}`);
      else await page.waitForSelector(row("src"));
      check(`AC-71 a foreign workspace read (${body}) loads or fails loud after retry`, body === "twice" ? reads === 2 : reads >= 2, String(reads));
    } catch (e) { check("AC-71 foreign workspace read", false, e.message.split("\n")[0]); }
    finally { await page.close(); }
  }

  // Directory pages: a valid empty listing is different from a foreign {} answer.
  for (const repeated of [false, true]) {
    const { page, url } = await fresh();
    try {
      await page.goto(url); await page.waitForSelector(row("src")); // warm the real listing, past status: loading
      let reads = 0, proofs = 0;
      page.on("request", (r) => { if (r.url().includes("/api/hello?")) proofs++; });
      await page.route("**/fb/api/ls?*", (r) => ++reads <= (repeated ? Infinity : 1) ? r.fulfill(FOREIGN) : r.fallback());
      await page.click("#refresh");
      await waitFor(() => reads >= 2);
      if (repeated) await page.waitForSelector(`#tree :text("${WHY}")`);
      else await page.waitForSelector(row("src"));
      check(`AC-52 a foreign listing ${repeated ? "twice fails loud" : "is proved and read again"}`, reads === 2 && proofs === 1, JSON.stringify({ reads, proofs }));
    } catch (e) { check("AC-52 foreign listing", false, e.message.split("\n")[0]); }
    finally { await page.close(); }
  }

  fs.writeFileSync(`${SB}/foreign-read.txt`, "file from terminumd\n");
  for (const repeated of [false, true]) {
    const { page, url } = await fresh();
    try {
      await page.goto(url); await page.waitForSelector(row("foreign-read.txt"));
      let reads = 0, proofs = 0;
      page.on("request", (r) => { if (r.url().includes("/api/hello?")) proofs++; });
      await page.route("**/fb/api/file?*", (r) => ++reads <= (repeated ? Infinity : 1) ? r.fulfill(FOREIGN) : r.fallback());
      await page.click(row("foreign-read.txt"));
      if (repeated) await page.waitForSelector(`#viewer :text("${WHY}")`);
      else await page.waitForFunction(() => document.querySelector(".cm-content")?.textContent.includes("file from terminumd"));
      check(`AC-53 a foreign file read ${repeated ? "twice fails loud" : "is proved and read again"}`, reads === 2 && proofs === 1, JSON.stringify({ reads, proofs }));
      if (repeated) await screenshot(page, "file-read-error");
    } catch (e) { check("AC-53 foreign file read", false, e.message.split("\n")[0]); }
    finally { await page.close(); }
  }

  fs.writeFileSync(`${SB}/foreign-save.txt`, "before\n");
  const file = await fresh();
  try {
    const { page, url } = file;
    await page.goto(url); await page.waitForSelector(row("foreign-save.txt"));
    await page.click(row("foreign-save.txt")); await page.waitForSelector(".cm-content");
    await page.click(".cm-content"); await page.keyboard.press("ControlOrMeta+End"); await page.keyboard.type("after");
    const matches = [];
    const foreignSave = (r) => {
      if (r.request().method() !== "PUT") return r.fallback();
      matches.push(r.request().headers()["if-match"]);
      return r.fulfill(FOREIGN);
    };
    await page.route("**/fb/api/file?*", foreignSave);
    await page.keyboard.press("ControlOrMeta+s"); await said(page, "Not saved: server restarting — save again");
    check("AC-56 a foreign save is sent once, keeps the dirty tab and leaves disk unchanged", matches.length === 1
      && !!await page.$(".tab.active.dirty") && fs.readFileSync(`${SB}/foreign-save.txt`, "utf8") === "before\n");
    await screenshot(page, "unsaved-file");
    await page.click(".tab.active .close"); await page.waitForSelector('.modal:has-text("Save changes to foreign-save.txt?")');
    await page.click('.modal [data-id="cancel"]');
    await page.unroute("**/fb/api/file?*", foreignSave);
    page.on("request", (r) => { if (r.method() === "PUT" && r.url().includes("/fb/api/file?")) matches.push(r.headers()["if-match"]); });
    await page.click(".cm-content"); await page.keyboard.press("ControlOrMeta+s");
    await page.waitForSelector(".tab.active:not(.dirty)");
    check("AC-56 closing still asks; saving again retains the etag and writes the edit", matches.length === 2 && matches[0] === matches[1]
      && fs.readFileSync(`${SB}/foreign-save.txt`, "utf8") === "before\nafter", JSON.stringify(matches));
  } catch (e) { check("AC-56 foreign file save", false, e.message.split("\n")[0]); }
  finally { await file.page.close(); }

  // Cover both a squatter (nothing written) and a response lost after the workspace landed.
  for (const landed of [false, true]) {
    const { page, key, url } = await fresh();
    try {
      await page.goto(url); await page.waitForSelector(row("foreign-read.txt"));
      await page.waitForTimeout(700); // finish any initial workspace save
      const headers = { "X-TM-Session": await page.evaluate(() => localStorage.getItem("tm.session")), "Content-Type": "application/json", Origin: base };
      const address = `${base}/fb/api/workspace?${new URLSearchParams({ key: wsKey(key) })}`;
      const read = async () => (await page.request.get(address, { headers })).json();
      const before = await read();
      let writes = 0;
      const foreignSave = async (r) => {
        if (r.request().method() !== "PUT") return r.fallback();
        writes++;
        if (landed) await r.fetch();
        await r.fulfill(FOREIGN);
      };
      await page.route("**/fb/api/workspace?*", foreignSave);
      await page.click(row("foreign-read.txt")); await said(page, `Panel state not saved: ${WHY}`);
      check(`AC-71 a foreign workspace answer (${landed ? "landed" : "not written"}) fails loud without resending`, writes === 1
        && (await read()).rev === before.rev + (landed ? 1 : 0), String(writes));
      await page.unroute("**/fb/api/workspace?*", foreignSave);
      await page.click(row("foreign-save.txt"));
      await within("AC-71 the next change survives ambiguous state and saves", async () => {
        for (let i = 0; i < 40; i++) {
          if ((await read()).selected.includes(`${SB}/foreign-save.txt`)) return;
          await page.waitForTimeout(100);
        }
        throw new Error("newer workspace selection was not saved");
      });
      const saved = await read();
      const response = await page.request.put(address, { headers, data: { ...saved, selected: [`${SB}/foreign-read.txt`] } });
      check("AC-71 another page's workspace write succeeds", response.ok());
      await within("AC-71 a newer workspace event still updates the panel", (ms) => page.waitForSelector(`${row("foreign-read.txt")}.sel`, { timeout: ms }));
    } catch (e) { check("AC-71 foreign workspace save", false, e.message.split("\n")[0]); }
    finally { await page.close(); }
  }
}

// A focused reproduction on either browser and at either viewport, using the same private harness.
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const browsers = await import(process.env.PLAYWRIGHT_CORE ?? "playwright-core");
  let browser;
  const errors = [];
  try {
    browser = await browsers[process.argv[2] ?? "chromium"].launch(NO_SIGNALS);
    const width = Number(process.argv[3] ?? 1400);
    const context = await browser.newContext({ viewport: { width, height: width === 390 ? 844 : 900 }, hasTouch: width === 390, isMobile: width === 390 });
    const page = await context.newPage();
    await signIn(page); await page.close();
    await panelForeign(context, errors);
    check("no page errors", !errors.length, errors.join("; "));
  } finally { try { await browser?.close(); } finally { await cleanup(); } }
  const failed = results.filter((r) => !r).length;
  console.log(failed ? `FAIL (${failed} of ${results.length})` : `PASS (${results.length} checks)`);
  process.exitCode = failed ? 1 : 0;
}
