// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Sent history (AC-90) and reminders (AC-91) against a fake terminumd (fake.mjs), at a wide and
// a phone size: Enter at a shell prompt tells what it sends before the key; the button under the
// split opens the last one as Markdown, ↑ asks for the one before; a reminder up makes the pale
// fuchsia button blink until it is pressed, its pane shows it as Markdown with Resolve; the
// dialog makes a reminder; the session list's strip is fuchsia for a session with one.

import fs from "node:fs";
import path from "node:path";
import { check, runningIn } from "./check.mjs";
import { drawn, group, item, line, openPage, pause, SIZES } from "./fake.mjs";

const DIST = path.join(import.meta.dirname, "../../ui/dist");   // the renderers, as terminumd serves them at /fb/
const SID = "tmux:local:terminum:%1";
const API = "/Users/alex/work/api";
const LAYOUT = [group("main", [item(SID, "api", { shell: true, sub: ["zsh", "~/work/api"], dir: API }), item("tmux:local:terminum:%2", "docs", { index: "2", dir: "/Users/alex/work/docs" }),
  item("tmux:local:terminum:%3", "api tests", { index: "3", dir: API })])];
const SCREEN = [line("out 1"), line("reminder text here"), { r: [["alex@mac api % git status", null, null, 0], [" ", null, null, 256]], e: true }];
const REMINDER = { id: "R1", sid: SID, key: SID, n: 1, text: "reminder text here", note: "**check** the retry", title: "api", host: null, at: 1792000000, remind: true, due: true };
const ITEM = { at: 1792000000, text: "fix the **tests**", agent: "claude", title: "api" };

