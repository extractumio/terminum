// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// One question at a time, and a window that keeps something sends it once (terminum.md AC-165), on the
// board against a real private terminumd (e2e_live.mjs): terminumd's answer is held back, Enter pressed
// again meanwhile; two questions asked at once, one Enter answers one. What is typed meanwhile is never
// lost, and a file that lands late never lands in another draft (terminum-kanban.md AC-133).
import { boardKit, nap, pressRun } from "./boardkit.mjs";
import { check, until } from "./check.mjs";

/** terminumd's answers to `method` on `url` held until `release()`, or lost by `release("abort")`; `sent()` counts
 *  the requests. */
async function hold(page, url, method) {
  let release, sent = 0;
  const gate = new Promise((r) => { release = r; });
  const handler = async (route) => {
    if (route.request().method() !== method) return route.fallback();
    sent++;
    return await gate === "abort" ? route.abort() : route.continue();
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
    await page.evaluate((box) => document.querySelector(`${box} button[value="cancel"]`).click(), box);
    await nap(300);
    check(`AC-165 ${what}: Enter again while it is sent sends nothing, Esc and Cancel wait, nothing asked`,
      held.sent() === 1 && !!(await page.$(`${box}[open]`)) && await page.isDisabled(ok) && !(await page.$("#confirmbox[open]")), `sent=${held.sent()}`);
    held.release();
    check(`AC-165 ${what}: kept once`, await until(kept) && held.sent() === 1, `sent=${held.sent()}`);
  } finally { held.release(); await held.end(); }
}

export async function liveBoardBusy(page, opts) {
  const { base, work } = opts;
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
    open: async () => { await page.click("#fproject summary"); await page.click("#newproject"); await page.waitForSelector("#projectbox[open]"); await page.fill("#projectname", "Made once"); },
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

  await newTaskOnce(page, { base, j, project });
  await commentOnce(page, { base, j, id: await make("commented once") });
  await commentWaitsForFiles(page, { base, j, id: await make("commented with a file") });
  await lateFile(page, { base, project });
  const runs = (await j("/api/task-projects", "POST", { name: "Run once", dir: work })).id;
  await runOnce(page, { base, project: runs, id: (await j("/api/tasks", "POST", { title: "run once", project: runs })).id });
  await page.goto(`${base}/kanban/`);
}

/** Text typed into `field` while terminumd has not answered: it stands still, nothing typed goes or is lost. */
async function typeMeanwhile(page, field) {
  await page.focus(field);
  await page.keyboard.type(" typed meanwhile");
  return page.inputValue(field);
}

/** The new task's window: Enter and ⌘Enter again send nothing, the text stands still and Esc waits; made once. */
async function newTaskOnce(page, { base, j, project }) {
  await page.goto(`${base}/kanban/?project=${project}`);
  await page.click("#newtask");
  await page.waitForSelector("#newbox[open]");
  await page.fill("#newtitle", "Made once by Enter");
  await page.fill("#newtext", "sent");
  const held = await hold(page, "**/api/tasks", "POST");
  try {
    for (let i = 0; i < 3; i++) await page.press("#newtitle", "Enter");
    await until(() => held.sent() > 0);
    await page.press("#newtext", "ControlOrMeta+Enter");
    const text = await typeMeanwhile(page, "#newtext");
    await page.keyboard.press("Escape");
    await nap(300);
    check("AC-165 a new task's window: Enter and ⌘Enter again send nothing, Esc waits, the text stands still while it is made",
      held.sent() === 1 && !!(await page.$("#newbox[open]")) && text === "sent" && await page.isDisabled("#newcreate"), `sent=${held.sent()} text=${text}`);
    held.release();
    const mine = async () => (await j(`/api/tasks?project=${project}`)).tasks.filter((t) => t.title === "Made once by Enter");
    check("AC-165 a new task's window: made once, with what was shown; it closes",
      await until(async () => !(await page.$("#newbox[open]")) && (await mine()).length === 1) && (await j(`/api/tasks/${(await mine())[0].id}`)).text === "sent" && held.sent() === 1, `sent=${held.sent()}`);
  } finally { held.release(); await held.end(); }
}

/** A comment's Close by ⌘Enter, three times while terminumd has not answered: one comment, nothing typed lost. */
async function commentOnce(page, { base, j, id }) {
  await page.goto(`${base}/kanban/${id}`);
  await page.waitForSelector("#taskbox[open] .newcomment textarea");
  await page.fill(".newcomment textarea", "said once");
  const held = await hold(page, `**/api/tasks/${id}/comments`, "POST");
  const said = async () => (await j(`/api/tasks/${id}`)).history.filter((h) => h.kind === "comment");
  try {
    for (let i = 0; i < 3; i++) await page.press(".newcomment textarea", "ControlOrMeta+Enter");
    await until(() => held.sent() > 0);
    const text = await typeMeanwhile(page, ".newcomment textarea");
    await page.keyboard.press("Escape");
    await nap(300);
    check("AC-165 a comment: ⌘Enter again sends nothing, Esc waits, the text stands still while it is kept",
      held.sent() === 1 && !!(await page.$("#taskbox[open]")) && text === "said once", `sent=${held.sent()} text=${text}`);
    held.release();
    check("AC-165 a comment: kept once; the window closes and its draft goes",
      await until(async () => !(await page.$("#taskbox[open]")) && (await said()).length === 1) && (await said())[0].text === "said once"
        && await page.evaluate((k) => localStorage.getItem(k) === null, `tm.comment.${id}`), JSON.stringify(await said()));
  } finally { held.release(); await held.end(); }
}

