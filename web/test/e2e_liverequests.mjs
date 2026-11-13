// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-02: a real history reply lost between private terminumd and its page, then scrolling again.
import { check, shownPane, until } from "./check.mjs";

export async function liveRequests(page, { base, tmux, work }) {
  const pane = await shownPane(page, { tmux, work });
  const messages = [], errors = [];
  let held = null, wire = null, drop = true;
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.routeWebSocket(`${base.replace("http", "ws")}/ws`, (ws) => {
    const server = ws.connectToServer();
    wire = ws;
    ws.onMessage((text) => { messages.push(JSON.parse(String(text))); server.send(text); });
    server.onMessage((text) => {
      const m = JSON.parse(String(text));
      if (drop && m.t === "hist" && m.mode === "prepend") { held = text; drop = false; return; }
      ws.send(text);
    });
  });
  // Only this harness's private pane receives the command; its history exceeds the initial page.
  tmux("send-keys", "-t", pane, "-l", "i=0; while [ $i -lt 2200 ]; do printf 'request-%04d\\n' $i; i=$((i+1)); done");
  tmux("send-keys", "-t", pane, "Enter");
  await page.waitForFunction(() => document.getElementById("screen").textContent.includes("request-2199"), null, { timeout: 15000 });
  await page.reload();
  await page.waitForFunction(() => document.querySelectorAll("#hist .ln").length > 900, null, { timeout: 10000 });
  await page.clock.install();
  const scroll = () => page.evaluate(() => {
    const term = document.getElementById("term"); term.scrollTop = 0; term.dispatchEvent(new Event("scroll"));
  });
  await scroll();
  check("AC-02 live: terminumd answered the first scroll and the transport dropped its reply", await until(() => held !== null));
  const first = await page.locator("#hist .ln").first().textContent();
  await page.clock.fastForward(15001);
  check("AC-02 live: a lost history reply times out visibly", (await page.textContent("#toast")).includes("did not answer"));
  await scroll();
  check("AC-02 live: another scroll retries and draws older real tmux history", await until(async () => (await page.locator("#hist .ln").first().textContent()) !== first));
  const reads = messages.filter((m) => m.t === "more");
  check("AC-02 live: the retry has a different request identity", reads.length >= 2 && reads[0].ref !== reads[1].ref);
  const count = await page.locator("#hist .ln").count();
  wire.send(held);
  await page.waitForTimeout(100);
  check("AC-02 live: the expired reply cannot redraw or duplicate its lines", await page.locator("#hist .ln").count() === count);
  check("AC-02 live: no browser exceptions", errors.length === 0, errors.join(" | "));
}
