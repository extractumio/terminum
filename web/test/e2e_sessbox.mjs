// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum-dispatcher.md AC-204 against a fake terminumd (fake.mjs): + Session on an agent's card opens a dock under the
// pool; text, a pasted image and an added file go in it, two task cards dropped on it put their IDs at the caret; Start
// asks terminumd for that agent's session with the prompt and the files' ids, then shows the session; a miss keeps the
// draft and says so; Start folds the dock at once, the agent's key "Starting…", and a refusal or a miss opens it again
// with the draft and the words; Esc keeps the draft, Discard asks and clears it; a remote project's composer takes no
// file; its box has the key to the whole window (terminum.md AC-184).
import { check, coversWindow, runningIn, until } from "./check.mjs";
import { fakeBoard, ORIGIN, openPage, PHONE, WIDE } from "./fake.mjs";

const NOW = Math.floor(Date.now() / 1000);
const task = (n) => ({ id: `API-${n}`, title: `Task ${n}`, project: "api", status: "TODO", severity: "P2", owner: "human", owner_name: "Alex", updated_at: NOW - n, parent: null });
const meta = (host = null) => ({ projects: [{ id: "api", name: "API", color: 0, dir: "/w/api", host }], users: [], sessions: [], hosts: [], last_agents: {}, most_bytes: 1 << 20,
  agents: [{ id: "a1", project: "api", name: "Opus reviewer", kind: "claude", color: 0, line: "claude", at_once: 3 },
    { id: "a2", project: "api", name: "Sonnet worker", kind: "claude", color: 1, line: "claude", at_once: 2 }] });
const json = (r, data, status = 200) => r.fulfill({ status, contentType: "application/json", body: JSON.stringify(data) });
const PNG = Buffer.from("89504e470d0a1a0a0000000d49484452", "hex");

/** `answer(body)`: the start's answer, `status` its HTTP status; `held`: the answer waits for `f.release()`.
 *  `f.redraw()`: the board's next read carries a new reading (of a kind no agent has), so the pool is drawn anew;
 *  `f.idle()`: none of the board's reads in flight. */
async function board(browser, device, { host = null, answer = (b) => ({ key: "tmux:local:terminum:%9", run: "r1", typed: true }), status = 200, held = false } = {}) {
  const f = await openPage(browser, { device, url: "/kanban/?project=api" });
  const posts = [], uploads = [], gate = Promise.withResolvers();
  let readings;
  if (!held) gate.resolve();
  await fakeBoard(f.page, { meta: () => meta(host), tasks: (s) => (s === "TODO" ? [task(1), task(2), task(3)] : []), counts: { TODO: 3 }, subscriptions: () => readings });
  await f.page.route("**/api/task-files?*", (r) => { const name = new URL(r.request().url()).searchParams.get("name"); uploads.push(name);
    return json(r, { id: `${uploads.length}`.padStart(32, "0"), name, bytes: 16, mime: name.endsWith(".png") ? "image/png" : "application/octet-stream" }); });
  await f.page.route("**/api/task-projects/*/session", async (r) => { const body = JSON.parse(r.request().postData() || "{}"); posts.push(body); await gate.promise; return json(r, answer(body), status); });
  // the board's reads in flight: a navigation cancels them, which WebKit reports as a page error
  let reading = 0;
  const ours = (r) => r.url().includes("/api/tasks");
  f.page.on("request", (r) => { if (ours(r)) reading++; });
  for (const end of ["requestfinished", "requestfailed"]) f.page.on(end, (r) => { if (ours(r)) reading--; });
  await f.page.goto(`${ORIGIN}/kanban/?project=api`);
  await f.page.waitForSelector('#agentbar .agentcard[data-agent="a1"]', { state: "attached" });
  return { ...f, posts, uploads, release: gate.resolve, idle: () => reading === 0, redraw: () => { readings = { other: [{ at: Date.now() }] }; } };
}
const dock = (page) => page.evaluate(() => { const d = document.getElementById("sesscompose"), a = document.getElementById("sesstext");
  return { open: !d.hidden && !!d.offsetWidth, agent: document.getElementById("sessagent").textContent, text: a.value, focus: document.activeElement?.id, files: [...document.querySelectorAll("#sessfiles li")].map((l) => l.textContent),
    error: document.getElementById("sesserror").hidden ? "" : document.getElementById("sesserror").textContent, start: document.getElementById("sessstart").disabled,
    keys: [...document.querySelectorAll("#agentbar .agsess")].map((k) => k.getAttribute("aria-expanded")),
    marks: [...document.querySelectorAll("#agentbar .agsess")].map((k) => `${k.textContent}|${k.getAttribute("aria-busy")}|${k.getAttribute("aria-disabled")}`) }; });
