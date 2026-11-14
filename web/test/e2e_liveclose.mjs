// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-133: a visible subtask link starts a real opening read; closing its parent cancels it. Save keeps a comment
// once under a real change of its task (AC-108). Two quick field changes land in the order made; a text saved under
// another edit redraws nothing over it.
import { boardKit, hold, nap, pressCard } from "./boardkit.mjs";
import { check, until } from "./check.mjs";

export async function liveTaskClose(page, opts) {
  const { j, open } = boardKit(page, opts);
  const project = (await j("/api/task-projects", "POST", { name: "Close windows", prefix: "CLS" })).id;
  const parent = (await j("/api/tasks", "POST", { title: "Close while a subtask opens", project })).id;
  const child = (await j("/api/tasks", "POST", { title: "The delayed subtask", project, parent })).id;
  const route = `**/api/tasks/${child}`;
  for (const how of ["×", "Esc", "Back"]) {
    await open(`?project=${project}`);
    await pressCard(page, parent);
    await page.waitForSelector("#taskbox[open]");
    const held = await hold(page, route, "GET");
    // Unrelated traffic must not hold this check: the board can still be reading while a task closes.
    const backgroundPath = "/api/tasks/meta?close-check=1", background = await hold(page, `**${backgroundPath}`, "GET");
    try {
      const backgroundAsked = page.waitForRequest(`**${backgroundPath}`);
      await page.evaluate(([child, backgroundPath]) => {
        const fetch = window.fetch;
        const read = window.taskCloseRead = { consumed: false, restore: () => { window.fetch = fetch; } };
        window.fetch = async (...args) => {
          const response = await fetch(...args);
          if (new URL(response.url).pathname === `/api/tasks/${child}`) {
            const text = response.text.bind(response);
            response.text = async () => {
              const body = await text();
              // api() parses text, then open() checks cancellation in microtasks. A later task sees their result.
              setTimeout(() => { read.consumed = true; }, 0);
              return body;
            };
          }
          return response;
        };
        window.taskCloseBackground = fetch(backgroundPath).then((r) => r.text());
      }, [child, backgroundPath]);
      await backgroundAsked;
      const asked = page.waitForRequest(route);
      await page.locator("#taskbody .subname").click();
      await asked;
      check(`AC-133 live ${how}: the parent's window stays while its subtask is read`,
        (await page.textContent("#tasktitle")).startsWith(`${parent} `));
      if (how === "×") await page.click("#taskclose");
      else if (how === "Esc") await page.keyboard.press("Escape");
      else await page.goBack();
      await page.waitForSelector("#taskbox[open]", { state: "hidden" });
      await page.waitForFunction(() => location.pathname === "/kanban/");
      const answered = page.waitForResponse(route);
      held.release();
      const response = await answered, task = await response.json();
      await page.waitForFunction(() => window.taskCloseRead.consumed);
      check(`AC-133 live ${how}: the real delayed subtask cannot reopen the closed window`,
        background.sent() === 1 && response.ok() && task.id === child && !await page.$eval("#taskbox", (b) => b.open) && new URL(page.url()).pathname === "/kanban/",
        `read status ${response.status()}, task ${task.id}`);
    } finally {
      held.release();
      background.release();
      await page.evaluate(async () => {
        window.taskCloseRead.restore();
        try { await window.taskCloseBackground; }
        finally { delete window.taskCloseRead; delete window.taskCloseBackground; }
      });
      await background.end();
      await held.end();
    }
  }
  await open(`?project=${project}`);
  await pressCard(page, parent);
  await page.waitForSelector("#taskbox[open]");
  await page.fill(".newcomment textarea", "Kept under a live change");
  const key = page.locator("#taskacts").getByRole("button", { name: "Save", exact: true }), rect = await key.boundingBox();
  await key.evaluate((b) => { window.pressedSave = b; });
  await page.mouse.move(rect.x + rect.width / 2, rect.y + rect.height / 2);
  await page.mouse.down();
  let changed;
  try {
    changed = await j(`/api/tasks/${parent}`, "PATCH", { text: "A live change while Save is pressed." });
    await page.waitForFunction(() => document.querySelector("#taskbody .tasktext")?.textContent.includes("A live change while Save is pressed."));
  } finally { await page.mouse.up(); }
  await page.waitForFunction(() => document.querySelector("#taskbody .ev-comment.by-human .markdown")?.textContent.includes("Kept under a live change"));
  const kept = (await j(`/api/tasks/${parent}`)).history.filter((e) => e.kind === "comment" && e.text === "Kept under a live change").length;
  check("AC-133 live a real task change during a Save press keeps its key, the comment once, the window open and its field empty",
    changed.code === 200 && kept === 1 && await page.evaluate(() => window.pressedSave.isConnected) && await page.$eval("#taskbox", (b) => b.open) && await page.inputValue(".newcomment textarea") === "");
  await page.click("#taskclose");
  await page.waitForFunction(() => location.pathname === "/kanban/");
  await liveFieldOrder(page, j, open, parent);
}