export async function webSent(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device, layout: LAYOUT, screen: SCREEN, hook: (m, say) => {
      if (m.t === "sentget") say({ t: "sentone", sid: SID, n: m.n, total: 2, item: m.n === 0 ? ITEM : { ...ITEM, text: "ls -la", agent: undefined }, host: "local", path: "/Users/alex/work/api" });
    } });
    const { page, sent } = f;
    await page.route("**/fb/**", (route) => {
      const file = path.join(DIST, path.normalize(new URL(route.request().url()).pathname.slice(4)));
      return file.startsWith(DIST) && fs.existsSync(file) ? route.fulfill({ contentType: "text/javascript", body: fs.readFileSync(file) }) : route.fulfill({ status: 404, body: "" });
    });
    await drawn(page);
    await pause(page, 300);

    await page.focus("#kbd");
    const n = sent.length;
    await page.keyboard.press("Enter");
    await pause(page, 150);
    const after = sent.slice(n).map((m) => m.t);
    const told = sent.slice(n).find((m) => m.t === "sent");
    check("sent: Enter at a shell prompt tells the command after the prompt, before the key", told?.text === "git status" && told.agent === ""
      && after.indexOf("sent") < after.indexOf("in"), JSON.stringify(sent.slice(n)));

    const at = await page.evaluate((phone) => { const r = (id) => document.getElementById(id).getBoundingClientRect(), s = r("sentfab"), p = r(phone ? "splitfab" : "findfab");
      return { below: s.top >= p.bottom, aligned: Math.abs(s.left - p.left) < 1, shown: !document.getElementById("sentfab").hidden }; }, size !== "1400×900");
    check("sent: its button stands under the split (on a phone; under find on a computer)", at.shown && at.below && at.aligned, JSON.stringify(at));
    await page.click("#sentfab");
    await page.waitForFunction(() => document.querySelector("#spbody strong")?.textContent === "tests", null, { timeout: 5000 }).catch(() => {});
    const shown = await page.evaluate(() => ({ pos: document.getElementById("sppos").textContent, where: document.getElementById("spwhere").textContent,
      md: document.querySelector("#spbody strong")?.textContent, newer: document.getElementById("spnewer").disabled, inView: (() => { const r = document.getElementById("sentpane").getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth; })() }));
    check("sent: the last one as Markdown, where, 1 of 2; newer is off; the pane fits the view", shown.md === "tests" && shown.pos === "1 of 2"
      && shown.where.replace(/\u200e/g, "") === "/Users/alex/work/api" && shown.newer && shown.inView, JSON.stringify(shown));
    await page.click("#spolder");
    await page.waitForFunction(() => document.querySelector("#spbody code")?.textContent.includes("ls -la"), null, { timeout: 5000 }).catch(() => {});
    const older = await page.evaluate(() => ({ code: document.querySelector("#spbody code")?.textContent, pos: document.getElementById("sppos").textContent }));
    check("sent: ↑ the one before; a command as a shell block", older.code?.includes("ls -la") && older.pos === "2 of 2" && sent.some((m) => m.t === "sentget" && m.n === 1), JSON.stringify(older));
    await page.click("#spclose");

    // a reminder up: the button blinks until pressed; its pane: Markdown, Resolve
    f.say({ t: "marks", items: [REMINDER, { ...REMINDER, id: "B1", remind: false, due: false, note: "a bookmark" }] });
    await pause(page, 300);
    const up = await page.evaluate(() => { const b = document.getElementById("remindfab"); return { shown: !b.hidden, due: b.classList.contains("due"), n: document.getElementById("remindfabn").textContent }; });
    check("reminders: one up shows the button blinking, with its count (reminders only)", up.shown && up.due && up.n === "1", JSON.stringify(up));
    const look = await page.evaluate(() => { const g = (id) => document.getElementById(id), r = (id) => g(id).getBoundingClientRect(), cs = (id) => getComputedStyle(g(id));
      return { marks: !g("marksfab").hidden, under: r("remindfab").top > r("marksfab").bottom && Math.abs(r("remindfab").left - r("marksfab").left) < 1,
        size: r("remindfab").width === r("marksfab").width, shadow: cs("remindfab").boxShadow === cs("marksfab").boxShadow }; });
    check("reminders: the bell stands under the bookmarks' button, the colored ones alike: same size, same shadow, no glow",
      look.marks && look.under && look.size && look.shadow, JSON.stringify(look));
    await page.click("#remindfab");
    await page.waitForFunction(() => document.querySelector("#remindlist strong")?.textContent === "check", null, { timeout: 5000 }).catch(() => {});
    const pane = await page.evaluate(() => ({ md: document.querySelector("#remindlist strong")?.textContent, quote: document.querySelector("#remindlist blockquote")?.textContent.trim(),
      acts: [...document.querySelectorAll(".rmacts button")].map((b) => b.textContent), due: document.getElementById("remindfab").classList.contains("due") }));
    check("reminders: pressed, it blinks no more; its note as Markdown, its text quoted, Open, Close, Delete, Resolve", pane.md === "check" && pane.quote === "reminder text here"
      && pane.acts.join() === "Open,Close,Delete,Resolve" && !pane.due, JSON.stringify(pane));
    await page.click(".rmacts .primary");
    await pause(page, 100);
    check("reminders: Resolve tells terminumd", sent.some((m) => m.t === "remind" && m.id === "R1" && m.action === "resolve"));
    if (size === "1400×900") {
      const strip = await page.evaluate((id) => [...document.querySelectorAll(`#sessions .item[data-id="${id}"] .wave`)].map((w) => w.className), SID);
      check("reminders: a wavy orange and a wavy pink line in the session list, side by side", strip.join() === "wave mark,wave remind", strip.join());
      // the dialog: Reminder sends remind
      await page.evaluate(() => { const row = [...document.querySelectorAll("#term .ln")].find((el) => el.textContent.includes("out 1"));
        const r = document.createRange(); r.selectNodeContents(row); getSelection().removeAllRanges(); getSelection().addRange(r); });
      await page.waitForSelector("#markadd:not([hidden])", { timeout: 3000 }).catch(() => {});
      await page.click("#markadd");
      await page.waitForSelector("#markbox[open]");
      await page.click('#markkind [data-kind="reminder"]');
      const title = await page.textContent("#marktitle");
      await page.click('#markbox button[value="save"]');
      await pause(page, 150);
      const mark = sent.find((m) => m.t === "mark" && !m.id);
      check("reminders: the dialog's Reminder makes one", title === "Reminder" && mark?.remind === true, JSON.stringify({ title, mark }));
      f.say({ t: "marked", ref: mark?.ref, id: "N1" });       // the Mac kept it: the dialog closes
      await page.waitForFunction(() => !document.getElementById("markbox").open, null, { timeout: 3000 }).catch(() => {});

      // by project (AC-91): another session of the folder shows its bookmarks; another folder's
      // session shows none; with no session of its folder left a bookmark is orphaned, only below the list
      const b = (id, sid, dir, note, extra = {}) => ({ id, sid, key: sid, n: 1, text: `${note} text`, note, title: "t", host: null, at: 1792000000, dir, ...extra });
      f.say({ t: "marks", items: [b("P1", "tmux:local:terminum:%3", API, "from the tests session"), b("P2", "tmux:local:terminum:%2", "/Users/alex/work/docs", "docs only"),
        b("O1", "tmux:local:terminum:%9", "/Users/alex/gone", "orphan one"), b("R2", "tmux:local:terminum:%9", "/Users/alex/gone", "orphan reminder", { remind: true })] });
      await pause(page, 300);
      await page.click("#marksfab");
      await pause(page, 200);
      const pane = await page.evaluate(() => [...document.querySelectorAll("#markpane .mkitem .note")].map((n) => n.textContent));
      await page.keyboard.press("Escape");
      await page.click("#marksbtn");
      await pause(page, 200);
      const list = await page.evaluate(() => [...document.querySelectorAll("#marklist .mkitem")].map((i) => `${i.querySelector(".note").textContent}${i.querySelector(".closed") ? ":" + i.querySelector(".closed").textContent : ""}`));
      await page.click('#marklist .mkkind button:text-is("Reminders")');
      const rem = await page.evaluate(() => [...document.querySelectorAll("#marklist .mkitem .note")].map((n) => n.textContent));
      await page.click("#marklist .mkall");
      await page.waitForFunction(() => document.getElementById("markdoc").open && document.querySelector("#markdocbody h3"), null, { timeout: 5000 }).catch(() => {});
      const docOpen = await page.evaluate(() => ({ open: document.getElementById("markdoc").open, h: [...document.querySelectorAll("#markdocbody h3")].map((h) => h.textContent),
        full: document.getElementById("markdoc").getBoundingClientRect().height > innerHeight * 0.8 }));
      await page.keyboard.press("Escape");
      check("projects: a session shows its folder's bookmarks from every session of it, none of another folder",
        pane.join() === "from the tests session", pane.join());
      check("projects: the list holds them all, one with no session of its folder left as orphaned; Reminders lists the reminders",
        list.includes("orphan one:orphaned") && list.includes("docs only") && !list.includes("orphan reminder") && rem.join() === "orphan reminder", JSON.stringify({ list, rem }));
      check("projects: Open all shows them as one document, full screen", docOpen.open && docOpen.h.join() === "orphan reminder" && docOpen.full, JSON.stringify(docOpen));
    }
    await f.context.close();
  }
  // a split's embedded page shows its shell alone: no floating buttons
  runningIn(`${name} 1400×900`);
  const e = await openPage(browser, { layout: LAYOUT, screen: SCREEN, url: `/?embed=${encodeURIComponent(SID)}` });
  await drawn(e.page);
  e.say({ t: "marks", items: [{ ...REMINDER, sid: SID }] });
  await pause(e.page, 300);
  const fabs = await e.page.evaluate(() => ["findfab", "splitfab", "sentfab", "marksfab", "remindfab"]
    .filter((id) => { const b = document.getElementById(id); return getComputedStyle(b).display !== "none" && b.getBoundingClientRect().width > 0; }));
  check("a split's embedded page shows none of the floating buttons", !fabs.length, fabs.join());
  await e.context.close();
}
