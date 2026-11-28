// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// What the board's parts share: terminumd's routes, small DOM helpers, dialogs and files.
import * as signin from "./signin.js";
import { PROJECT_HUES, el, hueColor, iconButton, plural, textKey, when } from "./listparts.js";
import { renderers } from "./renderlib.js";
import { toast } from "./toast.js";
import { askConfirm } from "./dialogs.js";
import { dropLit } from "./droplit.js";
import { cardState } from "./cardstate.js";
import { openReport } from "./reportlink.js";

export const COLUMNS = ["TODO", "IN PROGRESS", "WAIT FOR HUMAN", "INTEGRATION", "FAILED", "DONE"];
export const SEVERITIES = ["P0", "P1", "P2", "P3"];
export const THIS_MAC = "This Mac";
export const AT_LIMIT = "at its limit: its queue";   // an agent with no room, in a list of agents: Run offers its queue (terminum-dispatcher.md AC-169)
export const DEFAULT = "default";                  // terminumd's Default project
/** A server's choices: none is this Mac. */
export const hostOptions = (hosts) => [["", THIS_MAC], ...hosts.map((h) => [h, h])];
/** An agent's color: its place in the projects' palette (terminum-dispatcher.md AC-138). */
export const agentColor = (a) => hueColor(PROJECT_HUES[(a.color ?? 0) % PROJECT_HUES.length]);
/** An agent's name; one no longer in the pool whose run kept none. */
export const agentLabel = (a) => a?.name || "Agent gone";
/** An estimate in dollars: "≈$12.30"; whole dollars past a thousand. */
export const dollars = (x) => `≈$${x >= 1000 ? Math.round(x).toLocaleString("en-US") : x.toFixed(2)}`;
export const $ = (id) => document.getElementById(id);

/** What a session's row cannot say (AC-136): an agent gone quiet with its task open, why it failed, a closed one. */
export function sessionLine(t) {
  const st = cardState(t);
  if (!st) return null;
  const e = el("span", `cstate ${st.cls}`, st.words);
  e.title = st.why ? `${st.words}: ${st.why}` : st.words;
  return e;
}

/** A column's tasks with Show subtasks on (terminum-kanban.md AC-112), as [task, the parent it is drawn under or ""]:
 *  each subtask right under its parent when its parent is in the same list, in their own order; one whose parent is
 *  elsewhere keeps its place. */
export function underParents(tasks) {
  const here = new Set(tasks.map((t) => t.id)), kids = new Map(), drawn = [];
  for (const t of tasks) if (here.has(t.parent)) (kids.get(t.parent) ?? kids.set(t.parent, []).get(t.parent)).push(t);
  for (const t of tasks) {
    if (here.has(t.parent)) continue;
    drawn.push([t, ""]);
    for (const k of kids.get(t.id) ?? []) drawn.push([k, t.id]);
  }
  return drawn;
}

/** What a refused sign-in does: nothing on the session page, which signs in by itself (a task's window opens
 *  there too: terminum-kanban.md AC-150); the board goes to /sessions/, whose form signs in (signInElsewhere). */
let refused = () => {};

/** The board's, the Message history's and the Dashboard's: with no sign-in it goes to /sessions/; what does that. */
export function signInElsewhere() {
  refused = () => { location.href = "/sessions/"; };
  signin.onRefused(refused);
  return refused;
}

/** A JSON route (a write always has a body: terminumd asks its type); a refusal throws terminumd's
 *  message, with its status. An empty answer is {}; one that cannot be read (cut off, as a navigation
 *  leaves a read in WebKit) throws, never stands for an empty one. */
export async function api(path, { method = "GET", body, asText = false, signal } = {}) {
  const sent = method === "GET" ? undefined : JSON.stringify(body ?? {});
  const r = await fetch(path, { method, headers: sent ? { "Content-Type": "application/json" } : undefined, body: sent, signal });
  if (asText && r.ok) return r.text();
  let m = {}, unread;
  try {
    const text = await r.text();
    if (text) m = JSON.parse(text);
  } catch (e) { unread = e; }
  if (r.status === 401) refused();
  if (!r.ok) {
    const e = new Error(m.error ?? `terminumd answered ${r.status}`);
    e.status = r.status;
    throw e;
  }
  if (unread) throw new Error(`the answer was cut off or not JSON (${unread.message})`);
  return m;
}

