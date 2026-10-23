// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Ask's dialog (AC-37), run by e2e_marks.mjs on its page: Add to Chat's numbered, compact quote in a shell
// and in a coding agent's box, Send to Agent, Esc, a narrow window.

import { check } from "./check.mjs";

/** page: e2e_marks's page; sent: what it sent terminumd; say(msg): from terminumd; layout: its
 *  list; selectRow(page, text): selects a row's text. */
export async function askChecks({ page, sent, say, at, layout, selectRow }) {
  // Ask (AC-37): the dialog shows the selection; Add to Chat is off until a question is typed; the
  // quote numbered by terminumd and the question pasted at the cursor, compact, no Enter; Esc pastes nothing
  await page.click("#bottom").catch(() => {});
  await selectRow(page, "out 2500 lorem");
  await page.waitForSelector("#askfab:not([hidden])", { timeout: 3000 }).catch(() => {});
  await page.click("#askfab");
  await page.waitForSelector("#askbox[open]", { timeout: 3000 }).catch(() => {});
  const dialog = await page.evaluate(() => ({ text: document.getElementById("asktext").textContent, off: document.querySelector("#askbox .primary").disabled, noSend: document.getElementById("asksend").hidden,
    focused: document.activeElement?.id, gone: document.getElementById("askfab").hidden }));
  const placed = await page.evaluate(() => { const box = document.getElementById("askbox"), b = box.getBoundingClientRect(), t = document.getElementById("term").getBoundingClientRect();
    return { width: box.offsetWidth, dx: Math.round(b.left + b.width / 2 - (t.left + t.width / 2)), dy: Math.round(b.top + b.height / 2 - (t.top + t.height / 2)), termLeft: Math.round(t.left) }; });
  check(`${at} Ask: its dialog 600 px wide (a quarter larger), centered on the terminal, not on the window`,
    placed.width === 600 && Math.abs(placed.dx) <= 1 && Math.abs(placed.dy) <= 1 && placed.termLeft > 0, JSON.stringify(placed));
  await page.fill("#askq", "why does it never reset?");
  let k = sent.length;
  await page.click('#askbox button[value="add"]');
  await page.waitForFunction(() => !document.getElementById("askbox").open, null, { timeout: 5000 }).catch(() => {});
  const asked = sent.slice(k), quoted = asked.filter((x) => x.t === "in").map((x) => x.data).join("");
  check(`${at} Ask: a dialog with the selection, Add to Chat off until a question, no Send to Agent in a shell; the quote numbered by terminumd on its first line and the question pasted, no Enter`,
    dialog.text.includes("out 2500 lorem ipsum") && dialog.off && dialog.noSend && dialog.focused === "askq" && dialog.gone && asked.some((x) => x.t === "askno")
    && quoted.startsWith("\x1b[200~Quote #0007: «out 2500 lorem ipsum") && quoted.endsWith("»\rwhy does it never reset?\r\x1b[201~"),
    JSON.stringify({ dialog, quoted }));
  // the quote stays marked: terminumd keeps it with its number and the line its selection began on,
  // and its answer puts a light blue wavy line (the Ask button's) down the quote's rows' left, as a
  // reminder's quote has; no underline
  const keep = asked.find((x) => x.t === "asked");
  say({ t: "askedmarks", sid: "S1", items: keep ? [{ n: keep.n, text: keep.text, line: keep.line }] : [] });
  // the paste brought the view to the input: the quote is marked once it is in sight again
  await page.evaluate(() => [...document.querySelectorAll("#term .ln")].find((el) => el.textContent.includes("out 2500 lorem"))?.scrollIntoView({ block: "center" }));
  await page.waitForFunction(() => [...document.querySelectorAll("#term .askline")].some((l) => !l.hidden), null, { timeout: 3000 }).catch(() => {});
  const marked = await page.evaluate(() => {
    const lines = [...document.querySelectorAll("#term .askline")].filter((l) => !l.hidden);
    const row = [...document.querySelectorAll("#term .ln")].find((el) => el.textContent.includes("out 2500 lorem"))?.getBoundingClientRect();
    const term = document.getElementById("term").getBoundingClientRect(), text = document.querySelector("#term .ln span")?.getBoundingClientRect();
    const l = lines[0]?.getBoundingClientRect(), cs = lines[0] && getComputedStyle(lines[0]);
    const ask = getComputedStyle(document.getElementById("askfab")).backgroundColor;
    return l && { shown: lines.length, covers: Math.abs(l.top - row.top) <= 1 && Math.abs(l.bottom - row.bottom) <= 1, gutter: l.left >= term.left && l.right <= (text?.left ?? Infinity),
      color: cs.backgroundColor === ask, wavy: /url\(/.test(cs.maskImage || cs.webkitMaskImage), under: !!globalThis.CSS?.highlights?.get("asked") };
  });
  check(`${at} Ask: the quote is kept with its number and line, and keeps a light blue wavy line down its left where it was selected`,
    keep?.n === 7 && Number.isInteger(keep.line) && keep.text.includes("out 2500 lorem ipsum") && marked?.shown === 1 && marked.covers && marked.gutter && marked.color && marked.wavy && !marked.under,
    JSON.stringify({ keep, marked }));
  // the quote as sent (Claude Code shows the message after it) is never marked, even where it stands
  // nearest the line kept: the agent reprinted its answer and the original moved away
  const row = (text) => ({ r: [[text, null, null, 0]], e: true });
  say({ t: "screen", sid: "S1", full: false, n: 2, cols: 80, rows: 2, ch: [[0, row(`❯ Quote #0008: «${keep?.text}»`)], [1, row("  and why?")]] });
  say({ t: "askedmarks", sid: "S1", items: [{ n: 8, text: keep?.text, line: 3000 }] });
  await page.click("#bottom").catch(() => {});
  await page.waitForTimeout(400);
  const copy = await page.evaluate(() => ({ shown: [...document.querySelectorAll("#term .askline")].filter((l) => !l.hidden).length,
    there: [...document.querySelectorAll("#screen .ln")].some((el) => el.textContent.includes("«out 2500 lorem")) }));
  check(`${at} Ask: the quote as sent, in the message after it, never gets the wavy line`, copy.there && copy.shown === 0, JSON.stringify(copy));
  say({ t: "askedmarks", sid: "S1", items: [] });
  // in a coding agent's box the new line after it is typed (Ctrl+J), none before
  say({ t: "layout", groups: [{ ...layout[0], items: [{ ...layout[0].items[0], agent: "claude", shell: false }, layout[0].items[1]] }] });
  await page.waitForTimeout(150);
  await selectRow(page, "out 2500 lorem");
  await page.click("#askfab");
  await page.fill("#askq", "and now?");
  k = sent.length;
  await page.click('#askbox button[value="add"]');
  await page.waitForFunction(() => !document.getElementById("askbox").open, null, { timeout: 5000 }).catch(() => {});
  const typed = sent.slice(k).filter((x) => x.t === "in").map((x) => x.data);
  check(`${at} Ask in a coding agent's box: the paste, then a Ctrl+J (a new line after it, none before; nothing sent)`,
    typed.length === 2 && typed[1] === "\n" && typed[0].startsWith("\x1b[200~Quote #0007: «") && typed[0].endsWith("»\rand now?\x1b[201~"), JSON.stringify(typed));
  // Send to Agent (a coding agent's pane only): the same paste, then Enter a moment after
  await selectRow(page, "out 2500 lorem");
  await page.click("#askfab");
  await page.fill("#askq", "send it");
  k = sent.length;
  await page.click("#asksend");
  await page.waitForFunction(() => !document.getElementById("askbox").open, null, { timeout: 5000 }).catch(() => {});
  await page.waitForTimeout(400);
  const now = sent.slice(k).filter((x) => x.t === "in");
  check(`${at} Ask's Send to Agent in a coding agent's pane: the paste, then Enter`,
    now.length === 2 && now[0].data.startsWith("\x1b[200~Quote #0007: «") && now[0].data.endsWith("»\rsend it\x1b[201~") && now[1].data === "\r", JSON.stringify(now.map((x) => x.data)));
  say({ t: "layout", groups: layout });
  await page.waitForTimeout(150);
  await selectRow(page, "out 2500 lorem");
  await page.click("#askfab");
  await page.fill("#askq", "never mind");
  k = sent.length;
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  check(`${at} Ask: Esc closes it and pastes nothing`, !await page.evaluate(() => document.getElementById("askbox").open) && !sent.slice(k).some((x) => x.t === "in" || x.t === "askno"));
  // a narrow window: Ask under the [+] there too (the column's buttons give way)
  await page.setViewportSize({ width: 800, height: 900 });
  await page.waitForTimeout(300);
  await selectRow(page, "out 2500 lorem");
  await page.waitForTimeout(200);
  const narrow = await page.evaluate(() => { const r = (id) => document.getElementById(id).getBoundingClientRect();
    return { above: r("askfab").top >= r("markadd").bottom && r("askfab").top - r("markadd").bottom < 16, sameX: Math.abs(r("askfab").left - r("markadd").left) < 1,
      shown: !document.getElementById("askfab").hidden, sent: getComputedStyle(document.getElementById("sentfab")).visibility }; });
  check(`${at} Ask on a narrow window: just under the [+]`, narrow.shown && narrow.above && narrow.sameX && narrow.sent === "hidden", JSON.stringify(narrow));
  await page.evaluate(() => getSelection().removeAllRanges());
  await page.setViewportSize({ width: 1400, height: 900 });
}
