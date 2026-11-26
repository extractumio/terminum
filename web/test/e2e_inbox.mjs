// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Inbox and a report's pane (terminum-reports.md AC-267, AC-268, AC-269, AC-271, AC-273) against a fake terminumd,
// wide and on a phone: above the Sessions with its unread count; critical first and red, unread bold; a press opens the
// pane over the board's right part and reads the report after 2 seconds; Esc gives the columns back; U, Backspace and
// Undo; the row's menu; Find; a critical report's line; each row's type and sender; a body shown as the Files viewer shows
// a document (code highlighted, Mermaid drawn, an HTML document in a frame without scripts, no file read but its own);
// the session page with no Inbox, its row's envelope opening the board with the report.
import { check, until } from "./check.mjs";
import { drawn, fakeBoard, openPage, ORIGIN, PHONE, serveRenderers, shot, THEME, WIDE } from "./fake.mjs";

const NOW = Math.floor(Date.now() / 1000);
const report = (id, extra = {}) => ({ id, project: "api", project_name: "API", project_color: 0, title: `Report ${id}`, summary: `What ${id} found`, critical: false, read: false,
  source: { task: "API-41", schedule: null, incident: null, name: "API-41", sender: "task", label: "API-41 · Opus" }, kind: "report", author_name: "Opus", role: "worker", session: null, created_at: NOW - 3600,
  updated_at: NOW - 3600, deleted_at: null, number: Number(id.split("-")[1]), ...extra });
/** Bodies an agent wrote: Markdown with code, a diagram and pictures it may not read; an HTML document with a script. */
const BODIES = {
  "R_API-7": "## Findings\n\n- **serde** 1.0.200\n\n```rust\nfn main() { let x = 1; }\n```\n\n```mermaid\ngraph TD\n  A --> B\n```\n\n![key](../../.ssh/id.png) ![home](/Users/alex/a.png)\n",
  "R_API-6": "<!doctype html><html><body><h1 id=\"t\">Weekly</h1><script>parent.pwned = 1</script><img src=\"/api/raw?path=/etc/hosts\" onerror=\"parent.pwned = 2\"><link rel=\"stylesheet\" href=\"x.css\"></body></html>",
};
const meta = () => ({ projects: [{ id: "api", name: "API", color: 0 }], users: [], sessions: [], hosts: [], theme: THEME, agents: [] });