// made once: a format's making is the costly part, and a board says hundreds of times
const [STAMP, HOUR, DAY, DAY_YEAR] = [{ dateStyle: "medium", timeStyle: "short" }, { hour: "2-digit", minute: "2-digit" },
  { month: "short", day: "numeric" }, { month: "short", day: "numeric", year: "numeric" }].map((o) => new Intl.DateTimeFormat(undefined, o));

/** A date and time as the windows say them. */
export const stamp = (at) => STAMP.format(new Date(at * 1000));

/** A time as a conversation says it: today's by its hour, this year's with its day, an older one whole; `day`: a
 *  card's, an earlier day's without its hour (terminum-kanban.md AC-105). */
export function shortStamp(at, day = false) {
  const d = new Date(at * 1000), now = new Date(), thisYear = d.getFullYear() === now.getFullYear();
  if (d.toDateString() === now.toDateString()) return HOUR.format(d);
  if (day) return (thisYear ? DAY : DAY_YEAR).format(d);
  return thisYear ? when(at) : stamp(at);
}

/** `at` said in `words`, its whole date and time on hover. */
export function timeOf(at, words, cls = null) {
  const t = el("time", cls, words);
  t.title = stamp(at);
  t.dateTime = new Date(at * 1000).toISOString();
  return t;
}

/** "made …, changed …": when a task was made and last changed (AC-149). */
export const madeChanged = (t) => `made ${stamp(t.created_at)}${t.updated_at > t.created_at ? ` · changed ${stamp(t.updated_at)}` : ""}`;

/** A box in a status's color (AC-112). */
const statusClass = (status) => `stbox st-${status.toLowerCase().replaceAll(" ", "-")}`;

/** A task's status in words on its color; an archived task says so, in no status's color. */
export const statusTag = (t) => (t.archived_at ? el("span", "stbox word", "Archived") : el("span", `${statusClass(t.status)} word`, t.status));

/** A box in `status`'s color saying `text`, and `said` to a pointer and a screen reader; `press`: what a press does. */
function colorBox(status, text, said, press) {
  const b = el(press ? "button" : "span", statusClass(status), text);
  b.title = said;
  b.setAttribute("aria-label", said);
  if (press) {
    b.type = "button";
    b.addEventListener("click", (e) => { e.stopPropagation(); press(); });
  }
  return b;
}

/** A task as a small box: its ID on its status's color, the status in words too (AC-112). */
export const statusBox = (t, open) => colorBox(t.status, t.id, [t.id, t.status, t.title].filter(Boolean).join(" · "), open && (() => open(t.id)));

/** A parent's subtasks that need a human (AC-112, AC-117): a row of a box each for those FAILED and WAIT FOR HUMAN, the
 *  number of them on that status's color, a press opening the first; null when none. */
export function needBoxes(kids, open) {
  const boxes = [["FAILED", "failed"], ["WAIT FOR HUMAN", "waiting"]].flatMap(([status, word]) => {
    const of = kids.filter((k) => k.status === status && (status !== "WAIT FOR HUMAN" || !k.queued));
    if (!of.length) return [];
    const b = colorBox(status, `${of.length} ${word}`, `${plural(of.length, "subtask")} ${status}: ${of.map((k) => k.id).join(", ")}`, () => open(of[0].id));
    b.classList.add("word");
    return [b];
  });
  if (!boxes.length) return null;
  const row = el("div", "cneeds");
  row.append(...boxes);
  return row;
}

/** `parent` holding `nodes` in this order, moving only what must move: one that stays is never taken out, as a node
 *  taken out and put back between a press's down and up loses its click (Chromium). */
export function placeChildren(parent, nodes) {
  const kept = new Set(nodes);
  for (const c of [...parent.children]) if (!kept.has(c)) c.remove();
  nodes.forEach((n, i) => { if (parent.children[i] !== n) parent.insertBefore(n, parent.children[i] ?? null); });
}

