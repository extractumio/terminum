// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A box to read or type in, over the whole window (terminum.md AC-184), against a fake terminumd (fake.mjs): every
// such box of the board and of the session page has its key; pressed, the box covers the window (on a phone what
// is in sight), its dialog still open under it; Esc or the key gives it back, what was typed kept, the focus in it;
// a second Esc then closes the dialog as before; a question opened over it takes Esc for itself. The task window's boxes: e2e_taskwin.mjs.

import { check, runningIn } from "./check.mjs";
import { drawn, openPage, SIZES } from "./fake.mjs";

const BOARD = ["newtext", "newshown", "schedtext", "schedshown", "schedprompt", "projecttemplate", "runmessage", "runprompt"];
const SESSION = ["pastetext", "askq", "asktext", "marknote", "marktext", "mbtext"];

/** Which of `ids` has no key of its own box. */
const keyless = (page, ids) => page.evaluate(async (ids) => {
  (await import("/taskboxes.js")).taskBoxes();
  return ids.filter((id) => !document.getElementById(id).closest(".xbox")?.querySelector(":scope > .xkey"));
}, ids);

/** Opens `dialog`, fills `field`, presses its key: where the box stands, and what is open. */
const pressKey = (page, dialog, field, text) => page.evaluate(async ([dialog, field, text]) => {
  for (const d of document.querySelectorAll("dialog[open]")) d.close();
  const d = document.getElementById(dialog), f = document.getElementById(field);
  d.querySelector("details:has(#" + field + ")")?.setAttribute("open", "");
  d.showModal();
  if ("value" in f && f.tagName !== "PRE") f.value = text; else f.textContent = text;
  await new Promise((r) => setTimeout(r, 30));
  f.closest(".xbox").querySelector(".xkey").click();
  await new Promise((r) => setTimeout(r, 30));
  const r = f.closest(".xbox").getBoundingClientRect(), key = f.closest(".xbox").querySelector(".xkey");
  const at = [[1, 1], [innerWidth - 2, 1], [1, innerHeight - 2], [innerWidth - 2, innerHeight - 2], [innerWidth / 2, innerHeight / 2]];
  return { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height), win: { w: innerWidth, h: innerHeight },
    over: at.every(([x, y]) => f.closest(".xbox").contains(document.elementFromPoint(x, y))), open: d.open, pressed: key.getAttribute("aria-pressed"), focus: document.activeElement?.id };
}, [dialog, field, text]);

const state = (page, dialog, field) => page.evaluate(([dialog, field]) => {
  const f = document.getElementById(field);
  return { full: f.closest(".xbox").classList.contains("full"), open: document.getElementById(dialog).open, value: f.value ?? f.textContent, focus: document.activeElement?.id };
}, [dialog, field]);

const covers = (b) => b.x === 0 && b.y === 0 && Math.abs(b.w - b.win.w) <= 1 && Math.abs(b.h - b.win.h) <= 1 && b.over;

export async function webExpand(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const b = await openPage(browser, { device, url: "/kanban.html" });
    // the fake signs nothing in, so the board would leave for /sessions/: it stays, its windows are what is checked
    await b.page.route("**/sessions/**", (r) => r.abort());
    await b.page.waitForTimeout(300);
    check(`AC-184 every box of the board to read or type in has its key (${size})`, !(await keyless(b.page, BOARD)).length, (await keyless(b.page, BOARD)).join());
    const long = "a line of what the agent gets, long enough to wrap where the box is narrow\n".repeat(80);
    const shown = await pressKey(b.page, "schedbox", "schedprompt", long);
    check(`AC-184 the schedule's What the agent gets, its key pressed, covers the window, its dialog open under it (${size})`,
      covers(shown) && shown.open && shown.pressed === "true", JSON.stringify(shown));
    await b.page.keyboard.press("Escape");
    const back = await state(b.page, "schedbox", "schedprompt");
    check(`AC-184 Esc gives it back to its dialog, which stays open (${size})`, !back.full && back.open, JSON.stringify(back));
    const typed = await pressKey(b.page, "newbox", "newtext", "");
    await b.page.keyboard.type("typed over the whole window");
    const kept = await state(b.page, "newbox", "newtext");
    await b.page.click("#newtext ~ .xkey");
    const after = await state(b.page, "newbox", "newtext");
    check(`AC-184 the new task's text: typed over the whole window, its key gives it back with the words and the focus (${size})`,
      covers(typed) && typed.focus === "newtext" && kept.full && !after.full && after.open && after.value === "typed over the whole window" && after.focus === "newtext",
      JSON.stringify({ typed, kept, after }));
    const run = await pressKey(b.page, "runbox", "runprompt", long);
    check(`AC-184 RUN's What will be sent covers the window, past its own 30% (${size})`, covers(run), JSON.stringify(run));
    check("no page errors", !b.errors.length, b.errors.join(" | "));
    await b.close();
    const s = await openPage(browser, { device });
    await drawn(s.page);
    check(`AC-184 Paste, Ask's question and quote, a bookmark's note and quote, the bookmark bar's quote have their key (${size})`, !(await keyless(s.page, SESSION)).length, (await keyless(s.page, SESSION)).join());
    const ask = await pressKey(s.page, "askbox", "askq", "why");
    check(`AC-184 Ask's question covers the window, the focus in it (${size})`, covers(ask) && ask.open && ask.focus === "askq", JSON.stringify(ask));
    // a question opened over the box takes Esc for itself; the box stays over the window
    await s.page.evaluate(() => document.getElementById("confirmbox").showModal());
    await s.page.keyboard.press("Escape");
    const asked = await state(s.page, "askbox", "askq");
    check(`AC-184 a question over a full box: Esc answers it, the box stays (${size})`, asked.full && asked.open && !(await s.page.evaluate(() => document.getElementById("confirmbox").open)), JSON.stringify(asked));
    await s.page.keyboard.press("Escape");
    await s.page.keyboard.press("Escape");
    check(`AC-184 then Esc gives the box back, and a second Esc closes its dialog as before (${size})`, !(await state(s.page, "askbox", "askq")).open);
    const quote = await pressKey(s.page, "askbox", "asktext", "a selection quoted\n".repeat(60));
    check(`AC-184 Ask's quote covers the window (${size})`, covers(quote), JSON.stringify(quote));
    // the bookmark bar's quote stands in the page, not in a dialog
    const bar = await s.page.evaluate(async () => {
      for (const d of document.querySelectorAll("dialog[open]")) d.close();
      const t = document.getElementById("mbtext"), bar = document.getElementById("markbar");
      bar.hidden = false; t.hidden = false; t.textContent = "a bookmarked line\n".repeat(80);
      t.closest(".xbox").querySelector(".xkey").click();
      await new Promise((r) => setTimeout(r, 30));
      const r = t.closest(".xbox").getBoundingClientRect();
      return { x: r.x, y: r.y, w: Math.round(r.width) - innerWidth, h: Math.round(r.height) - innerHeight,
        over: [[1, 1], [innerWidth - 2, innerHeight - 2], [innerWidth / 2, innerHeight / 2]].every(([x, y]) => t.closest(".xbox").contains(document.elementFromPoint(x, y))) };
    });
    check(`AC-184 the bookmark bar's quote covers the window over the terminal and its buttons (${size})`, !bar.x && !bar.y && !bar.w && !bar.h && bar.over, JSON.stringify(bar));
    check("no page errors", !s.errors.length, s.errors.join(" | "));
    await s.close();
  }
}
