// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The page against a real private terminumd and a real private tmux server (never the
// user's): sign-in from a one-time code, the list following tmux, a pane's screen, typing,
// sizing for the view, Screen mode for a full-screen program, Files docked beside the
// terminal, custom.css, settings and themes changed live, and a terminumd restart.
// Chromium and WebKit, at a wide (1400×900) and a phone (390×844) size.
//
//   node web/test/e2e_live.mjs [chromium|webkit]
//     (needs terminumd built: cd terminumd && cargo build; and ui built: cd ui && npm run build)

import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { check, playwright, results, runningIn, within } from "./check.mjs";
import { livePaste } from "./e2e_livepaste.mjs";
import { liveQuick } from "./e2e_livequick.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const BIN = process.env.TERMINUMD_BIN ?? path.join(ROOT, "terminumd/target/debug/terminumd");
const PORT = 47969, SOCKET = "tm-test-live";
const BASE = `http://127.0.0.1:${PORT}`;
const tmp = fs.mkdtempSync("/tmp/tml-");
const HOME = path.join(tmp, "h"), APP = path.join(tmp, "a"), WORK = fs.realpathSync(fs.mkdtempSync("/tmp/tmlw-"));
fs.mkdirSync(HOME);
for (const f of ["alpha.txt", "beta.md"]) fs.writeFileSync(path.join(WORK, f), `${f}\n`);
const UI_BUILD = fs.readFileSync(path.join(ROOT, "ui/.build-id"), "utf8").trim();
const tmux = (...a) => { try { return execFileSync("tmux", ["-L", SOCKET, ...a], { encoding: "utf8" }).trim(); } catch { return ""; } };
/** This test's tmux server gone (after terminumd stopped: running, it would start it again). */
const purge = () => tmux("kill-server");

let proc = null;
const onSocket = (method, p, body) => new Promise((resolve, reject) => {
  const req = http.request({ socketPath: path.join(APP, "terminumd.sock"), method, path: p, headers: { "Content-Type": "application/json" } }, (res) => {
    let t = ""; res.on("data", (d) => (t += d)); res.on("end", () => resolve(JSON.parse(t || "null")));
  });
  req.on("error", reject);
  req.end(body === undefined ? undefined : JSON.stringify(body));
});
async function start() {
  // as the build the Files panel was bundled for: another one makes the panel reload itself (AC-34)
  const env = { ...process.env, TM_HOME: HOME, TM_APP_DIR: APP, TM_PORT: String(PORT), TM_TMUX_SOCKET: SOCKET, TM_LOG: "warn", TM_BUILD_ID: UI_BUILD };
  delete env.TMUX;
  proc = spawn(BIN, [], { env, stdio: ["ignore", "ignore", "inherit"] });
  for (let i = 0; i < 100; i++) {
    try { await onSocket("GET", "/health"); return; } catch { await new Promise((r) => setTimeout(r, 100)); }
  }
  throw new Error("terminumd did not start");
}
const stop = () => new Promise((r) => { if (!proc || proc.exitCode !== null) return r(); proc.once("exit", r); proc.kill(); });
const signIn = async (page) => {
  const { code } = await onSocket("POST", "/code", {});
  await page.goto(`${BASE}/open/${code}`);
  await page.waitForFunction(() => document.querySelectorAll("#sessions .item").length > 0, null, { timeout: 10000 });
};
const screenText = (page) => page.evaluate(() => document.getElementById("screen").innerText);