/** A line of keys at a window's foot. */
export const row = (...kids) => { const r = el("div", "actions"); r.append(...kids.filter(Boolean)); return r; };

/** One short message, where the action was taken cannot hold it. */
export const say = (text) => toast(text, 4000);
/** The one project of `all` the board shows; with several or none, says why there is none to `act` on (terminum-kanban.md AC-139). */
export function oneProject(all, act) {
  if (all.length === 1) return all[0];
  say(all.length ? `Multiple projects are selected: select only one to ${act}.` : "Make a project first.");
  return null;
}

/** A text pressed ⌘Enter in goes where its button would. */
export const onSend = (area, go) => area.addEventListener("keydown", (e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); go(); } });

/** A queued RUN stays visible, but cannot replace the press already waiting (AC-122, AC-169). */
export function guardQueuedRun(key, waits) {
  key.disabled = !!waits;
  if (waits) key.title = `Already queued for ${waits}: Stop waiting before running it again`;
  return key;
}

/** A text key; a pointer's press leaves it unfocused (textKey). */
export function button(cls, text, onClick) {
  const b = textKey(el("button", cls, text));
  b.type = "button";
  if (onClick) b.addEventListener("click", onClick);
  return b;
}

/** A window's project when none is given: the board's only project shown, else the only one there is, else none. */
export const onlyProject = (shown, meta) => (shown.length === 1 ? shown[0] : meta.projects.length === 1 ? meta.projects[0].id : "");

/** The projects for a window's Project field: a choice of none first while none is chosen, so a task or schedule
 *  never lands in Default unasked (terminum-kanban.md AC-106, terminum-schedules.md AC-160). */
export const projectOptions = (meta, chosen) => [...(chosen ? [] : [["", "Choose a project"]]), ...meta.projects.map((p) => [p.id, p.name])];

/** `select`'s options, each [value, label]; `chosen` if it is one of them, else the first. */
export function fillSelect(select, options, chosen) {
  select.replaceChildren(...options.map(([value, label]) => { const o = el("option", null, label); o.value = value; return o; }));
  select.value = options.some(([value]) => value === chosen) ? chosen : options[0]?.[0] ?? "";
}

/** `project`'s own pool: the agents it can run (terminum-dispatcher.md AC-137). */
export const agentsOf = (meta, project) => meta.agents.filter((a) => a.project === project);

/** The words of each slot an agent keeps, `kept` as terminumd counts them ({slots, schedules, integration}): one ring
 *  "kept for 3 schedules" for each of its schedules' K slots, then one "kept for integration" (terminum-schedules.md AC-161). */
export function keptRings(kept) {
  const rings = Array(kept?.slots ?? 0).fill(`kept for ${plural(kept?.schedules ?? 0, "schedule")}`);
  return kept?.integration ? [...rings, "kept for integration"] : rings;
}

/** How many slots an agent keeps, `kept` as terminumd counts them. */
const keptCount = (kept) => (kept?.slots ?? 0) + (kept?.integration ? 1 : 0);

/** How many of `a`'s slots are taken, by terminumd's `counted` slots: its sessions and the slots it keeps; Auto-Dispatcher
 *  rounds and integration waits take none (terminum-dispatcher.md AC-169, AC-170, terminum-schedules.md AC-161). */
export const slotsUsed = (counted, a) => (counted.agents?.[a.id] ?? []).filter((s) => !s.kept && !s.round && !s.integration && !s.ended).length + keptCount(counted.kept?.[a.id]);

/** A project's `agents` for `select`, a busy one said so by `busy` and chosen all the same: a run waits for its
 *  room (terminum-dispatcher.md AC-158); the first of `wanted`, else the first. `none`: the label of a choice of
 *  no agent, first; "" in `wanted` picks it. */
export function fillAgents(select, agents, busy, wanted, none = null) {
  const pick = wanted.find((id) => (id === "" ? none : agents.some((a) => a.id === id))) ?? agents[0]?.id ?? "";
  fillSelect(select, [...(none ? [["", none]] : []), ...agents.map((a) => [a.id, busy(a) ? `${a.name} (${busy(a)})` : a.name])], pick);
}