const free = (d) => d.marks.every((m) => m === "+ Session|null|null");

/** Start pressed with the answer held: the dock folds at once, that agent's key says it is starting, every key inert. */
async function folds(f, at, agentAt, how) {
  const { page } = f;
  await until(() => f.posts.length > 0);
  let d = await dock(page);
  const want = JSON.stringify([0, 1].map((i) => (i === agentAt ? "Starting…|true|true" : "+ Session|null|true")));
  check(`AC-204 ${at} ${how} folds the dock at once while terminumd starts it; its agent's key says "Starting…", every key inert`, !d.open && JSON.stringify(d.marks) === want, JSON.stringify(d));
  // the board redrawn meanwhile (its columns and pool read again): the keys drawn anew say the same
  f.redraw();
  // the board's wake (kanban.js) alone: with playLook's focus pinned on, WebKit's board still had reads out at the navigation
  await page.evaluate(() => { for (const k of document.querySelectorAll("#agentbar .agsess")) k.dataset.old = "1"; document.dispatchEvent(new Event("visibilitychange")); });
  const redrawn = await until(() => page.evaluate(() => !document.querySelector("#agentbar .agsess[data-old]"))) && await until(f.idle);
  d = await dock(page);
  check(`AC-204 ${at} the pool redrawn while it starts keeps "Starting…" and the inert keys`, redrawn && JSON.stringify(d.marks) === want, JSON.stringify(d));
  await page.evaluate((id) => document.querySelector(`#agentbar .agentcard[data-agent="${id}"] .agsess`).click(), `a${2 - agentAt}`);   // inert: Playwright would not press it
  d = await dock(page);
  check(`AC-204 ${at} another agent's key opens nothing while one starts`, !d.open, JSON.stringify(d));
}

/** Two cards picked, dragged and dropped on the box, as a drag between them carries them (kanban.js). */
async function dropTwo(page) {
  await page.click('.card[data-id="API-1"]', { modifiers: ["Meta"] });
  await page.click('.card[data-id="API-2"]', { modifiers: ["Meta"] });
  await page.evaluate(() => {
    const dt = new DataTransfer(), card = document.querySelector('.card[data-id="API-1"]'), box = document.getElementById("sesstext");
    card.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: dt }));
    box.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: dt }));
    box.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt }));
    card.dispatchEvent(new DragEvent("dragend", { bubbles: true, dataTransfer: dt }));
  });
}

