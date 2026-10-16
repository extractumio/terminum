// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A fake terminumd for the page's checks of terminum's own UI (e2e_signin, e2e_tmux,
// e2e_screen, e2e_move): web/static served at 127.0.0.1 (a secure context, as the real one),
// /open/<code> with its sign-in meta, /api/hello's proof, /auth/check, /custom.css, and a
// WebSocket that answers `sub` with a theme, an empty history and a screen.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const STATIC = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../static");
export const ORIGIN = "http://127.0.0.1:47999";
const TYPES = { html: "text/html", css: "text/css", svg: "image/svg+xml", js: "text/javascript" };
export const WIDE = { viewport: { width: 1400, height: 900 } };
export const PHONE = { viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true };
/** The sizes a check runs at: [label, newPage options]. */
export const SIZES = [["1400×900", WIDE], ["390×844", PHONE]];

export const THEME = { bg: "#1e1e1e", fg: "#d4d4d4", cursor: "#ffffff", cursorText: "#000000", bold: "#ffffff", useBold: false,
  ansi: Array.from({ length: 16 }, () => "#808080"), font: null, size: 13, dark: true };
export const item = (id, title, extra = {}) => ({ id, title, sub: ["zsh", "~/p"], index: "1", pane: 0, host: null, profile: "local",
  agent: null, shell: true, state: null, window: "@1", alt: false, ...extra });
export const group = (name, items, extra = {}) => ({ kind: "tmux", label: name, session: null, where: "", host: null,
  wid: extra.host ? `${extra.host}/terminum` : `local/${name}`,
  new: "New window", pool: null, items, ...extra });
export const LAYOUT = [group("main", [item("tmux:local:terminum:%1", "api", { sub: ["claude", "~/work/api"] }),
  item("tmux:local:terminum:%2", "docs", { index: "2" })]), group("notes", [item("tmux:local:terminum:%3", "notes")])];
export const line = (text, flags = 0) => ({ r: [[text, null, null, flags]], e: true });
export const SCREEN = [line("$ ls"), line("a.txt b.txt"), { r: [["$ ", null, null, 0], [" ", null, null, 256]], e: true }];

/** The proof terminumd gives for nonce n: HMAC(HMAC(secret, "tm-proof-key"), "tm-hello-v1:" + n). */
export function proofOf(secret, n) {
  const key = crypto.createHmac("sha256", secret).update("tm-proof-key").digest();
  return crypto.createHmac("sha256", key).update(`tm-hello-v1:${n}`).digest("hex");
}

/** Opens the page. secret: what the fake server knows (its proof; "unknown_signin": none, it
 *  answers 404 unknown_signin); meta: the sign-in the page
 *  at /open/<code> carries; stored: one already in the origin's storage; auth: /auth/check's
 *  answer (204, or {login}); hook(msg, say): every message the page sends. */
export async function openPage(browser, { device = WIDE, secret = null, meta = null, stored = null, auth = 204, layout = LAYOUT,
  screen = SCREEN, cols = 80, hook = null, url = "/", css = "", settings = null } = {}) {
  const context = await browser.newContext(device);
  const page = await context.newPage();
  const errors = [], sent = [], requests = [], sockets = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  if (stored) await page.addInitScript((s) => { if (!sessionStorage.getItem("seeded")) { localStorage.setItem("tm.session", s); sessionStorage.setItem("seeded", "1"); } }, stored);
  await page.route(`${ORIGIN}/**`, (route) => {
    const req = route.request(), u = new URL(req.url()), p = u.pathname;
    requests.push({ path: p, query: u.search, headers: req.headers() });
    if (p === "/auth/check") {
      const a = typeof auth === "function" ? auth() : auth;
      return typeof a === "number" ? route.fulfill({ status: a, body: "" })
        : route.fulfill({ status: 401, contentType: "application/json", body: JSON.stringify({ error: "bad_session", ...a }) });
    }
    if (p === "/api/hello") {
      const n = u.searchParams.get("n");
      if (secret === "unknown_signin") return route.fulfill({ status: 404, contentType: "application/json", body: JSON.stringify({ error: "unknown_signin" }) });
      return route.fulfill({ contentType: "application/json", body: JSON.stringify({ proof: secret ? proofOf(secret, n) : "0".repeat(64) }) });
    }
    if (p === "/custom.css") return route.fulfill({ contentType: "text/css", body: typeof css === "function" ? css() : css });
    if (p.startsWith("/fb/")) return route.fulfill({ status: 404, body: "" });
    const f = path.join(STATIC, p === "/" || p.startsWith("/open/") ? "index.html" : path.normalize(p));
    if (!f.startsWith(STATIC) || !fs.existsSync(f)) return route.fulfill({ status: 404, body: "" });
    let body = fs.readFileSync(f);
    if (f.endsWith("index.html")) {
      body = body.toString().replace("__BUILD__", "test");
      if (p.startsWith("/open/") && meta) body = body.replace("</head>", `<meta name="tm-session" content="${meta}">\n</head>`);
    }
    return route.fulfill({ contentType: TYPES[f.split(".").pop()] ?? "application/octet-stream", body });
  });
  let say = () => {};
  await page.routeWebSocket(`${ORIGIN.replace("http", "ws")}/ws`, (ws) => {
    sockets.push({ protocols: ws.protocols(), close: () => ws.close() });
    say = (m) => ws.send(JSON.stringify(m));
    ws.onMessage((m) => {
      const msg = JSON.parse(String(m));
      sent.push(msg);
      if (msg.t === "sub") {
        say({ t: "theme", sid: msg.id, theme: THEME });
        say({ t: "hist", mode: "reset", sid: msg.id, first: 0, oldest: 0, truncated: false, lines: [] });
        say({ t: "screen", sid: msg.id, full: true, n: screen.length, cols, rows: screen.length, alt: false, ch: screen.map((l, i) => [i, l]) });
      }
      hook?.(msg, say);
    });
    say({ t: "build", id: "test" });
    if (settings) say({ t: "settings", ...settings });
    say({ t: "layout", groups: layout });
  });
  await page.goto(ORIGIN + url);
  return { page, context, sent, requests, sockets, errors, say: (m) => say(m), close: () => context.close() };
}

/** Waits until the page drew the shown pane's screen. */
export const drawn = (page) => page.waitForFunction(() => document.querySelectorAll("#screen .ln").length > 0, null, { timeout: 10000 });
export const pause = (page, ms = 150) => page.waitForTimeout(ms);