/** The agent a first run starts on with nothing asked: the first of `wanted` (ids) in `agents`, else the pool's only
 *  one; none known: null, and RUN's window asks (terminum-dispatcher.md AC-121). */
export const knownAgent = (agents, wanted) => wanted.map((id) => agents.find((a) => a.id === id)).find(Boolean) ?? (agents.length === 1 ? agents[0] : null);

/** The agent a task is on, in its color (terminum-dispatcher.md AC-138). */
export function agentChip(agent) {
  const a = el("span", "cagent", agent.name);
  a.style.setProperty("--ag", agentColor(agent));
  a.title = `${agent.name} does it, or did`;
  return a;
}

/** The sessions a task can be linked to. */
export const sessionOptions = (meta) => [["", "None"], ...meta.sessions.map((s) => [s.key, s.title || s.key])];

/** A dialog's line of error: `text`, or hidden with none. */
export const showError = (box, text) => { box.textContent = text ?? ""; box.hidden = !text; };

/** An empty, hidden line of error that screen readers announce once showError fills it. */
export const alertLine = () => {
  const box = el("p", "merr");
  box.setAttribute("role", "alert");
  box.hidden = true;
  return box;
};

/** Whether what is dragged is files. */
export const hasFiles = (e) => e.dataTransfer?.types.includes("Files");

/** Files pasted into `area` or dropped on `zone` go to `add`; `show(on)` says one is over it. */
export function onFiles(area, zone, add, show = () => {}) {
  const lit = dropLit(show);
  area.addEventListener("paste", (e) => { const list = [...(e.clipboardData?.files ?? [])]; if (list.length) { e.preventDefault(); add(list); } });
  zone.addEventListener("dragover", (e) => { if (hasFiles(e)) { e.preventDefault(); lit(true); } });
  zone.addEventListener("dragleave", (e) => { if (!zone.contains(e.relatedTarget)) lit(false); });
  zone.addEventListener("drop", (e) => {
    const list = [...(e.dataTransfer?.files ?? [])];
    lit(false);
    if (list.length) { e.preventDefault(); add(list); }
  });
}

/** A window's files as chips in `list`, each with its × (`remove(f)`). */
export function fileChips(list, files, remove) {
  list.replaceChildren(...files.map((f) => {
    const li = el("li", null, f.name);
    li.append(iconButton("icon", "x", `Remove ${f.name}`, "Remove this file", () => remove(f)));
    return li;
  }));
}

/** Keeps a file with terminumd: its {id, name, bytes, mime}. Over `most` bytes is said before it is sent. */
export async function upload(file, most) {
  if (file.size > most) throw new Error(`${file.name} is over ${Math.round(most / 2 ** 20)} MB`);
  const r = await fetch(`/api/task-files?name=${encodeURIComponent(file.name)}`, { method: "POST", headers: { "Content-Type": "application/octet-stream" }, body: file });
  const m = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(m.error ?? `${file.name}: terminumd answered ${r.status}`);
  return m;
}

const fileUrl = (id) => `/api/task-files/${id}`;

/** Markdown of a kept file: terminumd calls a file image/* only when it shows as one; anything else links. */
export const fileMarkdown = (f) => `${f.mime?.startsWith("image/") ? "!" : ""}[${f.name.replace(/[[\]]/g, "")}](${fileUrl(f.id)})`;
/** `text` with file `f`'s link on a line of its own at its end. */
export const withFileLink = (text, f) => `${text}${text && !text.endsWith("\n") ? "\n" : ""}${fileMarkdown(f)}\n`;
/** `text` without the links to files `ids` (32 hex digits, terminumd's), and the name each first showed. */
export function withoutFileLinks(text, ids) {
  const names = [];
  for (const id of ids) {
    let name = null;
    text = text.replace(new RegExp(`!?\\[([^\\]]*)\\]\\(${fileUrl(id)}\\)\\n?`, "g"), (_, shown) => { name ??= shown; return ""; });
    names.push(name);
  }
  return { text, names };
}

