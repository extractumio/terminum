// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-32 on the screen (e2e_widgets.mjs' fake bridge): a coding agent's tool output still on the
// screen folds as terminumd says, one line with Show and Copy; Show keeps it open while the
// screen changes elsewhere; a fold the next screen drops shows its rows again.

import { check, runningIn } from "./check.mjs";
import { open } from "./e2e_widgets.mjs";

const line = (text) => ({ r: [[text, null, null, 0]], e: true });
const OUT = Array.from({ length: 10 }, (_, i) => `     line ${i + 1}`);
const ROWS = ["⏺ Bash(ls -l)", "  ⎿  total 12", "     a", ...OUT, "", "⏺ Done.", "> "];

export async function webScreenFolds(browser, name) {
  runningIn(`${name} 1200×800`);
  const f = await open(browser, { agent: true, screen: ROWS, history: [] });
  const { page, say } = f;
  const frame = (folds, last = "> ") => say({ t: "screen", sid: "S1", full: true, n: ROWS.length, cols: 80, rows: ROWS.length,
    ch: [...ROWS.slice(0, -1), last].map((t, i) => [i, line(t)]), folds });
  const state = () => page.evaluate(() => {
    const rows = [...document.querySelectorAll("#screen .ln")];
    const host = rows.find((r) => r.classList.contains("wg-fold") || r.shadowRoot?.querySelector(".fold"));
    return { hidden: rows.filter((r) => r.classList.contains("wg-hid")).length, label: host?.shadowRoot?.querySelector('[data-act="fold"]')?.textContent ?? null };
  });
  frame([[3, 12, "⋯ 10 lines of output"]]);
  await page.waitForTimeout(300);
  const folded = await state();
  check("AC-32 a tool's output on the screen is one line, its rows hidden", folded.label === "⋯ 10 lines of output" && folded.hidden === 9, JSON.stringify(folded));
  await page.evaluate(() => [...document.querySelectorAll("#screen .ln")].find((r) => r.shadowRoot?.querySelector(".fold"))
    .shadowRoot.querySelector('[data-act="fold"]').click());
  await page.waitForTimeout(200);
  frame([[3, 12, "⋯ 10 lines of output"]], "> typing");
  await page.waitForTimeout(300);
  const opened = await state();
  check("AC-32 Show opens it, and it stays open as the screen changes below it", opened.hidden === 0 && /^Hide/.test(opened.label ?? ""), JSON.stringify(opened));
  frame([]);
  await page.waitForTimeout(300);
  const gone = await state();
  check("AC-32 a fold the screen no longer has leaves its rows shown", gone.hidden === 0 && gone.label === null, JSON.stringify(gone));
  check("AC-32 no page errors", !f.errors.length, f.errors.join(" | "));
  await page.close();
}
