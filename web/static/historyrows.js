// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Message history's rows (terminum.md AC-151): a message's head reads as a sentence that starts with
// its time: who sent it, to which agent or a shell, in which session, then its project and tasks; what
// was sent stands right under it with the words' hits marked; opened, its keys: Open in session, its
// tasks, Context, Copy, and Only this session or task. terminumd's text is only ever text: its
// highlight's marks (\u0002 … \u0003) become <mark> here, nothing of it is markup.
import { el } from "./listparts.js";
import { icon } from "./icons.js";
import { agentMark, agentName } from "./nav.js";
import { sessionUrl, taskUrl } from "./tasklink.js";
import { copyText } from "./touchcopy.js";
import { toast } from "./toast.js";
import { button } from "./kanbanutil.js";
import { api, dotOf, hmOf, inkOf, projectLabel, timeLabel } from "./msgpage.js";

const HIT = "\u0002", HIT_END = "\u0003";
/** Lines a closed row shows. */
const LINES = 3;
/** Tasks a row's head names; more are counted ("+2"). */
const TASKS = 2;
/** Who sent a message, by the store's user: its word, its hue's class (a task's comment's, taskwin.css), its icon, its title. */
export const SENDERS = { human: ["You", "by-human"], dispatcher: ["Dispatcher", "by-system", "bot", "Dispatcher typed it, for a task"], manager: ["Manager", "by-agent", "crown", "A project's manager typed it (send_to_session)"] };

/** `text` with terminumd's hit marks as <mark>s, into `box`; what goes before the first hit's line is left
 *  out when `short` and that line is past the first LINES (an ellipsis says so). */
export function marked(box, text, short) {
  let t = text;
  const first = t.indexOf(HIT);
  if (short && first >= 0) {
    const start = t.lastIndexOf("\n", first) + 1;
    if (t.slice(0, start).split("\n").length > LINES) t = `… ${t.slice(start)}`;
  }
  let hit = false;
  for (const part of t.split(/([\u0002\u0003])/)) {
    if (part === HIT) hit = true;
    else if (part === HIT_END) hit = false;
    else if (part) box.append(hit ? el("mark", "hit", part) : document.createTextNode(part));
  }
}

/** Where a message was sent: its folder (home kept short from its end), and its host when not this Mac. */
function where(it) {
  const path = (it.path ?? "").replace(/^\/Users\/[^/]+|^\/home\/[^/]+/, "~");
  return [it.host && it.host !== "local" ? it.host : "", path].filter(Boolean).join(": ");
}

/** A compact row of the context around a message: its time and first line. */
function contextRow(it, self) {
  const r = el("div", `ctxrow${self ? " self" : ""}`);
  r.append(el("span", "ctxtime", timeLabel(it.ms)), el("span", "ctxtext", (it.text.split("\n").find((l) => l.trim()) ?? "").slice(0, 200)));
  return r;
}

/** A message's head: its time (with its day when `dated`), who → to what, the session, its project and tasks. */
function headOf(it, p, dated) {
  const head = el("div", "mhead");
  const time = el("time", "mtime", dated ? timeLabel(it.ms) : hmOf(it.ms));
  time.dateTime = new Date(it.ms).toISOString();
  time.title = new Date(it.ms).toLocaleString();
  const [word, by, mark, title] = SENDERS[it.sender] ?? [it.sender, ""];
  const who = el("span", `mwho ${by}`);
  if (mark) who.append(icon(mark, 12));
  if (title) who.title = title;
  who.append(document.createTextNode(word));
  const to = el("span", "mto");
  to.append(agentMark({ agent: it.agent }), document.createTextNode(it.agent ? agentName(it.agent) ?? it.agent : "Shell"));
  head.append(time, who, el("span", "marrow", "→"), to);
  if (it.title) {
    const session = el("span", "msess", it.title);
    session.title = `Session: ${it.title}`;
    head.append(session);
  }
  const tags = el("span", "mtags");
  if (it.project) {
    const proj = el("span", "mproj");
    proj.title = "Project";
    proj.append(dotOf(inkOf(p)), el("span", "pname", it.project_name || projectLabel(p ?? { id: it.project })));
    tags.append(proj);
  }
  const tasks = it.tasks ?? [];
  for (const id of tasks.slice(0, TASKS)) tags.append(el("span", "mtask", id));
  if (tasks.length > TASKS) tags.append(Object.assign(el("span", "mtask", `+${tasks.length - TASKS}`), { title: tasks.slice(TASKS).join(", ") }));
  if (tags.childElementCount) head.append(tags);
  return head;
}

