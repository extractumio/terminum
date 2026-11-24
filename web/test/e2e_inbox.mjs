// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Inbox and a report's pane (terminum-reports.md AC-267, AC-268, AC-269, AC-271, AC-273) against a fake terminumd,
// wide and on a phone: above the Sessions with its unread count; critical first and red, unread bold; a press opens the
// pane over the board's right part and reads the report after 2 seconds; Esc gives the columns back; U, Backspace and
// Undo; the row's menu; Find; a critical report's line; the session page opening the board with the report.
import { check, until } from "./check.mjs";
import { drawn, fakeBoard, openPage, ORIGIN, PHONE, serveRenderers, shot, THEME, WIDE } from "./fake.mjs";

const NOW = Math.floor(Date.now() / 1000);
const report = (id, extra = {}) => ({ id, project: "api", project_name: "API", project_color: 0, title: `Report ${id}`, summary: `What ${id} found`, critical: false, read: false,
  source: { task: "API-41", schedule: null, incident: null, name: "API-41" }, author_name: "Opus", role: "worker", session: null, created_at: NOW - 3600, updated_at: NOW - 3600,
  deleted_at: null, number: Number(id.split("-")[1]), ...extra });
const meta = () => ({ projects: [{ id: "api", name: "API", color: 0 }], users: [], sessions: [], hosts: [], theme: THEME, agents: [] });

/** The fake's reports and what the page asked of them. */
function reports(page) {
  const st = { replied: [], rows: [report("R_API-7"), report("R_API-6", { read: true }), report("R_API-8", { critical: true, title: "Build broken", created_at: NOW - 7200 })], asked: [], rev: 1 };
  const unread = () => ({ projects: { api: st.rows.filter((r) => !r.read && !r.deleted_at).length }, schedules: {}, sessions: {} });
  const json = (r, data, status = 200) => r.fulfill({ status, contentType: "application/json", body: JSON.stringify(data) });
  page.route("**/api/reports**", (r) => {
    const req = r.request(), u = new URL(req.url()), parts = u.pathname.split("/").slice(3);
    st.asked.push(`${req.method()} ${u.pathname}`);
    if (!parts.length) {
      const deleted = u.searchParams.get("deleted") === "1";
      const rows = st.rows.filter((x) => !!x.deleted_at === deleted).sort((a, b) => (b.critical && !b.read) - (a.critical && !a.read) || b.created_at - a.created_at);
      return json(r, { reports: rows, more: false, next: rows.length, unread: unread() });
    }
    const row = st.rows.find((x) => x.id === decodeURIComponent(parts[0]));
    if (!row) return json(r, { error: `No report ${parts[0]}.` }, 400);
    if (req.method() === "GET") return json(r, { ...row, body: "## Findings\n\n- **serde** 1.0.200", notes: [{ id: "n1", at: NOW - 60, author_name: "Opus", author_kind: "agent", kind: "comment", text: "One more thing, as R_API-6 said" }], files: [], follows: null, follow_ups: [],
      replyable: !row.critical, why_not: row.critical ? `${row.id} was made in a session terminum did not start: answer in that session.` : undefined, answering: st.replied.includes(row.id) ? "Opus" : null });
    if (parts[1] === "reply") {
      const text = JSON.parse(req.postData() ?? "{}").text;
      if (text.includes("busy")) return json(r, { error: "API-41 runs in session %8 at a human's RUN." }, 400);   // RUN's own refusal (AC-122)
      st.replied.push(row.id); st.rev++;
      return json(r, { task: "API-41" });
    }
    if (req.method() === "DELETE") row.deleted_at = NOW;
    else if (parts[1] === "read") row.read = true;
    else if (parts[1] === "unread") row.read = false;
    else if (parts[1] === "restore") row.deleted_at = null;
    st.rev++;
    return json(r, { ok: true, changed: true });
  });
  st.marks = (say, extra = {}) => say({ t: "taskmarks", marks: {}, counts: {}, waiting: 0, failed: 0, lines: [],
    reports: { rev: String(st.rev), unread: unread(), sessions: { "tmux:local:terminum:%1": st.rows.filter((r) => !r.read && !r.deleted_at).map((r) => r.id) } }, ...extra });
  return st;
}

const look = (page) => page.evaluate(() => {
  const box = (e) => { const r = e?.getBoundingClientRect(); return r && r.width > 0 ? { t: Math.round(r.top), b: Math.round(r.bottom), l: Math.round(r.left), r: Math.round(r.right) } : null; };
  const rows = [...document.querySelectorAll(".inboxrow")];
  return { inbox: box(document.getElementById("inbox")), sessions: box(document.getElementById("sessions")), count: document.querySelector(".inboxcount")?.textContent,
    rows: rows.map((r) => ({ id: r.dataset.id, unread: r.classList.contains("unread"), critical: r.classList.contains("critical"), weight: getComputedStyle(r.querySelector(".rtitle")).fontWeight })),
    pane: box(document.getElementById("reportpane")), board: box(document.getElementById("board")), pool: box(document.getElementById("agentbar")),
    paneTitle: document.querySelector(".rptitle")?.textContent, body: document.querySelector(".rpmarkdown strong")?.textContent, docW: document.documentElement.scrollWidth, win: innerWidth };
});

