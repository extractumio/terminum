// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A box to read or type in, over the whole window (terminum.md AC-184), against a fake terminumd (fake.mjs): every
// such box of the board and of the session page has its key; pressed, the box covers the window (on a phone what
// is in sight), its dialog still open under it; Esc or the key gives it back, what was typed kept, the focus in it;
// a second Esc then closes the dialog as before; a question opened over it takes Esc for itself. The task window's
// boxes: e2e_taskwin.mjs; the schedules' page: e2e_liveschedules.mjs.

import { check, coversWindow, runningIn } from "./check.mjs";
import { drawn, openPage, SIZES } from "./fake.mjs";

// the text boxes with no key (AC-184): the terminal's own input, a project's folder (a path), a session's last lines
// (a window that is itself a document)
const KEYLESS = ["kbd", "projectdir", "lastlinestext"];
const FULL = ".xbox.full";

/** The text fields, previews and quotes of the page with no key of their own box, but those meant to have none. */
const unkeyed = (page) => page.evaluate(async (none) => {
  (await import("/taskboxes.js")).taskBoxes();
  return [...document.querySelectorAll("textarea, pre, .markdown[data-grow], #asktext, #marktext, #mbtext")]
    .filter((f) => !none.includes(f.id) && !f.closest(".xbox")?.lastElementChild.matches(".xkey")).map((f) => f.id || f.className);
}, KEYLESS);

/** Opens `dialog` alone, `text` in `field`, and presses the field's key. */
const pressKey = (page, dialog, field, text) => page.evaluate(([dialog, field, text]) => {
  for (const d of document.querySelectorAll("dialog[open]")) d.close();
  const d = document.getElementById(dialog), f = document.getElementById(field);
  d.querySelector(`details:has(#${field})`)?.setAttribute("open", "");
  d.showModal();
  if (f.tagName === "TEXTAREA") f.value = text; else f.textContent = text;
  f.closest(".xbox").querySelector(".xkey").click();
}, [dialog, field, text]);

const state = (page, dialog, field) => page.evaluate(([dialog, field]) => {
  const f = document.getElementById(field), key = f.closest(".xbox").querySelector(".xkey");
  return { full: f.closest(".xbox").classList.contains("full"), pressed: key.getAttribute("aria-pressed"), open: document.getElementById(dialog).open,
    value: f.value ?? f.textContent, focus: document.activeElement?.id };
}, [dialog, field]);

const LONG = "a line of what the agent gets, long enough to wrap where the box is narrow\n".repeat(80);

export async function webExpand(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const b = await openPage(browser, { device, url: "/kanban.html" });
    const missing = await unkeyed(b.page);
    check(`AC-184 every box of the board to read or type in has its key (${size})`, !missing.length, missing.join());
    await pressKey(b.page, "schedbox", "schedprompt", LONG);
    const shown = await coversWindow(b.page, FULL), held = await state(b.page, "schedbox", "schedprompt");
    check(`AC-184 the schedule's What the agent gets, its key pressed, covers the window, its dialog open under it (${size})`,
      shown.covers && held.open && held.pressed === "true" && held.focus === "schedprompt", JSON.stringify({ shown, held }));
    await b.page.keyboard.press("Escape");
    const back = await state(b.page, "schedbox", "schedprompt");
    check(`AC-184 Esc gives it back to its dialog, which stays open (${size})`, !back.full && back.open && back.pressed === "false", JSON.stringify(back));
    await pressKey(b.page, "newbox", "newtext", "");
    const typing = await coversWindow(b.page, FULL);
    await b.page.keyboard.type("typed over the whole window");
    await b.page.click("#newtext ~ .xkey");
    const after = await state(b.page, "newbox", "newtext");
    check(`AC-184 the new task's text: typed over the whole window, its key gives it back with the words and the focus (${size})`,
      typing.covers && !after.full && after.open && after.value === "typed over the whole window" && after.focus === "newtext", JSON.stringify({ typing, after }));
    await pressKey(b.page, "runbox", "runprompt", LONG);
    const run = await coversWindow(b.page, FULL);
    check(`AC-184 RUN's What will be sent covers the window, past its own 30% (${size})`, run.covers, JSON.stringify(run));
    check("no page errors", !b.errors.length, b.errors.join(" | "));
    await b.close();

    const s = await openPage(browser, { device });
    await drawn(s.page);
    const lost = await unkeyed(s.page);
    check(`AC-184 every box of the session page to read or type in has its key (${size})`, !lost.length, lost.join());
    await pressKey(s.page, "askbox", "askq", "why");
    const ask = await coversWindow(s.page, FULL), asking = await state(s.page, "askbox", "askq");
    check(`AC-184 Ask's question covers the window, the focus in it (${size})`, ask.covers && asking.open && asking.focus === "askq", JSON.stringify({ ask, asking }));
    await s.page.evaluate(() => document.getElementById("confirmbox").showModal());
    await s.page.keyboard.press("Escape");
    const asked = await state(s.page, "askbox", "askq");
    check(`AC-184 a question over a full box: Esc answers it, the box stays (${size})`, asked.full && asked.open && !(await s.page.evaluate(() => document.getElementById("confirmbox").open)), JSON.stringify(asked));
    await s.page.keyboard.press("Escape");
    await s.page.keyboard.press("Escape");
    check(`AC-184 then Esc gives the box back, and a second Esc closes its dialog as before (${size})`, !(await state(s.page, "askbox", "askq")).open);
    await pressKey(s.page, "askbox", "asktext", "a selection quoted\n".repeat(60));
    const quote = await coversWindow(s.page, FULL);
    check(`AC-184 Ask's quote covers the window (${size})`, quote.covers, JSON.stringify(quote));
    // the bookmark bar's quote stands in the page, not in a dialog
    await s.page.evaluate(() => {
      for (const d of document.querySelectorAll("dialog[open]")) d.close();
      const t = document.getElementById("mbtext");
      document.getElementById("markbar").hidden = t.hidden = false;
      t.textContent = "a bookmarked line\n".repeat(80);
      t.closest(".xbox").querySelector(".xkey").click();
    });
    const bar = await coversWindow(s.page, FULL);
    check(`AC-184 the bookmark bar's quote covers the window over the terminal and its buttons (${size})`, bar.covers, JSON.stringify(bar));
    check("no page errors", !s.errors.length, s.errors.join(" | "));
    await s.close();
  }
}