export async function webSessBox(browser, name) {
  let at = `(${name} 1400×900)`;
  runningIn(`${name} 1400×900`);
  let f = await board(browser, WIDE, { held: true });
  let { page } = f;
  await page.click('#agentbar .agentcard[data-agent="a1"] .agsess');
  let d = await dock(page);
  const placed = await page.evaluate(() => { const c = document.querySelector("#agentbar .agentcard").getBoundingClientRect(), k = document.getElementById("sesscompose").getBoundingClientRect(); return k.top >= c.bottom; });
  check(`AC-204 ${at} a card's + Session opens the dock under the pool's cards, for that agent, its box in focus, its key expanded`,
    d.open && placed && d.agent === "Opus reviewer" && d.focus === "sesstext" && JSON.stringify(d.keys) === '["true","false"]' && !(await page.isVisible("#agentbox[open]")), JSON.stringify(d));
  // a long draft in a short window: the box gives way, the dock's head and keys stay in the pool's view
  for (const height of [900, 640]) {
    await page.setViewportSize({ width: 1400, height });
    await page.fill("#sesstext", Array.from({ length: 60 }, (_, i) => `Line ${i + 1} of a long first prompt.`).join("\n"));
    const room = await page.evaluate(() => { const r = (id) => document.getElementById(id).getBoundingClientRect(), bar = r("agentbar"), box = r("sesstext");
      const line = parseFloat(getComputedStyle(document.getElementById("sesstext")).lineHeight) || 16;
      return { seen: ["sesshead", "sessstart", "sessadd"].every((id) => { const k = r(id); return k.top >= bar.top - 1 && k.bottom <= bar.bottom + 1; }),
        lines: Math.floor(box.height / line), scrolled: document.getElementById("agentbar").scrollTop }; });
    check(`AC-204 (${name} 1400×${height}) a 60-line draft: the dock's head, Add files… and Start stay in view, the box at least three lines, the pool unscrolled`,
      room.seen && room.lines >= 3 && room.scrolled === 0, JSON.stringify(room));
  }
  await page.setViewportSize({ width: 1400, height: 900 });
  await page.fill("#sesstext", "");
  await page.click('#agentbar .agentcard[data-agent="a2"] .agsess');
  await page.fill("#sesstext", "Fix what");
  d = await dock(page);
  check(`AC-204 ${at} another card's key moves it to that agent`, d.agent === "Sonnet worker" && JSON.stringify(d.keys) === '["false","true"]', JSON.stringify(d));
  await page.click("#sesstext ~ .xkey");
  const full = await coversWindow(page, ".xbox.full");
  await page.keyboard.press("Escape");
  d = await dock(page);
  check(`AC-184 ${at} its key shows the box over the whole window; Esc gives it back to the dock, still open, the words and the focus kept`,
    full.covers && d.open && d.text === "Fix what" && d.focus === "sesstext" && !(await page.isVisible(".xbox.full")), JSON.stringify({ full, d }));
  // a pasted image and an added file: kept by terminumd, a chip each
  await page.evaluate((bytes) => {
    const dt = new DataTransfer();
    dt.items.add(new File([new Uint8Array(bytes)], "shot.png", { type: "image/png" }));
    document.getElementById("sesstext").dispatchEvent(new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: dt }));
  }, [...PNG]);
  await page.setInputFiles("#sessfile", { name: "dump.bin", mimeType: "application/octet-stream", buffer: Buffer.from([0, 159, 255]) });
  check(`AC-204 ${at} a pasted image and an added binary file each become a chip, the text left as typed`,
    await until(async () => (await dock(page)).files.length === 2) && JSON.stringify(f.uploads) === '["shot.png","dump.bin"]' && (await dock(page)).text === "Fix what", JSON.stringify(await dock(page)));
  await page.focus("#sesstext");
  await page.keyboard.press("End");
  await dropTwo(page);
  d = await dock(page);
  check(`AC-204 ${at} two task cards dropped: their IDs at the caret, a space before; neither runs`, d.text === "Fix what API-1 API-2 " && f.posts.length === 0, JSON.stringify(d));
  await page.keyboard.press("Escape");
  await page.reload();
  await page.waitForSelector('#agentbar .agentcard[data-agent="a2"]', { state: "attached" });
  await page.click('#agentbar .agentcard[data-agent="a2"] .agsess');
  d = await dock(page);
  check(`AC-204 ${at} Esc closes it; the text stays in this browser across a reload, the files do not`, d.open && d.text === "Fix what API-1 API-2 " && d.files.length === 0, JSON.stringify(d));
  await page.setInputFiles("#sessfile", { name: "shot.png", mimeType: "image/png", buffer: PNG });
  await until(async () => (await dock(page)).files.length === 1);
  await page.press("#sesstext", "Meta+Enter");
  await folds(f, at, 1, "⌘Enter");
  f.release();
  check(`AC-204 ${at} ⌘Enter asks terminumd for that agent's session with the prompt and the file's id, then shows the session`,
    await until(() => f.posts.length === 1) && f.posts[0].agent === "a2" && f.posts[0].prompt === "Fix what API-1 API-2" && f.posts[0].files.length === 1
    && await until(() => page.url().includes("/sessions/")), JSON.stringify({ posts: f.posts, url: page.url() }));
  check(`AC-204 ${at} the draft is cleared once it went in`, await page.evaluate(() => !localStorage.getItem("tm.sessiondraft")), "");
  check(`no page error ${at}`, f.errors.length === 0, f.errors.join("; "));
  await f.close();

  // a miss: the session stands, the dock says so with its link and keeps the draft
  f = await board(browser, WIDE, { held: true, answer: () => ({ key: "tmux:local:terminum:%9", run: "r2", typed: false, why: "its input box did not come in 60 s" }) });
  ({ page } = f);
  await page.click('#agentbar .agentcard[data-agent="a1"] .agsess');
  await page.fill("#sesstext", "Missed prompt");
  await page.click("#sessstart");
  await folds(f, at, 0, "Start");
  f.release();
  await until(async () => /not typed/.test((await dock(page)).error));
  d = await dock(page);
  check(`AC-204 ${at} a prompt not typed: the dock opens again for that agent, said with the why and a link to the session, the draft kept, the keys free, the page stays`,
    /not typed: its input box did not come in 60 s/.test(d.error) && d.open && d.agent === "Opus reviewer" && d.focus === "sesstext" && d.text === "Missed prompt" && free(d) && !page.url().includes("/sessions/")
    && await page.isVisible('#sesserror a[href^="/sessions/"]'), JSON.stringify(d));
  check(`AC-204 ${at} after a miss Start waits for a change: a second press would open a second session`, (await dock(page)).start, JSON.stringify(await dock(page)));
  await page.click("#sessdiscard");
  await until(() => page.isVisible("#confirmbox[open]"));
  await page.click("#confirmok");
  check(`AC-204 ${at} Discard asks, then clears it`, await until(async () => (await dock(page)).text === "") && await page.evaluate(() => !localStorage.getItem("tm.sessiondraft")), JSON.stringify(await dock(page)));
  await page.click("#sessstart");
  check(`AC-204 ${at} Start with nothing typed asks for a session with no prompt (AC-142)`, await until(() => f.posts.length === 2) && !("prompt" in f.posts[1]) && f.posts[1].files.length === 0, JSON.stringify(f.posts));
  await f.close();

  await refused(browser, WIDE, at);

  // a project whose agents run on another host takes no file
  f = await board(browser, WIDE, { host: "devbox.example" });
  ({ page } = f);
  await page.click('#agentbar .agentcard[data-agent="a1"] .agsess');
  await page.setInputFiles("#sessfile", { name: "dump.bin", mimeType: "application/octet-stream", buffer: Buffer.from([1]) });
  check(`AC-204 ${at} a remote project's composer refuses files, said`, await until(async () => /only to a session on this Mac/.test((await dock(page)).error)) && f.uploads.length === 0, JSON.stringify(await dock(page)));
  await f.close();

  at = `(${name} 390×844)`;
  runningIn(`${name} 390×844`);
  f = await board(browser, PHONE);
  ({ page } = f);
  await page.click("#poolline");
  await page.click('#agentbar .agentcard[data-agent="a1"] .agsess');
  const phone = await page.evaluate(() => { const s = document.getElementById("sessstart").getBoundingClientRect(), k = document.querySelector("#agentbar .agsess").getBoundingClientRect(), d = document.getElementById("sesscompose").getBoundingClientRect();
    return { start: [Math.round(s.width), Math.round(s.height)], dock: Math.round(d.width), key: Math.round(k.height), font: getComputedStyle(document.getElementById("sesstext")).fontSize, fits: d.right <= innerWidth && document.documentElement.scrollWidth <= innerWidth }; });
  check(`AC-204 ${at} in the pool's sheet: the dock under the cards, Start full width and 44 px, the box 16 px (no zoom), nothing past the screen`,
    phone.start[1] >= 44 && phone.start[0] >= phone.dock - 2 * 16 - 1 && phone.font === "16px" && phone.fits, JSON.stringify(phone));
  check(`no page error ${at}`, f.errors.length === 0, f.errors.join("; "));
  await f.close();
  await refused(browser, PHONE, at);
}

