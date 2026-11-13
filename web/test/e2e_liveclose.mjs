// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-133: a visible subtask link starts a real opening read; closing its parent cancels it.
import { boardKit, pressCard } from "./boardkit.mjs";
import { check } from "./check.mjs";

export async function liveTaskClose(page, opts) {
  const { j, open } = boardKit(page, opts);
  const project = (await j("/api/task-projects", "POST", { name: "Close windows", prefix: "CLS" })).id;
  const parent = (await j("/api/tasks", "POST", { title: "Close while a subtask opens", project })).id;
  const child = (await j("/api/tasks", "POST", { title: "The delayed subtask", project, parent })).id;
  const route = `**/api/tasks/${child}`;
  for (const how of ["Close", "×", "Esc", "Back"]) {
    await open(`?project=${project}`);
    await pressCard(page, parent);
    await page.waitForSelector("#taskbox[open]");
    let release;
    const held = new Promise((done) => { release = done; });
    const handler = async (r) => { await held; await r.continue(); };
    await page.route(route, handler);
    try {
      const asked = page.waitForRequest(route);
      await page.locator("#taskbody .subname").click();
      await asked;
      check(`AC-133 live ${how}: the parent's window stays while its subtask is read`,
        (await page.textContent("#tasktitle")).startsWith(`${parent} `));
      if (how === "Close") await page.getByRole("button", { name: "Close", exact: true }).last().click();
      else if (how === "×") await page.click("#taskclose");
      else if (how === "Esc") await page.keyboard.press("Escape");
      else await page.goBack();
      await page.waitForSelector("#taskbox[open]", { state: "hidden" });
      await page.waitForFunction(() => location.pathname === "/kanban/");
      const answered = page.waitForResponse(route);
      release();
      const response = await answered, task = await response.json();
      await page.waitForLoadState("networkidle");
      check(`AC-133 live ${how}: the real delayed subtask cannot reopen the closed window`,
        response.ok() && task.id === child && !await page.$eval("#taskbox", (b) => b.open) && new URL(page.url()).pathname === "/kanban/",
        `read status ${response.status()}, task ${task.id}`);
    } finally {
      release();
      await page.unroute(route, handler);
    }
  }
  await open(`?project=${project}`);
  await pressCard(page, parent);
  await page.waitForSelector("#taskbox[open]");
  const key = page.getByRole("button", { name: "Close", exact: true }).last(), rect = await key.boundingBox();
  await key.evaluate((b) => { window.pressedClose = b; });
  await page.mouse.move(rect.x + rect.width / 2, rect.y + rect.height / 2);
  await page.mouse.down();
  let changed;
  try {
    changed = await j(`/api/tasks/${parent}`, "PATCH", { text: "A live change while Close is pressed." });
    await page.waitForFunction(() => document.querySelector("#taskbody .tasktext")?.textContent.includes("A live change while Close is pressed."));
  } finally { await page.mouse.up(); }
  await page.waitForSelector("#taskbox[open]", { state: "hidden" });
  check("AC-133 live a real task change during a Close press keeps its key and closes the window",
    changed.code === 200 && await page.evaluate(() => window.pressedClose.isConnected) && !await page.$eval("#taskbox", (b) => b.open));
}