/** A comment's ⌘Enter while its file is on its way, after the window was drawn anew: nothing is sent; once the file
 *  is kept, the comment goes with it. */
async function commentWaitsForFiles(page, { base, j, id }) {
  await page.goto(`${base}/kanban/${id}`);
  await page.waitForSelector("#taskbox[open] .newcomment textarea");
  await page.fill(".newcomment textarea", "with a file");
  const upload = await hold(page, /\/api\/task-files\?/, "POST"), posts = await hold(page, `**/api/tasks/${id}/comments`, "POST");
  posts.release();
  const said = async () => (await j(`/api/tasks/${id}`)).history.filter((h) => h.kind === "comment");
  try {
    await page.evaluate(() => {
      const dt = new DataTransfer(), area = document.querySelector(".newcomment textarea");
      dt.items.add(new File(["kept"], "note.txt", { type: "text/plain" }));
      area.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt }));
    });
    await until(() => upload.sent() > 0);
    await j(`/api/tasks/${id}`, "PATCH", { severity: "P0" });   // the window is drawn anew
    await until(() => page.evaluate(() => [...document.querySelectorAll("#taskbody select")].some((s) => s.value === "P0")));
    await page.press(".newcomment textarea", "ControlOrMeta+Enter");
    await nap(400);
    check("AC-133 a comment's ⌘Enter while its file is on its way, the window drawn anew: nothing is sent, it stays",
      posts.sent() === 0 && !!(await page.$("#taskbox[open]")), `posts=${posts.sent()}`);
    upload.release();
    await until(async () => (await page.inputValue(".newcomment textarea")).includes("note.txt"));
    await page.press(".newcomment textarea", "ControlOrMeta+Enter");
    check("AC-133 once the file is kept, the comment goes once, with it", await until(async () => (await said()).length === 1) && posts.sent() === 1
      && (await said())[0].files?.length === 1, JSON.stringify(await said()));
  } finally { upload.release(); await upload.end(); await posts.end(); }
}

/** A file on its way when its draft is discarded: Create waits for it, and once it lands it is in no other draft. */
async function lateFile(page, { base, project }) {
  await page.goto(`${base}/kanban/?project=${project}`);
  await page.click("#newtask");
  await page.waitForSelector("#newbox[open]");
  await page.fill("#newtitle", "the first draft");
  const held = await hold(page, /\/api\/task-files\?/, "POST");
  try {
    await page.setInputFiles("#newpick", { name: "late.txt", mimeType: "text/plain", buffer: Buffer.from("late") });
    await until(() => held.sent() > 0);
    check("AC-133 Create waits for a file on its way", await page.isDisabled("#newcreate") && await page.isDisabled("#newrun"));
    await page.click("#newdiscard");
    await page.click("#confirmok");
    await page.waitForSelector("#newbox", { state: "hidden" });
    await page.click("#newtask");
    await page.waitForSelector("#newbox[open]");
    await page.fill("#newtitle", "the next draft");
    const landed = page.waitForResponse((r) => r.url().includes("/api/task-files?"));
    held.release();
    await landed;
    await nap(300);
    check("AC-133 a file landing after its draft was discarded is not in the next one, and Create waits for nothing",
      await page.inputValue("#newtext") === "" && (await page.$$("#newfiles li")).length === 0 && !(await page.isDisabled("#newcreate")), await page.inputValue("#newtext"));
    await page.click("#newdiscard");
    await page.click("#confirmok");
    await page.waitForSelector("#newbox", { state: "hidden" });
  } finally { held.release(); await held.end(); }
}

/** RUN's window: ⌘Enter again while terminumd answers sends nothing, and ⌘Enter while Run is off sends none; its
 *  answer is lost here, so no agent starts. */
async function runOnce(page, { base, project, id }) {
  await page.goto(`${base}/kanban/?project=${project}`);
  await pressRun(page, id);
  await page.waitForSelector("#runbox[open]");
  const held = await hold(page, `**/api/tasks/${id}/run`, "POST");
  try {
    for (let i = 0; i < 3; i++) await page.press("#runmessage", "ControlOrMeta+Enter");
    await until(() => held.sent() > 0);
    await page.keyboard.press("Escape");
    await nap(300);
    check("AC-165 RUN's window: ⌘Enter again sends nothing, Esc waits", held.sent() === 1 && !!(await page.$("#runbox[open]")) && await page.isDisabled("#rungo"), `sent=${held.sent()}`);
    held.release("abort");
    check("AC-165 RUN's window: a lost answer is said, the window stays", await until(async () => !(await page.isDisabled("#rungo")) && await page.isVisible("#runerror")) && !!(await page.$("#runbox[open]")));
    await page.evaluate(() => { document.getElementById("rungo").disabled = true; });
    await page.press("#runmessage", "ControlOrMeta+Enter");
    await nap(300);
    check("AC-165 RUN's window: ⌘Enter while Run is off sends nothing", held.sent() === 1, `sent=${held.sent()}`);
    await page.keyboard.press("Escape");
    await page.waitForSelector("#runbox", { state: "hidden" });
  } finally { held.release(); await held.end(); }
}
