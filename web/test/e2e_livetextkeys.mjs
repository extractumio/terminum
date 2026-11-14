// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The board's text keys pressed with the pointer take no focus (docs/UI-GUIDELINES.md "Focus"): what they opened,
// closed by Esc or by its Cancel, or a key typed after, lights no ring round them in Chrome; Tab still reaches and
// rings them. Run by `node web/test/e2e_live.mjs iconkeys`, after the icon keys.
import { check, until } from "./check.mjs";
import { boardKit, pressed, ring, tabKey } from "./boardkit.mjs";

const shut = (page, box) => until(async () => !(await page.$(`${box}[open]`)));

/** `phone`: the board as a phone shows it (its columns are tabs, no session list beside it). */
export async function liveTextKeys(page, live, phone = false) {
  const { j, open } = boardKit(page, live);
  const name = `Keys ${Date.now() % 1e6}`, project = (await j("/api/task-projects", "POST", { name })).id;
  const id = (await j("/api/tasks", "POST", { title: "text keys", project, text: "Some text" })).id;
  const agent = (await j("/api/task-agents", "POST", { project, name: "Key agent", kind: "claude", command: "claude" })).id;
  await open(`?project=${project}`);
  const bar = {
    "AC-106 + Task": ["#newtask", async () => { await page.keyboard.press("Escape"); await shut(page, "#newbox"); }],
    ...(phone ? { "AC-111 Filters": ["#filtersbtn"], "AC-105 a column's tab": ['#colswitch button[data-status="DONE"]'] }
      : { "AC-111 the project's menu": ["#fproject summary"], "AC-112 Show subtasks": ["#fsubtasks"], "AC-142 + Session": ["#adhocsession"],
        "AC-138 an agent's card": [`.agentcard[data-agent="${agent}"]`] }),
  };
  for (const [key, [sel, close]] of Object.entries(bar)) {
    if (!(await page.isVisible(sel))) { check(`${key} is shown`, false, sel); continue; }
    const r = await pressed(page, sel, close);
    check(`${key}, pressed, then closed and a key typed, draws no focus ring`, !r.lit && !r.focused, JSON.stringify(r));
  }
  if (await page.getAttribute("#fsubtasks", "aria-pressed") === "true") await page.click("#fsubtasks");

  // a task's window: a key that opens a window over it, closed by Esc or by its Cancel, leaves no ring
  await page.goto(`${live.base}/kanban/${id}`);
  await page.waitForSelector("#taskbox[open]");
  const sub = '#taskbody button:has-text("Make it a subtask")';
  let r = await pressed(page, sub, async () => { await page.keyboard.press("Escape"); await shut(page, "#namebox"); });
  check("AC-112 Make it a subtask, pressed, its window closed by Esc: no ring round it", !r.lit && !r.focused, JSON.stringify(r));
  await page.click('.tasktext button[aria-label="Edit"]');
  const purge = '.tasktext button:has-text("Purge text")';
  r = await pressed(page, purge, async () => { await page.click('#confirmbox button[value="cancel"]'); await shut(page, "#confirmbox"); });
  check("AC-109 Purge text, pressed, its question closed by Cancel: no ring round it", !r.lit && !r.focused, JSON.stringify(r));
  // a field in focus gives it up to a press on a text key: its change still reaches terminumd (WebKit lost it)
  await page.fill('#taskbody input[type="number"]', "15");
  await page.click('.tasktext button:has-text("Cancel")');
  check("AC-126 a budget typed, then a text key pressed: the budget is kept", await until(async () => (await j(`/api/tasks/${id}`)).budget_seconds === 900),
    JSON.stringify((await j(`/api/tasks/${id}`)).budget_seconds));
  await page.keyboard.press("Escape");
  await shut(page, "#taskbox");

  // the project's window: keys that open a window over it
  if (!phone) {
    await page.click("#fproject summary");
    await page.click(`#fproject button[aria-label="Edit ${name}"]`);
    await page.waitForSelector("#projectbox[open]");
    for (const [key, sel, box] of [["AC-118 Archived…", "#projectarchived", "#archivedbox"], ["AC-128 Memory limits…", "#projectlimits", "#limitsbox"]]) {
      r = await pressed(page, sel, async () => { await page.keyboard.press("Escape"); await shut(page, box); });
      check(`${key}, pressed, its window closed by Esc: no ring round it`, !r.lit && !r.focused, JSON.stringify(r));
    }
    await page.keyboard.press("Escape");
    await shut(page, "#projectbox");
    // Tab still reaches a text key and rings it
    await page.evaluate(() => { const k = document.createElement("button"); k.id = "tabfrom"; document.getElementById("newtask").before(k); k.focus(); });
    await tabKey(page);
    r = await ring(page, "#newtask");
    check("AC-106 Tab still reaches + Task and rings it", r.focused && r.lit, JSON.stringify(r));
    await page.evaluate(() => { document.activeElement?.blur(); document.getElementById("tabfrom")?.remove(); });
  }
  await j(`/api/task-agents/${agent}`, "DELETE");
}
