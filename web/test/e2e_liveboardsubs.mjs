// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// An agent's subscription stripes on the board, against a real private terminumd (terminum-dispatcher.md AC-167):
// not known until Claude Code's status line, through the real `terminum statusline` in a pane terminumd lists, says them.
import { execFileSync } from "node:child_process";
import path from "node:path";
import { check, until } from "./check.mjs";

/** terminum-dispatcher.md AC-167: the stripes of a Claude Code agent, not known until its status line says them. */
export async function subscriptionStripes(page, { pool, cli, newPane, envOf, tmux, shots }) {
  const stripes = () => page.textContent(`${pool} .agsubs`);
  check("AC-167 no reading yet: two stripes, each not known", (await stripes()).match(/not known/g)?.length === 2, await stripes());
  const pane = newPane("status"), now = Math.floor(Date.now() / 1000);
  const input = JSON.stringify({ session_id: "s", rate_limits: { five_hour: { used_percentage: 34, resets_at: now + 3600 }, seven_day: { used_percentage: 95, resets_at: now + 86400 } } });
  let t = "";
  // the pane is listed within a layout: the status line is drawn again until terminumd knows it, as Claude Code redraws it
  const said = await until(async () => {
    execFileSync(cli, ["statusline", "true"], { input, env: envOf(pane), timeout: 8000 });
    t = await stripes();
    return /5h\s*34%/.test(t) && /week\s*95%/.test(t);
  }, 10000);
  check("AC-167 Claude Code's status line fills its stripes", said, t);
  check("AC-167 a window at 90% or more takes the danger color", !!(await page.$(`${pool} .agtrack.high`)));
  const width = await page.$eval(`${pool} .agsub .agfill`, (f) => f.getBoundingClientRect().width / f.parentElement.getBoundingClientRect().width);
  check("AC-167 the five-hour stripe filled a third", Math.abs(width - 0.34) < 0.02, width);
  const over = await page.$eval(pool, (card) => card.scrollWidth > card.clientWidth + 1 || [...card.querySelectorAll(".agsub")].some((s) => s.getBoundingClientRect().right > card.getBoundingClientRect().right + 1));
  check("AC-167 the stripes fit the rectangle", !over);
  if (shots) await page.screenshot({ path: path.join(shots, "agent-stripes.png") });
  await page.click(pool, { position: { x: 60, y: 8 } });
  await page.waitForSelector("#agentbox[open]");
  const view = await page.textContent("#agentview");
  check("AC-167 its window says each window's reset and when it was read", /34% · resets/.test(view) && /Read \w+ \d\d:\d\d/.test(view), view);
  await page.click('#agentbox button[value="cancel"]');
  tmux("kill-pane", "-t", pane);
}