/** Of a draft's files, those terminumd no longer keeps: a file joined to no task goes after 7 days (AC-106, AC-108).
 *  Only its 404 says so: any other answer, or none, keeps the file in the draft. */
export async function goneFiles(ids) {
  const gone = await Promise.all(ids.map((id) => fetch(fileUrl(id), { method: "HEAD" }).then((r) => r.status === 404, () => false)));
  return ids.filter((_, i) => gone[i]);
}

/** terminumd's refusal of a draft's file the sweep took (tasks/files.rs FILE_GONE): only then is a refused draft
 *  asked of its files again. */
export const FILE_GONE = "A file of this window is gone: add it again";
/** Says refusal `e` by `tell`, unless it is FILE_GONE and `prune()` named the files let go: the rest stay. */
export async function sayRefused(e, prune, tell = say) {
  if (e.message !== FILE_GONE || !await prune()) tell(e.message);
}
/** What a window says of a draft's files that are gone. */
export const goneSaid = (names) => `${names.join(", ")}: gone, this draft was not saved for 7 days. Drop ${names.length > 1 ? "them" : "it"} again to keep ${names.length > 1 ? "them" : "it"}.`;

/** Saves a file through a fetch with the sign-in, never its address (terminum.md AC-09). */
export async function download(id, name) {
  const r = await fetch(fileUrl(id));
  if (!r.ok) throw new Error(`${name}: terminumd answered ${r.status}`);
  downloadBlob(await r.blob(), name);
}

