// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Switching panes against a fake terminumd (fake.mjs), on a wide page: the pane shown before stays
// until the new one's first frame and the new one is drawn at once, at the chosen text size
// (terminum.md AC-04); "sub" carries the view's grid; docked Files is each session's (AC-70).

import { check, runningIn } from "./check.mjs";
import { drawn, line, openPage, pause, THEME, WIDE } from "./fake.mjs";

const A = "tmux:local:terminum:%1", B = "tmux:local:terminum:%2";

export async function webSwitch(browser, name) {
  runningIn(`${name} 1400×900`);
  let delay = 0;
  const onSub = (msg, say) => {
    const mark = msg.id === A ? "pane-A" : "pane-B";
    const send = () => {
      say({ t: "theme", sid: msg.id, theme: THEME });
      say({ t: "hist", mode: "reset", sid: msg.id, first: 0, oldest: 0, truncated: false, lines: [] });
      say({ t: "screen", sid: msg.id, full: true, n: 3, cols: msg.cols ?? 80, rows: 3, alt: false, ch: [[0, line(`$ ${mark}`)], [1, line("ok")], [2, line("$ ")]] });
    };
    if (delay) setTimeout(send, delay); else send();
    return true;
  };
  const f = await openPage(browser, { device: WIDE, onSub });
  const { page, sent } = f;
  await drawn(page);
  const text = () => page.evaluate(() => document.getElementById("screen").textContent);
  const px = () => page.evaluate(() => getComputedStyle(document.getElementById("term")).fontSize);
  const docked = () => page.evaluate(() => document.body.classList.contains("docked"));
  const pick = async (id) => { await page.click(`#sessions .item[data-id='${id}']`); };
  if (!(await text()).includes("pane-A")) { await pick(A); await pause(page, 300); }

  // B's first frame comes 500 ms after its "sub": until then A stays, still; then B at once
  delay = 500;
  const size0 = await px();
  await pick(B);
  await pause(page, 200);
  const during = await text();
  await pause(page, 600);
  const after = await text(), size1 = await px();
  const sub = sent.filter((m) => m.t === "sub").at(-1);
  check("AC-04 another pane opened: the one shown stays, not blank, until the new one's first frame, which is drawn at once",
    during.includes("pane-A") && !during.includes("pane-B") && after.includes("pane-B") && !after.includes("pane-A"), JSON.stringify({ during, after }));
  check("AC-04 the text size does not change on the way, and \"sub\" carries this view's grid",
    size0 === size1 && sub.id === B && sub.cols > 45 && sub.rows > 14, JSON.stringify({ size0, size1, sub }));
  delay = 0;

  // docked Files: closed in B, still open in A; each session as it last had it
  const inB = await docked();
  await page.keyboard.press("Meta+KeyB");
  await pause(page, 150);
  const closedB = await docked();
  await pick(A);
  await pause(page, 300);
  const inA = await docked(), subA = sent.filter((m) => m.t === "sub").at(-1), from = sent.length;
  await pick(B);
  await pause(page, 700);
  const backB = await docked(), subB = sent.filter((m) => m.t === "sub").at(-1);
  const later = sent.slice(from).filter((m) => m.t === "size").map((m) => m.cols);
  const kept = await page.evaluate(() => JSON.parse(localStorage.getItem("filesDockedBy") ?? "{}"));
  check("AC-70 docked Files is each session's: closed in one, still open in another, as it was when back",
    inB && !closedB && inA && !backB && kept[B] === false && kept[A] === undefined, JSON.stringify({ inB, closedB, inA, backB, kept }));
  check("AC-70 \"sub\" carries the width the session's own Files leave, and no other width follows it",
    subB.cols > subA.cols && later.every((c) => c === subB.cols), JSON.stringify({ a: subA.cols, b: subB.cols, later }));
  check("AC-04 no page errors (switch)", !f.errors.length, f.errors.join(" | "));
  await f.close();
}
