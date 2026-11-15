// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-32 against a real private terminumd (e2e_live.mjs): a coding agent's tool output that
// scrolled off whole folds while the next tool's ⏺ is still on the screen (the history's scan
// reads the screen after it), not only once that scrolled off too.
import fs from "node:fs";
import path from "node:path";
import { check, shownPane, until } from "./check.mjs";

export async function liveFolds(page, { tmux, work }) {
  const pane = await shownPane(page, { tmux, work });
  const rows = Number(tmux("display", "-p", "-t", pane, "#{pane_height}"));
  // a tool's call and output, blank rows, then the next call whose output fills the screen but a few
  // rows (the shell's prompt takes some): the screen's top falls among the blank rows
  const lines = ["⏺ Bash(grep -n x a.rs)", "  ⎿  first: line", ...Array.from({ length: 12 }, (_, i) => `     ${i}: line`), "", "", "", "", "",
    "⏺ Bash(sed -n 1,9p b.rs)", "  ⎿  next", ...Array.from({ length: rows - 7 }, (_, i) => `     more ${i}`)];
  const file = path.join(work, "agent.txt");
  fs.writeFileSync(file, lines.join("\n") + "\n");
  // the folds of this label already there (another browser's run in the same pane): only a new one counts
  const folds = () => [...document.querySelectorAll("#hist .ln.wg-fold")]
    .filter((h) => h.shadowRoot?.querySelector('[data-act="fold"]')?.textContent === "⋯ 10 more lines of output").length;
  const before = await page.evaluate(folds);
  const more = (n) => [...document.querySelectorAll("#hist .ln.wg-fold")]
    .filter((h) => h.shadowRoot?.querySelector('[data-act="fold"]')?.textContent === "⋯ 10 more lines of output").length > n;
  await page.click("#term");
  await page.keyboard.type(`clear; cat ${file}`);
  await page.keyboard.press("Enter");
  // more 0 is only the start: tmux can capture it before the rest scrolls the first tool off.
  const printed = await until(() => tmux("capture-pane", "-p", "-t", pane).split("\n")
    .some((line) => line.trim() === `more ${rows - 8}`), 5000);
  check("AC-32 the tool fixture printed its final output row", printed);
  const label = await page.waitForFunction(more, before, { timeout: 5000 })
    .then(() => "⋯ 10 more lines of output", () => null);
  const screen = tmux("capture-pane", "-p", "-t", pane);   // current after the fold wait, never an earlier partial screen
  const next = screen.includes("⏺ Bash(sed -n 1,9p b.rs)") && !screen.includes("11: line");
  check("AC-32 a tool's output that scrolled off folds while the next tool's ⏺ is still on the screen", label !== null && next, JSON.stringify({ label, next, rows, top: screen.split("\n").slice(0, 4) }));
  fs.rmSync(file, { force: true });
}
