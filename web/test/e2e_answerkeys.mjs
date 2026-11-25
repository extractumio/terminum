// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-285: a waiting task's Answer, Continue with Recommended and Archive in its window, on a Mac and a phone.
import { check, runningIn, until } from "./check.mjs";
import { drawn, openPage, SIZES } from "./fake.mjs";

const ASK = "Why you are needed: choose the API.\n\nRecommended: 1. v2.\n\nDetails: both work today.";
const TASK = {
  id: "API-12", uid: "abcd1234", title: "Choose the API", project: "default", severity: "P2", status: "WAIT FOR HUMAN",
  text: "Move the clients.", text_rev: 1, files: [], children: [], parent: null, lists: {}, created_at: 1, updated_at: 2,
  session: "tmux:local:terminum:%8", asked: "q1",
  history: [{ id: "q1", kind: "comment", author_kind: "agent", author_name: "Agent", at: 1, text: ASK, questions: true }],
};

export async function webAnswerKeys(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device });
    const { page } = f;
    const task = structuredClone(TASK), posts = [];
    let reply = { typed: TASK.session };
    const json = (r, data, status = 200) => r.fulfill({ status, contentType: "application/json", body: JSON.stringify(data) });
    try {
      await drawn(page);
      await page.route("**/api/tasks/API-12", (r) => json(r, task));
      await page.route("**/api/tasks/API-12/answer", (r) => {
        posts.push(r.request().postDataJSON());
        return reply.error ? json(r, reply, 400) : json(r, reply);
      });
      await page.evaluate(async () => {
        const { taskWindow } = await import("/kanbanwin.js");
        window.opened = [];
        window.taskwin = taskWindow({ getMeta: () => ({ projects: [{ id: "default", name: "Default" }], agents: [], users: [], sessions: [] }),
          runs: { open: async (list, how) => { window.opened.push(how); return true; } }, openTask() {}, newSubtask() {}, openSchedule() {} });
      });
      const open = async () => { await page.evaluate(() => window.taskwin.open("API-12")); await page.waitForSelector("#taskbox[open]"); };
      // a phone's ways on stand in the bar at the window's top (phonewin.js), what is seldom pressed in the row
      const key = (label) => page.locator("#taskbox").getByRole("button", { name: label, exact: true });
      const press = (label) => device.hasTouch ? key(label).tap() : key(label).click();
      const isOpen = () => page.$eval("#taskbox", (b) => b.open);
      const input = page.locator("#taskbody textarea").last();

      await open();
      const shown = ["Archive", "Save", "Continue with Recommended", "Answer", "Run"];
      const labels = (await page.locator("#taskacts button, #taskbox .barkeys button").allTextContents()).filter((l) => shown.includes(l));
      const order = device.hasTouch ? ["Save", "Answer", "Archive", "Continue with Recommended"] : ["Archive", "Continue with Recommended", "Save", "Answer"];
      check("AC-285 the keys: Archive and Continue with Recommended at the left, Save and Answer at the right; no Run", JSON.stringify(labels) === JSON.stringify(order), JSON.stringify(labels));
      check("AC-285 Answer is off while the field is empty", await key("Answer").isDisabled());
      const fits = await page.locator("#taskacts button, #taskbox .barkeys button").evaluateAll((keys) => keys.every((b) => {
        const r = b.getBoundingClientRect();
        return b.scrollWidth <= b.clientWidth + 1 && r.left >= 0 && r.right <= innerWidth;
      }));
      check("AC-285 every key's label fits, in sight", fits);

      // a refusal: said where the press was, the answer kept, the window open
      reply = { error: "That question is no longer waiting for an answer." };
      await input.fill("Use v2");
      await press("Answer");
      check("AC-285 a stale question is refused, the text kept, the window open",
        await until(() => page.getByText(reply.error).isVisible()) && (await input.inputValue()) === "Use v2" && await isOpen());

      // into a live session the human opened: terminumd typed it; the window closes
      reply = { typed: TASK.session };
      await press("Answer");
      check("AC-285 Answer posts the question, the choice and the text, then the window closes",
        await until(async () => !await isOpen()) && JSON.stringify(posts.at(-1)) === JSON.stringify({ choice: "answer", question: "q1", text: "Use v2", files: [] }),
        JSON.stringify(posts.at(-1)));
      check("AC-285 typed: no RUN window", await page.evaluate(() => window.opened.length) === 0);

      // elsewhere: on as RUN, with the fixed comment and the question's mark; what is written stays
      await open();
      await input.fill("my own note");
      reply = { run: true, comment: "Go ahead with the recommended answer(s)." };
      await press("Continue with Recommended");
      const went = await until(async () => (await page.evaluate(() => window.opened.length)) === 1) && await page.evaluate(() => window.opened[0]);
      check("AC-285 Continue with Recommended goes on as RUN with its fixed words and the question",
        went && went.words === reply.comment && JSON.stringify(went.answer) === JSON.stringify({ choice: "recommended", question: "q1" }) && went.atOnce === true,
        JSON.stringify(went));
      check("AC-285 Continue with Recommended sends no text of the field", posts.at(-1).text === "");

      // Archive asks first, then goes with what is written
      await open();
      await input.fill("Not needed after all");
      reply = { archived: true };
      await press("Archive");
      await page.waitForSelector("#confirmbox[open]");
      check("AC-285 Archive says what follows", (await page.locator("#confirmtext").textContent()).includes("fails as not wanted"));
      await page.locator("#confirmok").click();
      check("AC-285 Archive posts its choice with the written comment, and the window closes",
        await until(async () => !await isOpen()) && JSON.stringify(posts.at(-1)) === JSON.stringify({ choice: "archive", question: "q1", text: "Not needed after all", files: [] }),
        JSON.stringify(posts.at(-1)));

      // a wait with no question (AC-124: it asks in its window): RUN as before
      delete task.asked;
      await open();
      check("AC-285 a wait without a question keeps Run", await key("Run").isVisible() && await key("Answer").count() === 0
        && await key("Continue with Recommended").count() === 0);
      check("AC-285 no page errors", f.errors.length === 0, f.errors.join(" | "));
    } finally { await f.close(); }
  }
}
