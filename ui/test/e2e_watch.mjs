// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-61: retain the event/request timeline and the visible state of a failed live refresh.
import { within } from "./e2e_harness.mjs";

export function watchDiagnostics(page) {
  const events = [], start = Date.now();
  const record = (kind, data) => {
    events.push({ ms: Date.now() - start, kind, data });
    if (events.length > 160) events.shift();
  };
  page.on("websocket", (ws) => {
    record("socket", new URL(ws.url()).pathname);
    ws.on("framereceived", ({ payload }) => {
      try { record("event", JSON.parse(String(payload))); } catch { /* not JSON */ }
    });
    ws.on("close", () => record("socket-close", null));
  });
  page.on("request", (r) => {
    const u = new URL(r.url());
    if (u.pathname.startsWith("/fb/api/")) record("request", { method: r.method(), path: u.pathname + u.search });
  });
  page.on("response", (r) => {
    const u = new URL(r.url());
    if (u.pathname.startsWith("/fb/api/")) record("response", { status: r.status(), path: u.pathname + u.search });
  });
  return (name, fn, ms = 5000) => within(name, async (timeout) => {
    record("check", name);
    try { await fn(timeout); }
    catch (e) {
      const state = await page.evaluate(() => ({
        tabs: [...document.querySelectorAll(".tab")].map((t) => ({ text: t.textContent, class: t.className })),
        editor: document.querySelector(".cm-content")?.textContent,
        banners: [...document.querySelectorAll(".banner")].map((b) => b.textContent),
        rows: [...document.querySelectorAll("#tree .row")].map((r) => r.dataset.p),
        notice: document.querySelector("#notice")?.textContent,
      }));
      const health = await page.evaluate(async () => {
        const r = await fetch("/fb/api/health", { headers: { "X-TM-Session": localStorage.getItem("tm.session") ?? "" } });
        const h = await r.json();
        return { status: r.status, watchedDirs: h.watched_dirs, cacheDirs: h.cache_dirs };
      }).catch((e) => ({ error: e.message }));
      console.log("     | watch diagnostics", JSON.stringify({ startedAt: new Date(start).toISOString(), events, health, state }));
      throw e;
    }
  }, ms);
}
