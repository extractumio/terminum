// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-230 and AC-157: a task window's one-press choices, refusals, and free-text answers.
import path from "node:path";
import { check, runningIn, until } from "./check.mjs";
import { drawn, openPage, SIZES, THEME } from "./fake.mjs";

const ASK = "Why you are needed: choose how long the retry can wait.\n\nRecommended: Wait.\n\nDetails: the first attempt timed out.";
const LABEL = "<b>Wait</b> **then retry**";
const QUESTION = { kind: "operational", options: [{ id: "retry", label: "Retry" }, { id: "wait", label: LABEL },
  { id: "later", label: "Give the second attempt more time ".repeat(5).trim() }], recommended: "wait" };
const TASK = {
  id: "API-12", uid: "abcd1234", title: "Retry the job", project: "default", severity: "P2", status: "WAIT FOR HUMAN",
  text: "Retry a job that timed out.", text_rev: 1, files: [], children: [], parent: null, lists: {}, created_at: 1, updated_at: 2,
  agent: { name: "Agent", conversation: "test-conversation", color: 0 }, active_question: "q1",
  history: [{ id: "q1", kind: "comment", author_kind: "agent", author_name: "Agent", at: 1, text: ASK, questions: true, question: QUESTION }],
};
const LIGHT = { ...THEME, bg: "#ffffff", fg: "#1f2328", dark: false };

export async function webQuestionChoices(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const theme = device.hasTouch ? LIGHT : THEME, f = await openPage(browser, { device,
      hook: (m, say) => { if (m.t === "sub") say({ t: "theme", sid: m.id, theme }); } });
    const { page } = f;
    const task = structuredClone(TASK), posts = [];
    const json = (r, data, status = 200) => r.fulfill({ status, contentType: "application/json", body: JSON.stringify(data) });
    try {
      await drawn(page);
      await page.route("**/api/tasks/API-12", (r) => json(r, task));
      await page.route("**/api/tasks/API-12/comments/q1/choice", async (r) => {
        const body = r.request().postDataJSON();
        posts.push(body);
        if (posts.length === 1) return json(r, { error: "That question is no longer waiting for an answer." }, 400);
        task.history.push({ id: "a1", kind: "comment", author_kind: "human", author_name: "You", at: 3,
          text: "Selected option: wait.", choice: { question: "q1", option: body.option } });
        return json(r, { event: "a1" });
      });
      await page.route("**/api/tasks/API-12/comments", async (r) => {
        const { text } = r.request().postDataJSON();
        task.history.push({ id: "a2", kind: "comment", author_kind: "human", author_name: "You", at: 4, text });
        return json(r, { event: "a2" });
      });
      await page.evaluate(async () => {
        const { taskWindow } = await import("/kanbanwin.js");
        window.runs = 0;
        window.taskwin = taskWindow({ getMeta: () => ({ projects: [{ id: "default", name: "Default" }], agents: [], users: [], sessions: [] }),
          runs: { open: async () => { window.runs++; return true; } }, openTask() {}, newSubtask() {}, openSchedule() {} });
        await window.taskwin.open("API-12");
      });
      const recommended = page.getByRole("button", { name: `${LABEL} (Recommended)`, exact: true });
      const buttons = page.locator(".question-choices button");
      const sizeAndText = await buttons.evaluateAll((keys) => keys.map((b) => {
        const r = b.getBoundingClientRect();
        return { width: r.width, height: r.height, childElements: b.querySelector("span").children.length, fits: b.scrollHeight <= b.clientHeight && b.scrollWidth <= b.clientWidth };
      }));
      check("AC-230 all options have literal labels, wrap in the window and give fingers room",
        sizeAndText.length === 3 && sizeAndText.every((b) => !b.childElements && b.fits && (b.width >= 40 && b.height >= (device.hasTouch ? 40 : 24))), JSON.stringify(sizeAndText));
      check("AC-230 the recommendation uses the existing primary key", await recommended.evaluate((b) => b.classList.contains("primary")));
      if (process.env.TM_TEST_QUESTION_SHOTS) await page.locator("#taskbox").screenshot({
        path: path.join(process.env.TM_TEST_QUESTION_SHOTS, `question-${name}-${device.viewport.width}.png`),
      });
      const input = page.locator("#taskbody textarea").last();
      await input.fill("Wait an hour.");
      if (device.hasTouch) await recommended.tap();
      else {
        await page.getByRole("button", { name: "Retry", exact: true }).focus();
        await page.keyboard.press(name === "webkit" ? "Alt+Tab" : "Tab");
        check("AC-230 Tab reaches the recommendation and shows focus", await recommended.evaluate((b) =>
          b.matches(":focus-visible") && getComputedStyle(b).outlineStyle !== "none"));
        await page.keyboard.press("Enter");
      }
      check("AC-230 a refused choice is reported and the draft stays", await until(() => page.getByText("That question is no longer waiting for an answer.").isVisible())
        && (await input.inputValue()) === "Wait an hour.");
      if (device.hasTouch) await recommended.tap();
      else await recommended.click();
      await page.getByText(`Saved answer: ${LABEL}`, { exact: true }).waitFor();
      check("AC-230 one press posts just the choice ID and saves without running or changing the text draft",
        JSON.stringify(posts) === JSON.stringify([{ option: "wait" }, { option: "wait" }])
        && (await page.evaluate(() => window.runs)) === 0 && (await input.inputValue()) === "Wait an hour.", JSON.stringify(posts));
      await page.locator("#taskbox").getByRole("button", { name: "Save", exact: true }).click();
      await page.getByText("Wait an hour.", { exact: true }).waitFor();
      check("AC-157 Run is still a separate press", await page.evaluate(() => window.runs) === 0);
      await page.locator("#taskbox").getByRole("button", { name: "Run", exact: true }).click();
      check("AC-157 the tally records one Run", await page.evaluate(() => window.runs) === 1);
      const old = task.history[0];
      delete old.question;
      delete task.active_question;
      task.history = [old];
      await page.evaluate(() => window.taskwin.open("API-12"));
      check("AC-230 an older question shows without any option buttons", await buttons.count() === 0 && await page.getByText("Asks you", { exact: true }).first().isVisible());
      check("AC-230 no page errors", f.errors.length === 0, f.errors.join(" | "));
    } finally { await f.close(); }
  }
}
