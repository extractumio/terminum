// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Ask's dialog (AC-37), run by e2e_marks.mjs on its page: Add to Chat's numbered quote in a shell
// and in a coding agent's box, Send to Agent, Esc, a narrow window.

import { check } from "./check.mjs";

/** page: e2e_marks's page; sent: what it sent terminumd; say(msg): from terminumd; layout: its
 *  list; selectRow(page, text): selects a row's text. */
export async function askChecks({ page, sent, say, at, layout, selectRow }) {
  // Ask (AC-37): the dialog shows the selection; Add to Chat is off until a question is typed; the
  // quote numbered by terminumd, the question and a rule pasted at the cursor, no Enter; Esc pastes nothing
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
  check(`${at} Ask: a dialog with the selection, Add to Chat off until a question, no Send to Agent in a shell; the quote numbered by terminumd, the question and a rule pasted, no Enter`,
    dialog.text.includes("out 2500 lorem ipsum") && dialog.off && dialog.noSend && dialog.focused === "askq" && dialog.gone && asked.some((x) => x.t === "askno")
    && quoted.startsWith("\x1b[200~Quote #0007:\r - Quote Start ----\r«out 2500 lorem ipsum") && quoted.endsWith("»\r- Quote End ----\rwhy does it never reset?\r----\r\r\x1b[201~"),
    JSON.stringify({ dialog, quoted }));
  // in a coding agent's box the blank line after it is typed (Ctrl+J), none before
  say({ t: "layout", groups: [{ ...layout[0], items: [{ ...layout[0].items[0], agent: "claude", shell: false }, layout[0].items[1]] }] });
  await page.waitForTimeout(150);
  await selectRow(page, "out 2500 lorem");
  await page.click("#askfab");
  await page.fill("#askq", "and now?");
  k = sent.length;
  await page.click('#askbox button[value="add"]');
  await page.waitForFunction(() => !document.getElementById("askbox").open, null, { timeout: 5000 }).catch(() => {});
  const typed = sent.slice(k).filter((x) => x.t === "in").map((x) => x.data);
  check(`${at} Ask in a coding agent's box: the paste, then two Ctrl+J (a blank line after it, none before; nothing sent)`,
    typed.length === 2 && typed[1] === "\n\n" && typed[0].startsWith("\x1b[200~Quote #0007:\r") && typed[0].endsWith("and now?\r----\x1b[201~"), JSON.stringify(typed));
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
    now.length === 2 && now[0].data.startsWith("\x1b[200~Quote #0007:\r") && now[0].data.endsWith("send it\r----\x1b[201~") && now[1].data === "\r", JSON.stringify(now.map((x) => x.data)));
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
