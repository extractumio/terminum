// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Regression scenarios from the full-tree review, driving the real page and DOM with fake tmux messages.
import { check, runningIn } from "./check.mjs";
import { drawn, group, item, openPage, pause, SIZES } from "./fake.mjs";

const A = "tmux:local:terminum:%1", B = "tmux:devbox:default:%1";
const QA = "tmux:local:terminum:%8", QB = "tmux:devbox:default:%8";
const one = (id, extra = {}) => item(id, id === A ? "One" : "Two", extra);
const layout = (id) => [group("main", [one(id)])];

export async function webReview(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const row = { r: [[["$", " ", "a", "😀", "", "b"], null, null, 0], [" ", null, null, 256]], e: true };
    const f = await openPage(browser, { device, layout: layout(A), screen: [row] });
    const { page, sent } = f;
    try {
      await drawn(page);
      const start = sent.length;
      const tapped = await page.evaluate(() => {
        const row = document.querySelector("#screen .ln"), walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
        let text;
        while ((text = walker.nextNode()) && !text.data.includes("😀")) {}
        const at = text.data.indexOf("😀"), range = document.createRange();
        range.setStart(text, at); range.setEnd(text, at + 2);
        const r = range.getBoundingClientRect(), x = r.left + 1, y = r.top + r.height / 2;
        document.getElementById("term").dispatchEvent(new MouseEvent("click", { bubbles: true, altKey: true, clientX: x, clientY: y }));
        return { text: row.textContent, at };
      });
      await pause(page);
      const moved = sent.slice(start).filter((m) => m.t === "in").map((m) => m.data).join("");
      check("review: a shell tap before an emoji sends two Left keys, preserving the Unicode editing offset", moved === "\x1b[D\x1b[D", JSON.stringify({ tapped, moved }));

      const unicode = await page.evaluate(async () => {
        const { rangeOfText } = await import("/marktext.js");
        const el = document.createElement("div");
        el.append(document.createTextNode("İ"), document.createTextNode("x İ"), document.createTextNode("x"));
        document.body.append(el);
        const ns = [...el.childNodes];
        const got = [rangeOfText(ns, "x"), rangeOfText(ns, "x", 1), rangeOfText(ns, "i\u0307x")].map((r) => r.toString());
        el.remove();
        return got;
      });
      check("review: Unicode search highlights original characters across text nodes without an invalid Range", unicode.join("|") === "x|x|İx", JSON.stringify(unicode));

      await page.evaluate(() => {
        Object.defineProperty(navigator, "clipboard", { configurable: true, value: { readText: () => new Promise((resolve) => { window.reviewRead = resolve; }) } });
        document.querySelector('#hotkeys [data-action="paste"]').click();
      });
      f.say({ t: "layout", groups: layout(B) });
      await page.waitForFunction(() => document.querySelector('#sessions .item[aria-current="true"]')?.dataset.id === "tmux:devbox:default:%1");
      const beforePaste = sent.length;
      await page.evaluate(() => window.reviewRead("synthetic clipboard"));
      await pause(page);
      check("review: a clipboard permission completed after a pane switch pastes nothing into the new pane", !sent.slice(beforePaste).some((m) => m.t === "in"), JSON.stringify(sent.slice(beforePaste)));
      await page.evaluate(() => document.querySelector('#hotkeys [data-action="paste"]').click());
      await page.evaluate(() => window.reviewRead("synthetic same pane"));
      await pause(page);
      check("review: clipboard text still pastes when its original pane remains shown", sent.slice(beforePaste).some((m) => m.t === "in" && m.data.includes("synthetic same pane")));

      await page.evaluate(async () => {
        const { Find } = await import("/find.js");
        const original = window.setTimeout;
        const callbacks = [];
        window.setTimeout = (fn, ms, ...args) => ms === 30000 ? (callbacks.push(fn), 0) : original(fn, ms, ...args);
        const requests = [];
        const finder = Object.assign(Object.create(Find.prototype), { q: "", hits: [], jumper: { token: 0, clear() {} }, send: (m) => requests.push(m) });
        document.getElementById("findq").value = "needle";
        try { finder.find(); callbacks[0](); finder.find(); window.reviewSearch = requests.length; }
        finally { window.setTimeout = original; }
      });
      check("review: a timed-out Find retries the unchanged text", await page.evaluate(() => window.reviewSearch === 2));
      check("review: no page errors", !f.errors.length, f.errors.join(" | "));
    } finally { await f.close(); }
  }

  runningIn(`${name} 1400×900`);
  const f = await openPage(browser, { layout: layout(A) });
  try {
    const { page, sent } = f;
    await drawn(page);
    // Embedded frames are blank here so all fake messages continue to reach the parent page.
    await page.route(/\/\?embed=/, (route) => route.fulfill({ contentType: "text/html", body: "<!doctype html>" }));
    const quickLayout = (id, quick) => [group("main", [one(id, { quick }), item(quick, "quick shell", { quickOf: id, shell: false, sub: ["npm", "quick shell"] })])];
    f.say({ t: "layout", groups: quickLayout(A, QA) });
    await page.waitForSelector("#quickclose", { state: "visible" });
    await page.click("#quickclose");
    await page.waitForSelector("#confirmbox[open]");
    f.say({ t: "layout", groups: quickLayout(B, QB) });
    await page.waitForFunction(() => document.querySelector('#sessions .item[aria-current="true"]')?.dataset.id === "tmux:devbox:default:%1");
    await page.click("#confirmok");
    await pause(page);
    const closed = sent.filter((m) => m.t === "kill");
    check("review: a quick-shell confirmation closes only its captured shell after another pane becomes shown", closed.length === 1 && closed[0].id === QA, JSON.stringify(closed));
    await page.click("#quickclose");
    await page.waitForSelector("#confirmbox[open]");
    await page.click('#confirmbox button[value="cancel"]');
    await pause(page);
    check("review: cancelling a quick-shell confirmation sends no additional close", sent.filter((m) => m.t === "kill").length === closed.length);
    check("review: quick-shell scenario has no page errors", !f.errors.length, f.errors.join(" | "));
  } finally { await f.close(); }
}