/** The fake's reports and what the page asked of them. */
function reports(page) {
  const st = { replied: [], rows: [report("R_API-7", { kind: "attention", summary: "Two outdated", source: { task: "API-41", schedule: "SCH-1", incident: null, name: "Nightly deps", sender: "routine", label: "Routine Nightly deps · Opus" } }),
    report("R_API-6", { read: true }), report("R_API-8", { critical: true, title: "Build broken", created_at: NOW - 7200 })], asked: [], rev: 1, reads: [] };
  page.on("request", (q) => { if (/\/api\/raw|\.ssh|\/Users\/|\/x\.css/.test(q.url())) st.reads.push(q.url()); });
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
    if (req.method() === "GET") return json(r, { ...row, body: BODIES[row.id] ?? "## Findings\n\n- **serde** 1.0.200", notes: [{ id: "n1", at: NOW - 60, author_name: "Opus", author_kind: "agent", kind: "comment", text: "One more thing, as R_API-6 said" }], files: [], follows: null, follow_ups: [],
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
    paneTitle: document.querySelector(".rptitle > span:not(.rpcritical)")?.textContent, body: document.querySelector(".rpmarkdown strong")?.textContent, docW: document.documentElement.scrollWidth, win: innerWidth };
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
  const row7 = await page.evaluate(() => { const r = document.querySelector('.inboxrow[data-id="R_API-7"]'); return { kind: r.querySelector(".rkind")?.textContent, sum: r.querySelector(".rsumtext")?.textContent, from: r.querySelector(".rsource").textContent }; });
  check(`AC-267 ${w}: a row shows its type, its summary and its sender`, row7.kind === "attention" && row7.sum === "Two outdated" && row7.from === "From Routine Nightly deps · Opus · R_API-7", JSON.stringify(row7));
  // AC-267: a panel of its own, the board's gap between it and the Sessions' panel; an envelope in the project's color,
  // closed while unread, open once read; each type a tag whose words read on its fill
  const looks = await page.evaluate(() => {
    const rgb = (c) => { const m = c.match(/[\d.]+/g).map(Number); return c.startsWith("color(") ? m.map((v) => v * 255) : m; };
    const lum = (c) => rgb(c).slice(0, 3).map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }).reduce((s, v, i) => s + v * [0.2126, 0.7152, 0.0722][i], 0);
    const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)]; return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
    const inbox = getComputedStyle(document.getElementById("inbox"));
    const env = (id) => { const e = document.querySelector(`.inboxrow[data-id="${id}"] .renv`); return e && { open: e.innerHTML.includes("M21.2 8.4"), color: getComputedStyle(e).color }; };
    const kinds = [...document.querySelectorAll(".inboxrow .rkind")].map((k) => { const c = getComputedStyle(k); return { fill: c.backgroundColor, ratio: ratio(c.color, c.backgroundColor) }; });
    return { gap: parseFloat(inbox.borderBottomWidth), gapColor: inbox.borderBottomColor, board: getComputedStyle(document.body).backgroundColor,
      unread: env("R_API-7"), read: env("R_API-6"), dot: getComputedStyle(document.querySelector(".inboxrow .report"), "::before").content, kinds };
  });
  check(`AC-267 ${w}: the Inbox a panel of its own, the board's gap above the Sessions' panel`, looks.gap >= 8 && looks.gapColor === looks.board, JSON.stringify(looks));
  check(`AC-267 ${w}: an envelope in the project's color, closed while unread, open once read, no dot`, looks.unread && !looks.unread.open && looks.read?.open
    && looks.unread.color === looks.read.color && looks.unread.color !== "rgb(0, 0, 0)" && looks.dot === "none", JSON.stringify(looks));
  check(`AC-267 ${w}: each type a filled tag whose words read on it (4.5:1)`, looks.kinds.length === 3 && looks.kinds.every((k) => !/rgba\(0, 0, 0, 0\)|transparent/.test(k.fill) && k.ratio >= 4.5)
    && new Set(looks.kinds.map((k) => k.fill)).size >= 2, JSON.stringify(looks.kinds));
  await shot(page, `inbox-${w}`);
  // a press: the pane over the board's right part; read after 2 seconds
  await page.click('.inboxrow[data-id="R_API-7"] .report');
  await page.waitForSelector("#reportpane:not([hidden]) .rptitle", { timeout: 8000 });
  await until(async () => (await look(page)).body === "serde", 4000);
  g = await look(page);
  const over = narrow ? g.pane.t <= (g.board?.t ?? g.pane.t) + 1 : g.pane.t <= (g.pool ?? g.board).t + 1 && g.pane.t >= 40 && g.pane.l <= g.board.l + 1 && g.pane.r >= g.board.r - 1;
  check(`AC-268 ${w}: the report's pane covers the board's right part, its address names it`, over && new URL(page.url()).searchParams.get("report") === "R_API-7" && g.docW <= g.win, JSON.stringify(g));
  // the board's bar stays usable: its project list opens over the pane, never under it
  const onTop = await page.evaluate(() => {
    document.getElementById("fproject").open = true;   // a click on the bar would also close the pane
    const list = document.querySelector("#fproject > .box").getBoundingClientRect(), pane = document.getElementById("reportpane").getBoundingClientRect();
    const t = Math.max(list.top, pane.top), b = Math.min(list.bottom, pane.bottom), l = Math.max(list.left, pane.left), r = Math.min(list.right, pane.right);
    const hit = b > t && r > l ? document.elementFromPoint((l + r) / 2, (t + b) / 2) : null;
    document.getElementById("fproject").open = false;
    return { crosses: b > t && r > l, inList: !!hit?.closest("#fproject"), hit: hit?.className ?? null };
  });
  check(`AC-268 ${w}: the board's project list opens over the report's pane`, onTop.crosses && onTop.inList, JSON.stringify(onTop));
  const sheetOpen = () => page.evaluate(() => document.getElementById("app").classList.contains("nav-open"));
  if (narrow) check(`AC-268 ${w}: the list's sheet closes when a report opens`, !await sheetOpen());
  check(`AC-268 ${w}: its body by the Markdown rules, its comments below`, g.body === "serde" && await page.isVisible(".rpnotes li"), JSON.stringify(g));
  // as the Files viewer shows a document: its code highlighted, its diagram drawn, its typography the viewer's
  const doc = await until(() => page.evaluate(() => !!document.querySelector(".rpmarkdown pre code .tok-keyword") && !!document.querySelector(".rpmarkdown .md-diagram img.diagram")), 8000);
  const look2 = await page.evaluate(() => ({ cls: document.querySelector(".rpmarkdown").className, font: getComputedStyle(document.querySelector(".rpmarkdown")).lineHeight,
    lang: document.querySelector(".rpmarkdown pre[data-lang]")?.dataset.lang, imgs: document.querySelectorAll(".rpmarkdown img:not(.diagram)").length }));
  check(`AC-268 ${w}: the body as the Files viewer shows it: code highlighted, a Mermaid diagram drawn, the viewer's prose`, doc && look2.cls.includes("md-body") && look2.lang === "rust", JSON.stringify(look2));
  check(`AC-268 ${w}: pictures a report names outside its own files are never read`, look2.imgs === 0 && st.reads.length === 0, JSON.stringify({ ...look2, reads: st.reads }));
  check(`AC-265 ${w}: the pane names its type and its sender`, (await page.textContent(".rptitle .rkind")) === "attention" && (await page.textContent(".rpmeta")).startsWith("From Routine Nightly deps · Opus"),
    await page.textContent(".rpmeta"));
  // a report's ID in a comment opens it (AC-273)
  await page.click('.rpnotes .linklike:has-text("R_API-6")');
  check(`AC-273 ${w}: a report's ID in a comment is a link that opens it`, await until(async () => (await look(page)).paneTitle === "Report R_API-6", 3000), (await look(page)).paneTitle);
  const html = await until(() => page.evaluate(() => document.querySelector(".rpmarkdown iframe.html-doc")?.contentDocument?.querySelector("h1")?.textContent === "Weekly"), 4000);
  const frame = await page.evaluate(() => { const f = document.querySelector(".rpmarkdown iframe.html-doc"), d = f?.contentDocument;
    return { sandbox: f?.getAttribute("sandbox"), scripts: d?.querySelectorAll("script").length, handlers: d?.querySelectorAll("[onerror]").length, pwned: window.pwned ?? null }; });
  check(`AC-268 ${w}: an HTML body shows in a frame without scripts, its handlers and other files dropped`, html && frame.sandbox === "allow-same-origin" && frame.scripts === 0 && frame.handlers === 0
    && frame.pwned === null && st.reads.length === 0, JSON.stringify({ ...frame, reads: st.reads }));
  if (narrow) await page.click("#navbtn");          // the list's sheet over the pane, as a phone shows it
  await page.click('.inboxrow[data-id="R_API-7"] .report');
  await page.waitForFunction(() => document.querySelector(".rptitle > span:not(.rpcritical)")?.textContent === "Report R_API-7");
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
  // the session that made unread reports wears an envelope with their count (AC-273)
  const envelope = await page.waitForSelector('#sessions .item[data-id="tmux:local:terminum:%1"] .reportmark', { timeout: 5000 }).then((e) => e.textContent(), () => null);
  check(`AC-273 ${w}: a session row shows its unread reports' count`, envelope === "2", String(envelope));
  check(`AC-267 ${w}: the session page has no Inbox: it is the board's alone`, !await page.$("#inbox") && !await page.$(".inboxrow"));
  await Promise.all([page.waitForURL(/\/kanban\/\?report=R_API-7/, { timeout: 5000 }).catch(() => {}), page.click('#sessions .item[data-id="tmux:local:terminum:%1"] .reportmark')]);
  check(`AC-273 ${w}: on the session page a row's envelope opens the board with its newest report`, page.url().endsWith("/kanban/?report=R_API-7"), page.url());
  await f.close();
}

export async function webInbox(browser) {
  for (const [w, device] of [["1400×900", WIDE], ["390×844", PHONE]]) {
    await onBoard(browser, device, w);
    await fromSessions(browser, w, device);
  }
}