export function downloadBlob(blob, name) {
  const url = URL.createObjectURL(blob);
  const a = el("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 60000);
}

/** Markdown by the Panel's rules (raw HTML off, javascript: links dropped, remote images not
 *  loaded); a task's own pictures are fetched with the sign-in, its files download. */
export async function renderInto(box, text, { comment = false } = {}) {
  box.textContent = text;
  try { box.innerHTML = (await renderers()).taskMarkdownHtml(text, comment); } catch { return; }
  wireRendered(box);
}

/** A report's body as the Files viewer shows a document (terminum-reports.md AC-268): its code highlighted, its
 *  Mermaid fences drawn, an HTML document in a frame without scripts; in the viewer's look (/fb/renderers.css), loaded
 *  with the first report shown. */
export async function renderReportInto(box, text) {
  box.textContent = text;
  try {
    const [lib] = await Promise.all([renderers(), docLook()]);
    if (lib.reportInto(box, text) === "html") { wireFrame(box.querySelector("iframe")); return; }
  } catch { box.textContent = text; return; }
  wireRendered(box);
}

let looked = null;
const docLook = () => (looked ??= new Promise((done) => {
  const link = Object.assign(document.createElement("link"), { rel: "stylesheet", href: "/fb/renderers.css" });
  link.onload = link.onerror = done;              // unstyled rather than never: the body shows either way
  document.head.append(link);
}));

/** Rendered words wired: their own pictures fetched, report IDs and links made keys. */
function wireRendered(box) {
  ownPictures(box);
  linkReports(box);
  for (const a of box.querySelectorAll("a")) wireLink(a, box);
}

/** `#anchor` scrolled to in `scope`, by its id or name, as the Files viewer does (ui/src/docview.ts scrollToAnchor). */
function scrollToAnchor(scope, href) {
  const id = CSS.escape(decodeURIComponent(href.slice(1)));
  scope.querySelector(`[id="${id}"], [name="${id}"]`)?.scrollIntoView({ block: "start" });
}

/** The pictures of a task's or report's own files (data-raw), fetched with the sign-in. */
function ownPictures(root) {
  for (const img of root.querySelectorAll("img[data-raw]")) {
    signin.blobUrl(img.dataset.raw).then((got) => {
      if (!got.url) return;
      const done = () => URL.revokeObjectURL(got.url);   // a picture decoded needs its address no more
      img.addEventListener("load", done, { once: true });
      img.addEventListener("error", done, { once: true });
      img.src = got.url;
    });
  }
}

/** A link: its own file downloads, an anchor scrolls in `scope`, any other opens in a new tab. */
function wireLink(a, scope) {
  const href = a.getAttribute("href") ?? "";
  const id = /^\/api\/task-files\/([0-9a-f]{32})$/.exec(href)?.[1];
  if (id) a.addEventListener("click", (e) => { e.preventDefault(); download(id, a.textContent).catch((err) => say(err.message)); });
  else if (href.startsWith("#")) a.addEventListener("click", (e) => { e.preventDefault(); scrollToAnchor(scope, href); });
  else { a.target = "_blank"; a.rel = "noopener noreferrer"; }
}

/** A report's HTML document in its frame: its own pictures, its links as the page's, as tall as it is. */
function wireFrame(frame) {
  const fit = () => { const d = frame.contentDocument; if (d?.documentElement) frame.style.height = `${d.documentElement.scrollHeight}px`; };
  frame.addEventListener("load", () => {
    const doc = frame.contentDocument;
    if (!doc) return;
    ownPictures(doc);
    for (const img of doc.images) img.addEventListener("load", fit);
    doc.addEventListener("click", (e) => {
      const a = e.target.closest?.("a[href]");
      if (!a) return;
      e.preventDefault();
      const href = a.getAttribute("href");
      if (href.startsWith("#")) scrollToAnchor(doc, href);
      else if (/^(https?:|mailto:)/i.test(href)) window.open(href, "_blank", "noopener,noreferrer");
    });
    fit();
  });
}

/** A report's ID in rendered words, outside code and links, a key that opens it (terminum-reports.md AC-273). */
const REPORT_ID = /\bR_[A-Z][A-Z0-9]{0,7}-\d+\b/;
function linkReports(box) {
  const walk = document.createTreeWalker(box, NodeFilter.SHOW_TEXT, { acceptNode: (n) => (n.parentElement?.closest("a, code, pre, button") ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT) });
  const found = [];
  for (let n = walk.nextNode(); n; n = walk.nextNode()) if (REPORT_ID.test(n.data)) found.push(n);
  for (const n of found) {
    const parts = n.data.split(/(R_[A-Z][A-Z0-9]{0,7}-\d+)/);
    n.replaceWith(...parts.map((p, i) => (i % 2 ? button("linklike", p, () => openReport(p)) : p)));
  }
}

/** 1,234 as 1.2k, 4,800,000 as 4.8M. */
export function countOf(n) {
  return n >= 1e6 ? `${(n / 1e6).toFixed(1).replace(/\.0$/, "")}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1).replace(/\.0$/, "")}k` : String(n);
}

/** A task's text typed (a new task's, a schedule's, an open task's editor: terminum-kanban.md AC-106): fills `tabs`
 *  with its Write and Preview keys over its area and its rendered Markdown; the view. */
export function writePreview(tabs, area, shown) {
  tabs.className = "seg";
  tabs.setAttribute("role", "tablist");
  tabs.setAttribute("aria-label", "Text view");
  const [write, preview] = ["Write", "Preview"].map((word) => {
    const b = el("button", null, word);
    b.type = "button";
    b.setAttribute("role", "tab");
    return b;
  });
  tabs.replaceChildren(write, preview);
  const view = (on) => {
    write.setAttribute("aria-selected", String(!on));
    preview.setAttribute("aria-selected", String(on));
    area.hidden = on;
    shown.hidden = !on;
    if (on) renderInto(shown, area.value);
  };
  write.addEventListener("click", () => view(false));
  preview.addEventListener("click", () => view(true));
  view(false);
  return view;
}

/** What was typed is not lost by a slip of Esc or ×: with a change, the window asks first (terminum-kanban.md AC-133). */
export function guardCancel(dialog, unchanged) {
  dialog.addEventListener("cancel", async (e) => {
    if (e.defaultPrevented || unchanged()) return;   // refused while it is kept (dialogs.js oneAtATime, AC-165)
    e.preventDefault();
    if (await askConfirm("Discard what you typed?", "Discard")) dialog.close();
  });
}
