// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// An agent's subscription stripes on the board, against a real private terminumd (terminum-dispatcher.md AC-168):
// not known until Claude Code's status line, through the real `terminum statusline` in a pane terminumd lists, says them;
// then a usage command set in settings.json (a stand-in printing /usage's JSON) reads them, and one that fails says why.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { check, until } from "./check.mjs";

/** terminum-dispatcher.md AC-168: the stripes of a Claude Code agent, not known until its status line says them. */
export async function subscriptionStripes(page, { pool, cli, newPane, envOf, tmux, shots, home }) {
  const stripes = () => page.textContent(`${pool} .agsubs`);
  check("AC-168 no reading yet: two stripes, each not known", (await stripes()).match(/not known/g)?.length === 2, await stripes());
  const pane = newPane("status"), now = Math.floor(Date.now() / 1000);
  const input = JSON.stringify({ session_id: "s", rate_limits: { five_hour: { used_percentage: 34, resets_at: now + 3600 }, seven_day: { used_percentage: 95, resets_at: now + 86400 } } });
  let t = "";
  // the pane is listed within a layout: the status line is drawn again until terminumd knows it, as Claude Code redraws it
  const said = await until(async () => {
    execFileSync(cli, ["statusline", "true"], { input, env: envOf(pane), timeout: 8000 });
    t = await stripes();
    return /5h\s*34%/.test(t) && /week\s*95%/.test(t);
  }, 10000);
  check("AC-168 Claude Code's status line fills its stripes", said, t);
  const hues = await page.$$eval(`${pool} .agtrack`, (ts) => ts.map((t) => [t.className, getComputedStyle(t.firstElementChild).backgroundColor]));
  check("AC-168 a window over 90% is red, one under 40% green", hues.length === 2 && /\blow\b/.test(hues[0][0]) && /\bhigh\b/.test(hues[1][0]) && hues[0][1] !== hues[1][1], JSON.stringify(hues));
  const width = await page.$eval(`${pool} .agsub .agfill`, (f) => f.getBoundingClientRect().width / f.parentElement.getBoundingClientRect().width);
  check("AC-168 the five-hour stripe filled a third", Math.abs(width - 0.34) < 0.02, width);
  const over = await page.$eval(pool, (card) => card.scrollWidth > card.clientWidth + 1 || [...card.querySelectorAll(".agsub")].some((s) => s.getBoundingClientRect().right > card.getBoundingClientRect().right + 1));
  check("AC-168 the stripes fit the rectangle", !over);
  if (shots) await page.screenshot({ path: path.join(shots, "agent-stripes.png") });
  await page.click(pool, { position: { x: 60, y: 8 } });
  await page.waitForSelector("#agentbox[open]");
  const view = await page.textContent("#agentview");
  check("AC-168 its window says each window's reset and when it was read", /34% · resets/.test(view) && /Read \w+ \d\d:\d\d/.test(view), view);
  await page.click('#agentbox button[value="cancel"]');
  tmux("kill-pane", "-t", pane);
  await usageCommand(page, { pool, home });
}

/** A shell word: `s` in single quotes. */
const quoted = (s) => `'${s.replaceAll("'", `'\\''`)}'`;

/** AC-168: Claude Code's usage command, as settings.json names it, read on a beat; a failing one says why, its last reading kept. */
async function usageCommand(page, { pool, home }) {
  const file = path.join(home, "settings.json"), was = fs.existsSync(file) ? fs.readFileSync(file, "utf8") : null;
  const set = (claude) => fs.writeFileSync(file, JSON.stringify({ dispatcher: { usageCommands: { claude } } }));
  const usage = JSON.stringify([{ type: "system", subtype: "init" }, { type: "assistant", usage_report: { rate_limits: { limits: [{ kind: "weekly_all", percent: 61, resets_at: null, scope: null }] } } }, { type: "result", total_cost_usd: 0 }]);
  const stripes = () => page.textContent(`${pool} .agsubs`);
  const view = async () => {
    await page.click(pool, { position: { x: 60, y: 8 } });
    await page.waitForSelector("#agentbox[open]");
    const t = await page.textContent("#agentview");
    await page.click('#agentbox button[value="cancel"]');
    return t;
  };
  try {
    set(`printf '%s' ${quoted(usage)}`);
    let t = "";
    check("AC-168 the usage command set in settings.json fills its week", await until(async () => /week\s*61%/.test(t = await stripes()), 15000), t);
    check("AC-168 its window says the usage command read it", /Read \w+ \d\d:\d\d by Claude Code's usage command/.test(t = await view()), t);
    set("echo 'not logged in' >&2; exit 3");
    check("AC-168 a failing usage command says why in its window", await until(async () => /usage command failed .*exit 3: not logged in\./.test(t = await view()), 15000), t);
    check("AC-168 and its last reading stands", /week\s*61%/.test(await stripes()), await stripes());
  } finally {
    if (was === null) fs.rmSync(file, { force: true });
    else fs.writeFileSync(file, was);
  }
}
