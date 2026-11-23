// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Inbox (terminum-reports.md AC-267, AC-269): reports above the Sessions in the session list, on the session page,
// the board and a phone's sheet; unread in bold, critical in red and first; it follows the board's project choice and
// Find. A press opens the report's pane over the board (reportpane.js), on the session page by going there. Keys, a
// right-click or long-press menu and each row's buttons read, unread and delete; a delete is undone from its toast.
import { icon } from "./icons.js";
import { el, iconButton, plural, projectInk, read, write } from "./listparts.js";
import { api } from "./kanbanutil.js";

const SHOWN = 8;                                   // rows before "Show all"
const FOLDED = "inbox.folded";
const CHOSEN = "board.project";                    // the board's project choice, this browser's (kanbanfilters.js)
const UNDO_MS = 8000;
export const REPORT_EVENT = "tm-report";           // reportpane.js opens the report this names
export const CHANGED_EVENT = "tm-reports";         // the reports changed: an open pane reads its report again

/** The board's address of report `id`. */
export const reportUrl = (id) => `/kanban/?report=${encodeURIComponent(id)}`;

/** Opens report `id`: its pane on the board, else the board with it (AC-268). */
export function openReport(id) {
  if (!document.body.classList.contains("board-page")) { location.href = reportUrl(id); return; }
  if (matchMedia("(max-width: 860px)").matches) document.getElementById("drawerclose")?.click();   // a phone's list sheet gives way
  dispatchEvent(new CustomEvent(REPORT_EVENT, { detail: id }));
}

/** "now", "5 min", "3 h", "2 d": how old. */
export function age(at, now = Date.now() / 1000) {
  const s = Math.max(0, now - at);
  return s < 60 ? "now" : s < 3600 ? `${Math.floor(s / 60)} min` : s < 86400 ? `${Math.floor(s / 3600)} h` : `${Math.floor(s / 86400)} d`;
}

/** The projects the Inbox shows: the board's choice in its address, else this browser's last one; none: all. */
function chosen() {
  const named = new URLSearchParams(location.search).get("project");
  const list = named !== null ? named.split(",") : [].concat(read(CHOSEN) ?? []);
  return list.filter(Boolean);
}

/** The board's Find, which finds reports too (AC-273). */
const findWords = () => (new URLSearchParams(location.search).get("q") ?? "").trim().toLowerCase();

const st = { rows: [], more: false, unread: {}, rev: null, deleted: false, all: false, folded: !!read(FOLDED), key: "", undo: null, focus: null };
let box = null;

/** terminumd's `taskmarks`: the Inbox's revision and unread counts; a moved revision reads the list again. */
export function inboxMarks(m) {
  const r = m.reports;
  if (!r || !box) return;
  st.unread = r.unread ?? {};
  if (r.rev === st.rev) { drawHead(); return; }
  if (st.rev !== null) dispatchEvent(new CustomEvent(CHANGED_EVENT));
  st.rev = r.rev;
  load();
}

/** Reads the list of the projects shown, or the deleted ones (AC-269); `older`: the next page after the rows read. */
async function load(older = false) {
  const projects = chosen();
  const q = new URLSearchParams();
  if (projects.length === 1) q.set("project", projects[0]);
  if (st.deleted) q.set("deleted", "1");
  if (older) q.set("from", String(st.read ?? 0));
  try {
    const got = await api(`/api/reports?${q}`);
    st.read = got.next;
    const rows = got.reports.filter((r) => projects.length < 2 || projects.includes(r.project));
    st.rows = older ? [...st.rows, ...rows] : rows;
    st.more = got.more;
    st.unread = got.unread ?? st.unread;
    st.failed = "";
  } catch (e) { st.failed = e.message; }
  draw();
}

const unreadCount = () => { const p = chosen(); return Object.entries(st.unread.projects ?? {}).filter(([id]) => !p.length || p.includes(id)).reduce((n, [, c]) => n + c, 0); };

function drawHead() {
  const n = unreadCount(), count = box.querySelector(".inboxcount");
  count.textContent = n ? String(n) : "";
  count.title = n ? `${plural(n, "report")} unread` : "";
  box.querySelector(".inboxfold").setAttribute("aria-expanded", String(!st.folded));
  box.querySelector(".inboxtitle").textContent = st.deleted ? "Inbox · Deleted" : "Inbox";
  document.getElementById("navbtn")?.classList.toggle("hasunread", n > 0);   // a phone's list button (AC-267)
}

function shownRows() {
  const words = findWords();
  return words ? st.rows.filter((r) => [r.id, r.title, r.summary].some((x) => x.toLowerCase().includes(words))) : st.rows;
}