/** terminumd refuses the start: the dock opens again with the draft, its file and terminumd's words; Start takes a press again. */
async function refused(browser, device, at) {
  const f = await board(browser, device, { held: true, status: 500, answer: () => ({ error: "the agent's command could not start" }) });
  const { page } = f;
  if (device === PHONE) await page.click("#poolline");
  await page.click('#agentbar .agentcard[data-agent="a1"] .agsess');
  await page.fill("#sesstext", "Refused prompt");
  await page.setInputFiles("#sessfile", { name: "shot.png", mimeType: "image/png", buffer: PNG });
  await until(async () => (await dock(page)).files.length === 1);
  await page.click("#sessstart");
  await folds(f, at, 0, "Start");
  const moved = device === WIDE;                   // wide: the user went on in Find tasks meanwhile; the phone: left it
  if (moved) await page.focus("#find");
  f.release();
  await until(async () => (await dock(page)).open);
  const d = await dock(page);
  check(`AC-204 ${at} a refused start opens the dock again for that agent: its draft, its file and terminumd's words, ${moved ? "the focus left in Find tasks" : "its box in focus"}, Start and the keys free`,
    d.open && /could not start/.test(d.error) && d.agent === "Opus reviewer" && d.text === "Refused prompt" && d.files.length === 1 && d.focus === (moved ? "find" : "sesstext") && !d.start && free(d)
    && !page.url().includes("/sessions/") && await page.evaluate(() => JSON.parse(localStorage.getItem("tm.sessiondraft") || "{}").text === "Refused prompt"), JSON.stringify(d));
  check(`no page error ${at}`, f.errors.length === 0, f.errors.join("; "));
  await f.close();
}
