// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// One question at a time, and a window that keeps something sends it once (terminum.md AC-165), on the
// board against a real private terminumd (e2e_live.mjs): terminumd's answer is held back, Enter pressed
// again meanwhile; two questions asked at once, one Enter answers one.
import { boardKit, nap } from "./boardkit.mjs";
import { check, until } from "./check.mjs";

/** terminumd's answers to `method` on `url` held until `release()`; `sent()` counts the requests. */
async function hold(page, url, method) {
  let release, sent = 0;
  const gate = new Promise((r) => { release = r; });
  const handler = async (route) => {
    if (route.request().method() !== method) return route.fallback();
    sent++;
    await gate;
    return route.continue();
  };
  await page.route(url, handler);
  return { sent: () => sent, release, end: () => page.unroute(url, handler) };
}

/** Enter pressed in `field` three times while terminumd has not answered: one request, the window
 *  waits (Esc too), then closes once kept. */
async function sendsOnce(page, { what, url, method, open, box, field, ok, kept }) {
  const held = await hold(page, url, method);
  try {
    await open();
    await page.waitForSelector(`${box}[open]`);
    for (let i = 0; i < 3; i++) await page.press(field, "Enter");
    await until(() => held.sent() > 0);
    await page.keyboard.press("Escape");
    await nap(300);
    check(`AC-165 ${what}: Enter again while it is sent sends nothing, Esc waits`,
      held.sent() === 1 && !!(await page.$(`${box}[open]`)) && await page.isDisabled(ok), `sent=${held.sent()}`);
    held.release();
    check(`AC-165 ${what}: kept once`, await until(kept) && held.sent() === 1, `sent=${held.sent()}`);
  } finally { held.release(); await held.end(); }
}

export async function liveBoardBusy(page, opts) {
  const { base } = opts;
  const { j } = boardKit(page, opts);
  const project = (await j("/api/task-projects", "POST", { name: "Once" })).id;
  const meta = () => j("/api/tasks/meta");
  await page.goto(`${base}/kanban/?project=${project}`);
  await page.waitForSelector("#addagent");

  await sendsOnce(page, {
    what: "an agent's window", url: "**/api/task-agents", method: "POST",
    open: async () => { await page.click("#addagent"); await page.waitForSelector("#agentbox[open]"); await page.fill("#agentname", "Only one"); },
    box: "#agentbox", field: "#agentname", ok: "#agentok",
    kept: async () => !(await page.$("#agentbox[open]")) && (await meta()).agents.filter((a) => a.name === "Only one").length === 1,
  });

  await sendsOnce(page, {
    what: "a new project's window", url: "**/api/task-projects", method: "POST",
    open: async () => { await page.click("#newproject"); await page.waitForSelector("#projectbox[open]"); await page.fill("#projectname", "Made once"); },
    box: "#projectbox", field: "#projectname", ok: "#projectok",
    kept: async () => (await meta()).projects.filter((p) => p.name === "Made once").length === 1 && await page.isVisible("#projectaddagent"),
  });
  await page.click("#projectok");
  await page.waitForSelector("#projectbox", { state: "hidden" });

  // a name asked and kept by terminumd (Add existing): Enter again sends nothing while it is kept
  const make = async (title) => (await j("/api/tasks", "POST", { title, project })).id;
  const parent = await make("the parent"), kid = await make("the kid");
  await page.goto(`${base}/kanban/${parent}`);
  await page.waitForSelector("#taskbox[open] .subtasks");
  await sendsOnce(page, {
    what: "a name's dialog that saves", url: `**/api/tasks/${kid}`, method: "PATCH",
    open: async () => { await page.click('#taskbody .subtasks button:has-text("Add existing")'); await page.waitForSelector("#namebox[open]"); await page.fill("#namefield", kid); },
    box: "#namebox", field: "#namefield", ok: "#nameok",
    kept: async () => !(await page.$("#namebox[open]")) && (await j(`/api/tasks/${kid}`)).parent === parent,
  });
  await page.click("#taskclose");

  // two questions at once: the second waits for the first's answer
  const answers = page.evaluate(async () => {
    const { askConfirm } = await import(new URL("dialogs.js", document.baseURI).href);
    window.__asked = [];
    const one = askConfirm("Delete the first? It goes.", "Delete").then((a) => window.__asked.push(["first", a]));
    const two = askConfirm("Delete the second? It goes.", "Delete").then((a) => window.__asked.push(["second", a]));
    await Promise.all([one, two]);
    return window.__asked;
  });
  await page.waitForSelector("#confirmbox[open]");
  check("AC-165 two questions asked at once: one shows, the first", await page.textContent("#confirmtitle") === "Delete the first?");
  await page.keyboard.press("Enter");
  check("AC-165 one Enter answers the first; the second shows then",
    await until(async () => (await page.textContent("#confirmtitle")) === "Delete the second?" && !!(await page.$("#confirmbox[open]")))
      && JSON.stringify(await page.evaluate(() => window.__asked)) === JSON.stringify([["first", true]]),
    JSON.stringify(await page.evaluate(() => window.__asked)));
  await page.keyboard.press("Escape");
  const both = await answers;
  check("AC-165 Esc then answers the second alone", JSON.stringify(both) === JSON.stringify([["first", true], ["second", false]]), JSON.stringify(both));
  await page.goto(`${base}/kanban/`);
}