function draw() {
  if (!box) return;
  drawHead();
  const list = box.querySelector(".inboxrows"), rows = shownRows();
  const visible = st.all ? rows : rows.slice(0, SHOWN);
  list.replaceChildren(...visible.map(rowOf));
  list.hidden = st.folded;
  const more = box.querySelector(".inboxmore");
  more.hidden = st.folded || (st.all ? !st.more : rows.length <= SHOWN && !st.more);
  more.textContent = st.all ? "Older reports" : `Show all (${rows.length}${st.more ? "+" : ""})`;
  const none = box.querySelector(".inboxnone");
  none.hidden = st.folded || rows.length > 0;
  none.textContent = st.failed ? `The Inbox could not be read: ${st.failed}` : st.deleted ? "No deleted reports" : "No reports";
  if (st.focus) list.querySelector(`[data-id="${CSS.escape(st.focus)}"] .report`)?.focus();
}

function rowOf(r) {
  const li = el("li", `inboxrow${r.read ? "" : " unread"}${r.critical ? " critical" : ""}`);
  li.dataset.id = r.id;
  li.style.setProperty("--proj", projectInk({ color: r.project_color }));
  const b = el("button", "report");
  b.type = "button";
  b.title = `${r.id} · ${r.project_name}${r.read ? "" : " · unread"}`;
  const head = el("span", "rhead");
  if (r.critical) head.append(icon("triangle-alert", 13));
  head.append(el("span", "rtitle", r.title), el("span", "rage", st.deleted ? `deleted ${age(r.deleted_at)}` : age(r.created_at)));
  b.append(head, el("span", "rsum", r.summary || r.id), el("span", "rsource", `${r.id} · ${r.source.name}`));
  b.addEventListener("click", () => { st.focus = r.id; if (!st.deleted) openReport(r.id); });
  b.addEventListener("focus", () => { st.focus = r.id; });
  const acts = el("span", "racts");
  if (st.deleted) acts.append(iconButton("icon rrestore", "corner-left-up", `Restore ${r.id}`, "Restore", () => act(r, "restore")));
  else acts.append(
    iconButton("icon rread", r.read ? "mail" : "mail-open", r.read ? `Mark ${r.id} as unread` : `Mark ${r.id} as read`, r.read ? "Mark as unread (U)" : "Mark as read (U)", () => act(r, r.read ? "unread" : "read")),
    iconButton("icon rdelete", "x", `Delete ${r.id}`, "Delete (Backspace)", () => remove(r)));
  li.append(b, acts);
  return li;
}

/** read, unread, restore: terminumd's word, every page hearing it; a refusal said where it was pressed (AC-269). */
async function act(r, what) {
  try { await api(`/api/reports/${encodeURIComponent(r.id)}/${what}`, { method: "POST", body: {} }); }
  catch (e) { say(`${r.id}: ${e.message}`); }
  load();
}

/** Deleted at once, with Undo for 8 seconds (AC-269). */
async function remove(r) {
  st.rows = st.rows.filter((x) => x !== r);
  draw();
  try { await api(`/api/reports/${encodeURIComponent(r.id)}`, { method: "DELETE" }); }
  catch (e) { say(`${r.id}: ${e.message}`); load(); return; }
  dispatchEvent(new CustomEvent(CHANGED_EVENT, { detail: { deleted: r.id } }));
  offerUndo(r);
}

function offerUndo(r) {
  const bar = box.querySelector(".inboxundo");
  clearTimeout(st.undo);
  bar.replaceChildren(el("span", null, `${r.id} deleted`), Object.assign(el("button", "textkey", "Undo"), { type: "button", onclick: () => { bar.hidden = true; act(r, "restore"); } }));
  bar.hidden = false;
  st.undo = setTimeout(() => { bar.hidden = true; }, UNDO_MS);
}

/** What went wrong, in the Inbox's own line (the page's toast lets clicks through and may stand elsewhere). */
function say(text) {
  const bar = box.querySelector(".inboxundo");
  bar.replaceChildren(el("span", "inboxerr", text));
  bar.hidden = false;
  clearTimeout(st.undo);
  st.undo = setTimeout(() => { bar.hidden = true; }, UNDO_MS);
}

const rowData = (li) => st.rows.find((r) => r.id === li?.dataset.id);

/** ↑ ↓ move, Enter opens, Backspace or Delete deletes, U toggles read (AC-267). */
function onKey(e) {
  const li = e.target.closest?.(".inboxrow"), r = rowData(li);
  if (!r || e.target.closest(".racts") || e.metaKey || e.ctrlKey || e.altKey) return;
  const go = (next) => { if (next) { e.preventDefault(); next.querySelector(".report").focus(); } };
  if (e.key === "ArrowDown") go(li.nextElementSibling);
  else if (e.key === "ArrowUp") go(li.previousElementSibling);
  else if ((e.key === "Backspace" || e.key === "Delete") && !st.deleted) { e.preventDefault(); st.focus = li.nextElementSibling?.dataset.id ?? li.previousElementSibling?.dataset.id ?? null; remove(r); }
  else if ((e.key === "u" || e.key === "U") && !st.deleted) { e.preventDefault(); act(r, r.read ? "unread" : "read"); }
}