async function wide(browser, name) {
  runningIn(`${name} 1400×900`);
  const ctx = await browser.newContext({ viewport: { width: 1400, height: 900 } });
  const page = await ctx.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await signIn(page);
  check("AC-09 the page signs in from a one-time code; its address holds no secret", new URL(page.url()).pathname === "/" && !page.url().includes("open"));
  check("AC-09 the sign-in is kept in the origin's storage", await page.evaluate(() => /^[0-9a-f]{64}$/.test(localStorage.getItem("tm.session") ?? "")));
  check("AC-01 the list shows the session main", (await page.textContent("#sessions")).includes("main"));
  await within("AC-02 the shown pane's screen is drawn", () => page.waitForFunction(() => document.querySelectorAll("#screen .ln").length > 3, null, { timeout: 5000 }));
  await page.keyboard.type(`cd ${WORK} && echo live-$((6*7))`);
  await page.keyboard.press("Enter");
  await within("AC-03 typed keys reach the pane and echo", () => page.waitForFunction(() => document.getElementById("screen").innerText.includes("live-42"), null, { timeout: 5000 }));
  await livePaste(page, { home: HOME, name });
  const size = tmux("list-windows", "-F", "#{window_width}x#{window_height} #{window-size}");
  check("AC-13 the pane is sized for this view (window-size manual)", /^\d+x\d+ manual/.test(size) && Number(size.split("x")[0]) > 80, size);
  await liveQuick(page, { tmux, work: WORK });
  tmux("new-window", "-d", "-n", "byhand");
  await within("AC-01 a window made by hand shows in the list", () => page.waitForFunction(() => document.getElementById("sessions").innerText.includes("byhand"), null, { timeout: 3000 }));
  await within("AC-70 Files is docked and shows the pane's folder", async () => {
    const frame = await (await page.waitForSelector("#files:not([hidden])", { timeout: 5000 })).contentFrame();
    await frame.waitForFunction(() => document.body.innerText.includes("alpha.txt"), null, { timeout: 8000 });
  });
  const files = await (await page.$("#files")).contentFrame();
  await files.click("#new-file");
  await files.waitForSelector("#tree input", { timeout: 3000 }).catch(() => {});
  await page.keyboard.type(`fresh-${name}.txt`);                // the folder is shared by both browsers
  await page.keyboard.press("Enter");
  await within("AC-70 a new file opens in View, where a click opens files, not under the tree", async () => {
    await page.waitForSelector("#filewin:not([hidden])", { timeout: 5000 });
    const view = await (await page.$("#file")).contentFrame();
    await view.waitForSelector(`.tab.active .tname:text("fresh-${name}.txt")`, { timeout: 5000 });
    if (await files.$(".tab")) throw new Error("a tab opened in Files too");
  });
  await page.click("#fileclose");
  await page.click("#term");
  await page.keyboard.type("vim -u NONE -N");
  await page.keyboard.press("Enter");
  await within("AC-05 a full-screen program switches the page to Screen mode", () => page.waitForFunction(() => document.body.innerText.includes("Screen ·") || document.getElementById("term").classList.contains("screenmode"), null, { timeout: 5000 }));
  await page.keyboard.type(":q");
  await page.keyboard.press("Enter");
  await within("AC-05 Screen mode ends with the program", () => page.waitForFunction(() => !document.getElementById("term").classList.contains("screenmode"), null, { timeout: 5000 }));
  fs.writeFileSync(path.join(HOME, "custom.css"), "#nav { --nav-w: 333px }\n");
  await within("AC-19 custom.css applies without a reload", () => page.waitForFunction(() => getComputedStyle(document.getElementById("nav")).getPropertyValue("--nav-w").trim() === "333px", null, { timeout: 5000 }));
  fs.writeFileSync(path.join(HOME, "settings.json"), JSON.stringify({ size: "biggest" }));
  await within("AC-19 a wrong setting is said once as a toast", () => page.waitForFunction(() => document.getElementById("toast").innerText.includes("size must be latest, largest or manual"), null, { timeout: 5000 }));
  fs.writeFileSync(path.join(HOME, "settings.json"), "{}");
  const theme = JSON.parse(fs.readFileSync(path.join(HOME, "themes/local.json"), "utf8"));
  fs.writeFileSync(path.join(HOME, "themes/local.json"), JSON.stringify({ ...theme, bg: "#102030" }));
  await within("AC-12 a changed theme file reaches the page without a reload", () => page.waitForFunction(() => getComputedStyle(document.documentElement).getPropertyValue("--t-bg").trim() === "#102030", null, { timeout: 5000 }));
  await stop();
  await start();
  await within("AC-14 the page reconnects after terminumd restarts and shows the pane", () => page.waitForFunction(() => document.querySelectorAll("#screen .ln").length > 3 && !document.getElementById("status").innerText.includes("not running"), null, { timeout: 8000 }));
  check("no page errors", errors.length === 0, errors.slice(0, 3).join(" | "));
  return ctx;
}

async function phone(browser, name) {
  runningIn(`${name} 390×844`);
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: name === "chromium", hasTouch: true });
  const page = await ctx.newPage();
  await signIn(page);
  await page.waitForFunction(() => document.querySelectorAll("#screen .ln").length > 3, null, { timeout: 8000 });
  const before = tmux("display", "-p", "#{window_width}");
  await page.evaluate(() => { const k = document.getElementById("kbd"); k.focus(); });
  await page.keyboard.type("echo phone");
  await page.keyboard.press("Enter");
  await within("AC-13 a phone that types sizes the pane to its narrow view", async () => {
    for (let i = 0; i < 50; i++) {
      const w = Number(tmux("display", "-p", "#{window_width}"));
      if (w > 0 && w < 60) return;
      await new Promise((r) => setTimeout(r, 100));
    }
    throw new Error(`width ${tmux("display", "-p", "#{window_width}")} (was ${before})`);
  });
  await ctx.close();
}

try {
  purge();
  await start();
  const pw = await playwright();
  const which = process.argv.slice(2).filter((a) => ["chromium", "webkit"].includes(a));
  for (const name of which.length ? which : ["chromium", "webkit"]) {
    const browser = await pw[name].launch();
    try {
      const ctx = await wide(browser, name);
      await phone(browser, name);
      await ctx.close();
    } catch (e) {
      check("ran to its end", false, String(e.stack ?? e).split("\n").slice(0, 3).join(" | "));
    } finally { await browser.close(); }
    await stop();
    purge();
    fs.rmSync(path.join(HOME, "themes"), { recursive: true, force: true });
    fs.rmSync(path.join(HOME, "custom.css"), { force: true });
    await start();
  }
} finally {
  runningIn("");
  await stop();
  purge();
  fs.rmSync(tmp, { recursive: true, force: true });
  fs.rmSync(WORK, { recursive: true, force: true });
}
const failed = results.filter((r) => !r.ok).length;
console.log(failed ? `FAIL (${failed} of ${results.length})` : `PASS (${results.length} checks)`);
process.exit(failed ? 1 : 0);
