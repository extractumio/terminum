// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Icon keys pressed with the pointer take no focus (docs/UI-GUIDELINES.md "Focus"): Esc for what they opened, or a key
// typed after, lights no ring round them in Chrome; a menu one opened gives the keys back to the terminal; Tab still
// reaches and rings them. `node web/test/e2e_live.mjs iconkeys`.
import { check, until } from "./check.mjs";
import { boardKit, nap, ring, tabKey } from "./boardkit.mjs";

/** A press on `sel`, Esc for what it opened, a key typed after: Chrome lights a focused key's ring by either. */
async function pressed(page, sel) {
  await page.click(sel);
  await nap(200);
  await page.keyboard.press("Escape");
  await nap(200);
  await page.keyboard.press("Shift+ArrowRight");
  return ring(page, sel);
}

/** The board's icon keys; `phone`: those a phone shows (its pool is a row of its own), and no session list. */
export async function liveIconKeys(page, live, phone = false) {
  const { j, open, backToSessions } = boardKit(page, live);
  const agent = phone ? null : (await j("/api/task-agents", "POST", { name: "Icon agent", kind: "claude", command: "claude" })).id;
  await open("?project=default");
  const newTask = '.col[data-status="TODO"] .new';
  const keys = {
    "AC-106 + New task": newTask,
    "AC-105 a column's sort": '.col[data-status="TODO"] .colsort button',
    ...(phone ? {} : { "AC-138 Add agent": 'button[aria-label="Add agent"]', "AC-138 an agent's Edit": 'button[aria-label="Edit Icon agent"]' }),
  };
  for (const [name, sel] of Object.entries(keys)) {
    const r = await pressed(page, sel);
    check(`${name}, pressed, then Esc and a key, draws no focus ring`, !r.lit && !r.focused, JSON.stringify(r));
  }
  if (!phone) {                                    // Tab from the column's name, the key before it (a phone's columns are tabs)
    await page.evaluate(() => { const n = document.querySelector('.col[data-status="TODO"] .colname'); n.tabIndex = -1; n.focus(); });
    await tabKey(page);
    const r = await ring(page, newTask);
    check("AC-106 Tab still reaches + New task and rings it", r.focused && r.lit, JSON.stringify(r));
    await page.evaluate(() => document.activeElement?.blur());
  }
  if (agent) await j(`/api/task-agents/${agent}`, "DELETE");
  if (!phone) await liveListKeys(page, backToSessions);
}

/** A group's ... key in the session list: its menu, opened by a press and closed by Esc, gives the keys back to the
 *  terminal; opened from the keyboard, back to the key. */
async function liveListKeys(page, backToSessions) {
  const menu = "#sessions h2 .hostbtn";
  await backToSessions();
  check("AC-07 the terminal has the keys", await until(() => page.evaluate(() => document.activeElement?.id === "kbd")));
  await page.hover("#sessions .grp h2:has(.hostbtn)");
  await page.click(menu);
  const opened = await until(() => page.isVisible("#rowmenu[open]"));
  await page.keyboard.press("Escape");
  await until(async () => !(await page.isVisible("#rowmenu[open]")));
  let r = await ring(page, menu);
  const kbd = await page.evaluate(() => document.activeElement?.id);
  check("AC-07 a group's ... key, pressed, its menu closed by Esc: no ring round it, the terminal has the keys again", opened && !r.lit && !r.focused && kbd === "kbd", JSON.stringify({ ...r, kbd }));
  await page.focus(menu);
  await page.keyboard.press("Enter");
  await until(() => page.isVisible("#rowmenu[open]"));
  await page.keyboard.press("Escape");
  await until(async () => !(await page.isVisible("#rowmenu[open]")));
  r = await ring(page, menu);
  check("AC-07 opened from the keyboard, Esc gives the focus back to the ... key", r.focused, JSON.stringify(r));
  await page.evaluate(() => document.activeElement?.blur());
}