/** The row's menu, by a right click or a long press (AC-267). */
function menu(r, x, y) {
  closeMenu();
  const m = el("div", "box menurows inboxmenu");
  m.setAttribute("role", "menu");
  const item = (text, run) => { const b = el("button", null, text); b.type = "button"; b.setAttribute("role", "menuitem"); b.onclick = () => { closeMenu(); run(); }; m.append(b); };
  if (st.deleted) item("Restore", () => act(r, "restore"));
  else {
    item("Open", () => openReport(r.id));
    item("Open in new tab", () => window.open(reportUrl(r.id), "_blank", "noopener"));
    item(r.read ? "Mark as unread" : "Mark as read", () => act(r, r.read ? "unread" : "read"));
    if (r.source.task) item(`Open source task ${r.source.task}`, () => { location.href = `/kanban/${encodeURIComponent(r.source.task)}`; });
    item("Copy link", () => navigator.clipboard?.writeText(new URL(reportUrl(r.id), location.href).href).then(() => say("Link copied"), (e) => say(e.message)));
    item("Delete", () => remove(r));
  }
  document.body.append(m);
  const w = m.offsetWidth, h = m.offsetHeight;
  m.style.left = `${Math.max(8, Math.min(x, innerWidth - w - 8))}px`;
  m.style.top = `${Math.max(8, Math.min(y, innerHeight - h - 8))}px`;
  m.querySelector("button")?.focus();
}

function closeMenu() { document.querySelector(".inboxmenu")?.remove(); }

function mount(at) {
  box = at;
  box.classList.add("inbox");
  box.setAttribute("aria-label", "Inbox");
  const head = el("div", "inboxhead"), fold = el("button", "inboxfold");
  fold.type = "button";
  fold.append(icon("chevron-down", 12), el("span", "inboxtitle", "Inbox"), el("span", "inboxcount"));
  fold.addEventListener("click", () => { st.folded = !st.folded; write(FOLDED, st.folded); draw(); });
  const view = iconButton("icon inboxview", "history", "Deleted reports", "Deleted reports: restore one for 30 days", () => {
    st.deleted = !st.deleted; st.all = false; view.setAttribute("aria-pressed", String(st.deleted)); load();
  });
  view.setAttribute("aria-pressed", "false");
  head.append(fold, view);
  const rows = el("ol", "inboxrows"), more = el("button", "inboxmore textkey"), none = el("p", "inboxnone"), undo = el("div", "inboxundo");
  more.type = "button";
  more.addEventListener("click", () => { if (st.all) load(true); else { st.all = true; draw(); } });
  undo.hidden = true;
  undo.setAttribute("role", "status");
  rows.addEventListener("keydown", onKey);
  rows.addEventListener("contextmenu", (e) => { const r = rowData(e.target.closest(".inboxrow")); if (r) { e.preventDefault(); menu(r, e.clientX, e.clientY); } });
  let hold = 0;
  rows.addEventListener("pointerdown", (e) => {
    if (e.pointerType !== "touch") return;
    const r = rowData(e.target.closest(".inboxrow"));
    if (r) hold = setTimeout(() => { hold = 0; menu(r, e.clientX, e.clientY); }, 550);
  });
  for (const end of ["pointerup", "pointercancel", "pointermove"]) rows.addEventListener(end, () => clearTimeout(hold));
  addEventListener("click", (e) => { if (!e.target.closest?.(".inboxmenu")) closeMenu(); });
  addEventListener("keydown", (e) => { if (e.key === "Escape") closeMenu(); });
  box.replaceChildren(head, rows, more, none, undo);
  // the board's project and Find live in its address: a change there reads or filters again
  setInterval(() => {
    const key = `${chosen().join(",")}|${findWords()}`;
    if (key !== st.key && st.rev !== null) { const projects = st.key.split("|")[0]; st.key = key; if (projects !== chosen().join(",")) load(); else draw(); }
  }, 1000);
  // what made no change of a task (the Supervisor's digest) is read when the page shows again, and every 30 s; the
  // first read waits for terminumd's first marks, which come once the page is signed in
  const live = () => st.rev !== null && document.visibilityState === "visible";
  document.addEventListener("visibilitychange", () => { if (live()) load(); });
  setInterval(() => { if (live()) load(); }, 30000);
  st.key = `${chosen().join(",")}|${findWords()}`;
}

// a delete from the report's pane: Undo here, as for a row's
addEventListener(CHANGED_EVENT, (e) => { if (box && e.detail?.undo) { offerUndo({ id: e.detail.deleted }); load(); } });

const at = document.getElementById("inbox");
if (at) mount(at);