async function onBoard(browser, device, w) {
  const f = await openPage(browser, { device, secret: "a".repeat(64), stored: "a".repeat(64) });
  const { page } = f;
  await drawn(page);
  await fakeBoard(page, { meta, tasks: () => [], counts: {} });
  await serveRenderers(page);
  const st = reports(page);
  await page.goto(`${ORIGIN}/kanban/?project=api`);
  await page.waitForFunction(() => document.body.dataset.socket === "open");
  st.marks(f.say);
  const narrow = w === "390×844";
  if (narrow) await page.click("#navbtn");
  await page.waitForSelector(".inboxrow");
  let g = await look(page);
  check(`AC-267 ${w}: the Inbox stands above the Sessions with its unread count`, g.inbox && g.sessions && g.inbox.b <= g.sessions.t + 1 && g.count === "2", JSON.stringify(g));
  check(`AC-267 ${w}: critical unread first, unread rows bold, a read one plain`, g.rows.map((r) => r.id).join() === "R_API-8,R_API-7,R_API-6"
    && g.rows[0].critical && g.rows[1].unread && Number(g.rows[1].weight) > Number(g.rows[2].weight), JSON.stringify(g.rows));
  await shot(page, `inbox-${w}`);
  // a press: the pane over the board's right part; read after 2 seconds
  await page.click('.inboxrow[data-id="R_API-7"] .report');
  await page.waitForSelector("#reportpane:not([hidden]) .rptitle", { timeout: 8000 });
  await until(async () => (await look(page)).body === "serde", 4000);
  g = await look(page);
  const over = narrow ? g.pane.t <= (g.board?.t ?? g.pane.t) + 1 : g.pane.t <= (g.pool ?? g.board).t + 1 && g.pane.t >= 40 && g.pane.l <= g.board.l + 1 && g.pane.r >= g.board.r - 1;
  check(`AC-268 ${w}: the report's pane covers the board's right part, its address names it`, over && new URL(page.url()).searchParams.get("report") === "R_API-7" && g.docW <= g.win, JSON.stringify(g));
  const sheetOpen = () => page.evaluate(() => document.getElementById("app").classList.contains("nav-open"));
  if (narrow) check(`AC-268 ${w}: the list's sheet closes when a report opens`, !await sheetOpen());
  check(`AC-268 ${w}: its body by the Markdown rules, its comments below`, g.body === "serde" && await page.isVisible(".rpnotes li"), JSON.stringify(g));
  // a report's ID in a comment opens it (AC-273)
  await page.click('.rpnotes .linklike:has-text("R_API-6")');
  check(`AC-273 ${w}: a report's ID in a comment is a link that opens it`, await until(async () => (await look(page)).paneTitle === "Report R_API-6", 3000), (await look(page)).paneTitle);
  if (narrow) await page.click("#navbtn");          // the list's sheet over the pane, as a phone shows it
  await page.click('.inboxrow[data-id="R_API-7"] .report');
  await page.waitForFunction(() => document.querySelector(".rptitle")?.textContent === "Report R_API-7");
  check(`AC-269 ${w}: not read before 2 seconds`, !st.asked.includes("POST /api/reports/R_API-7/read"), st.asked.join());
  await shot(page, `inbox-pane-${w}`);
  check(`AC-269 ${w}: read once open 2 seconds`, await until(() => st.asked.includes("POST /api/reports/R_API-7/read"), 4000), st.asked.join());
  // AC-270: the reply, refused with RUN's words and kept in its field, then sent; its agent answering
  await page.fill(".rpreply", "Which break the build? busy");
  await page.click('.rpfoot button:has-text("Send")');
  check(`AC-270 ${w}: a refused reply says RUN's words and stays in its field`, await until(async () => (await page.textContent(".rpsaid")) === "API-41 runs in session %8 at a human's RUN." && (await page.inputValue(".rpreply")).endsWith("busy"), 3000));
  await page.fill(".rpreply", "Which break the build?");
  await page.keyboard.press(process.platform === "darwin" ? "Meta+Enter" : "Control+Enter");
  check(`AC-270 ${w}: ⌘Enter sends it; the pane says its agent is answering`, await until(async () => st.replied.includes("R_API-7") && (await page.textContent(".rpanswering").catch(() => "")) === "Opus is answering", 4000));
  await page.keyboard.press("Escape");
  check(`AC-268 ${w}: Esc gives the columns back and the address loses the report`, await until(async () => !await page.isVisible("#reportpane") && !page.url().includes("report="), 2000), page.url());
  // keys on a row: U, Backspace, Undo
  st.marks(f.say);
  if (narrow && !await sheetOpen()) await page.click("#navbtn");
  await page.waitForSelector('.inboxrow[data-id="R_API-6"]');
  await page.focus('.inboxrow[data-id="R_API-6"] .report');
  await page.keyboard.press("u");
  check(`AC-269 ${w}: U marks a read report unread`, await until(() => st.asked.includes("POST /api/reports/R_API-6/unread"), 2000), st.asked.join());
  await page.focus('.inboxrow[data-id="R_API-6"] .report');
  await page.keyboard.press("Backspace");
  // Undo pressed as soon as it shows: it stands 8 seconds, which a loaded machine's checks could outlast
  const offered = await page.waitForSelector(".inboxundo button", { timeout: 5000 }).then(() => true, () => false);
  const gone = !await page.isVisible('.inboxrow[data-id="R_API-6"]');
  if (offered) await page.click(".inboxundo button", { timeout: 3000 }).catch(() => {});
  check(`AC-269 ${w}: Backspace deletes it at once, Undo offered`, offered && gone && st.asked.includes("DELETE /api/reports/R_API-6"), st.asked.join());
  check(`AC-269 ${w}: Undo restores it`, await until(async () => st.asked.includes("POST /api/reports/R_API-6/restore") && await page.isVisible('.inboxrow[data-id="R_API-6"]'), 2000), st.asked.join());
  // every action has a button (CLAUDE.md §1): the row's delete
  await page.hover('.inboxrow[data-id="R_API-7"]').catch(() => {});
  check(`AC-267 ${w}: a row's delete and read keys are reachable`, await page.isVisible('.inboxrow[data-id="R_API-7"] .rdelete') || await page.isVisible('.inboxrow[data-id="R_API-7"] .rread'));
  if (!narrow) {
    await page.click('.inboxrow[data-id="R_API-7"] .report', { button: "right" });
    const items = await page.$$eval("#reportmenu[open] button", (bs) => bs.map((b) => b.textContent));
    check(`AC-267 ${w}: a right click offers Open, Mark, Open source task, Copy link, Delete`, ["Open", "Mark as unread", "Open source task API-41", "Copy link", "Delete"].every((x) => items.includes(x)), items.join());
    await page.keyboard.press("Escape");
    // Find filters the Inbox too (AC-273)
    await page.fill("#find", "broken");
    check(`AC-273 ${w}: Find finds reports by their words`, await until(async () => (await page.$$eval(".inboxrow", (rs) => rs.map((r) => r.dataset.id))).join() === "R_API-8", 3000));
    await page.fill("#find", "");
    // a critical report's line opens it (AC-271)
    st.marks(f.say, { lines: [{ id: "R_API-8", project: "api", failed: true, words: "R_API-8 critical: Build broken", key: "report|R_API-8", report: true }] });
    await page.waitForSelector('.taskline[data-task="R_API-8"]');
    await page.click('.taskline[data-task="R_API-8"] .go');
    check(`AC-271 ${w}: a critical report's line opens its pane`, await until(async () => (await look(page)).paneTitle?.includes("Build broken"), 3000));
    check(`AC-270 ${w}: a report of no task Dispatcher ran has no reply field: where to answer instead`, !await page.isVisible(".rpreply")
      && (await page.textContent(".rpwhy")).startsWith("R_API-8 was made in a session terminum did not start: answer in that session."));
    await page.click('.taskline[data-task="R_API-8"] .x');
    check(`AC-271 ${w}: its × closes the line by the report's route`, await until(() => st.asked.includes("POST /api/reports/R_API-8/seen"), 2000), st.asked.join());
  }
  // empty
  st.rows = [];
  st.rev++;
  st.marks(f.say);
  check(`AC-267 ${w}: an empty Inbox says so`, await until(async () => (await page.textContent(".inboxnone").catch(() => "")) === "No reports" && await page.isVisible(".inboxnone"), 3000));
  await f.close();
}

async function fromSessions(browser, w, device) {
  const f = await openPage(browser, { device, secret: "a".repeat(64), stored: "a".repeat(64) });
  const { page } = f;
  await drawn(page);
  const st = reports(page);
  st.marks(f.say);
  if (w === "390×844") await page.click("#navbtn");
  await page.waitForSelector(".inboxrow");
  // the session that made unread reports wears an envelope with their count (AC-273)
  const envelope = await page.textContent('#sessions .item[data-id="tmux:local:terminum:%1"] .reportmark').catch(() => null);
  check(`AC-273 ${w}: a session row shows its unread reports' count`, envelope === "2", String(envelope));
  await Promise.all([page.waitForURL(/\/kanban\/\?report=R_API-8/, { timeout: 5000 }).catch(() => {}), page.click('.inboxrow[data-id="R_API-8"] .report')]);
  check(`AC-268 ${w}: on the session page a report opens the board with its pane`, page.url().endsWith("/kanban/?report=R_API-8"), page.url());
  await f.close();
}

export async function webInbox(browser) {
  for (const [w, device] of [["1400×900", WIDE], ["390×844", PHONE]]) {
    await onBoard(browser, device, w);
    await fromSessions(browser, w, device);
  }
}