/** Severity changed twice at once, terminumd's first answer held back: the second waits for it and stands (AC-133). */
async function liveFieldOrder(page, j, open, id) {
  await j(`/api/tasks/${id}`, "PATCH", { severity: "P3" });
  await open(`?project=${(await j(`/api/tasks/${id}`)).project}`);
  await pressCard(page, id);
  await page.waitForSelector("#taskbox[open]");
  const route = `**/api/tasks/${id}`, held = await hold(page, route, "PATCH");
  const severity = page.locator("#taskbody .fieldrow", { hasText: "Severity" }).locator("select");
  try {
    await severity.selectOption("P1");
    await until(() => held.sent() === 1);
    await severity.selectOption("P0");
    await nap(300);
    check("AC-133 live a field's second change waits for the answer to the first", held.sent() === 1, `sent ${held.sent()}`);
    held.release();
    const stood = await until(async () => held.sent() === 2 && (await j(`/api/tasks/${id}`)).severity === "P0" && await severity.inputValue() === "P0");
    check("AC-133 live the last choice of two quick ones stands, in terminumd and in the window", stood, `shown ${await severity.inputValue()}`);
  } finally {
    held.release();
    await held.end();
  }
  // refused while the text's editor holds the window's redraw: the field goes back to what stands
  const refuse = (r) => r.request().method() === "PATCH" ? r.fulfill({ status: 400, contentType: "application/json", body: '{"error":"Refused for the check."}' }) : r.continue();
  await page.click('#taskbody .tasktext button[aria-label="Edit"]');
  await page.route(route, refuse);
  try {
    await severity.selectOption("P2");
    check("AC-133 live a refused choice goes back to what stands while an edit is open",
      await until(async () => await severity.inputValue() === "P0") && (await j(`/api/tasks/${id}`)).severity === "P0", await severity.inputValue());
  } finally { await page.unroute(route, refuse); }
  await page.locator(".tasktext").getByRole("button", { name: "Cancel", exact: true }).click();
  // a text saved while a comment of the task is being edited: the comment's edit stays as typed
  await j(`/api/tasks/${id}/comments`, "POST", { text: "Edited while the text is saved" });
  const comment = page.locator('#taskbody .ev-comment.by-human:has-text("Edited while the text is saved")');
  await comment.locator('button[aria-label="Edit"]').waitFor();   // drawn before the text's editor holds the window
  await page.click('#taskbody .tasktext button[aria-label="Edit"]');
  // its edit cancelled first while the text's editor holds the window, then opened again: the new one holds too
  await comment.locator('button[aria-label="Edit"]').click();
  await comment.locator(".markdown").getByRole("button", { name: "Cancel", exact: true }).click();
  await comment.locator('button[aria-label="Edit"]').click();
  await page.fill("#taskbody .ev-comment.by-human .markdown textarea:not(:disabled)", "Comment being rewritten");
  await page.fill(".tasktext textarea", "Text saved under another edit");
  await page.locator(".tasktext").getByRole("button", { name: "Save", exact: true }).click();
  const saved = await until(async () => (await j(`/api/tasks/${id}`)).text === "Text saved under another edit");
  await nap(300);
  const typed = await page.$eval("#taskbody .ev-comment.by-human .markdown textarea:not(:disabled)", (a) => a.value).catch(() => "its editor gone");
  check("AC-133 live Save of a text redraws nothing over a comment being edited", saved && typed === "Comment being rewritten", typed);
  await page.locator("#taskbody .ev-comment.by-human .markdown").getByRole("button", { name: "Cancel", exact: true }).click();
  check("AC-133 live the comment's edit closed, the window shows the saved text",
    await until(async () => (await page.textContent("#taskbody .tasktext")).includes("Text saved under another edit") && !await page.$("#taskbody .tasktext textarea, #taskbody .markdown textarea")));
  await page.click("#taskclose");
  await page.waitForFunction(() => location.pathname === "/kanban/");
}