/** Message `it` as a row; `onPick(row, open)` when it is pressed (an opened row shows its keys); `only(filter)`
 *  narrows the list to its session or a task of it. */
export function messageRow(it, { projects, dated, onPick, only }) {
  const row = el("div", "msg");
  row.id = `m${it.id}`;
  row.setAttribute("role", "option");
  row.setAttribute("aria-selected", "false");
  row.dataset.id = it.id;
  const p = projects.get(it.project ?? "") ?? null;
  row.style.setProperty("--dot", inkOf(p));
  const text = el("div", `mtext${it.agent ? "" : " shell"}`);
  marked(text, it.hl ?? it.text, true);
  // cut at its start when long (right to left), the marks keeping its slashes in place (sent.js's way)
  const foot = el("div", "mmeta", `\u200e${[where(it), it.live ? "" : "session ended"].filter(Boolean).join(" · ")}\u200e`);
  if (!it.live) foot.classList.add("gone");
  row.append(headOf(it, p, dated), text, foot);
  row.addEventListener("click", (e) => { if (!e.target.closest("a, button, .ctx")) onPick(row, true); });
  row.open = () => openRow(row, it, text, only);
  return row;
}

/** The opened row: all its text, its keys, the keys that narrow the list to it, a place for its context. */
function openRow(row, it, text, only) {
  if (row.querySelector(".macts")) return;
  text.replaceChildren();
  marked(text, it.hl ?? it.text, false);
  row.classList.add("open");
  const acts = el("div", "macts");
  const go = el("a", "textkey primary", "Open in session");
  if (it.live) go.href = sessionUrl(it.key, `msg:${it.id}`);
  else { go.setAttribute("aria-disabled", "true"); go.title = "This session has ended"; }
  const box = el("div", "ctx");
  box.hidden = true;
  const ctx = Object.assign(button("textkey", "Context", () => context(box, it, ctx)), { title: "The messages sent before and after it in that pane" });
  const copy = button("textkey", "Copy", async () => toast((await copyText({ text: it.text })).said));
  acts.append(go, ...(it.tasks ?? []).map((id) => Object.assign(el("a", "textkey", `Open ${id}`), { href: taskUrl(id), title: "Its task on the board" })), ctx, copy);
  const find = el("div", "mfind");
  const session = Object.assign(button("textkey", "This session", () => only({ pane: it.key })), { title: "Every message sent in this session" });
  find.append(el("span", "", "Show only"), session, ...(it.tasks ?? []).map((id) => Object.assign(button("textkey", id, () => only({ task: id })), { title: `Every message sent for ${id}` })));
  row.append(acts, find, box);
  row.copy = () => copy.click();
  row.context = () => ctx.click();
  row.go = () => { if (it.live) location.href = go.href; else toast("This session has ended"); };
}

/** The messages sent before and after `it` in its pane, read once, shown or hidden by `key`. */
async function context(box, it, key) {
  if (box.dataset.read) { box.hidden = !box.hidden; key.setAttribute("aria-expanded", String(!box.hidden)); return; }
  box.hidden = false;
  box.replaceChildren(el("p", "msay", "Reading…"));
  try {
    const c = await api(`/api/messages/${it.id}/context?before=3&after=3`);
    box.dataset.read = "1";
    const rows = [...c.before.map((x) => contextRow(x, false)), contextRow(c.item, true), ...c.after.map((x) => contextRow(x, false))];
    box.replaceChildren(...rows);
    if (!c.before.length && !c.after.length) box.append(el("p", "msay", "Nothing else was sent in that pane."));
  } catch (e) {
    box.replaceChildren(el("p", "merr", e.message));
  }
  key.setAttribute("aria-expanded", "true");
}
