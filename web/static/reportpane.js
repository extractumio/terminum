// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A report's reading pane (terminum-reports.md AC-268, AC-269): over the board's right part, its agents, routines and
// columns, the list staying beside it; on a phone over the whole board. Its address is /kanban/?report=<ID>; × or Esc
// gives the columns back. Open 2 seconds with its page shown, the report is read. Its body is agents' text: it shows only
// through the Panel's Markdown rules (renderInto), its files download, nothing of it runs.
import { icon } from "./icons.js";
import { el, iconButton } from "./listparts.js";
import { api, button, download, onSend, renderInto, stamp } from "./kanbanutil.js";
import { sessionUrl } from "./tasklink.js";
import { CHANGED_EVENT, REPORT_EVENT, act as inboxAct, age, copyLink, openReport, remove as inboxRemove } from "./inbox.js";

const READ_AFTER = 2000;
const $ = (id) => document.getElementById(id);
const pane = el("section", "reportpane");
pane.id = "reportpane";
pane.hidden = true;
pane.tabIndex = -1;
pane.setAttribute("aria-label", "Report");
let shown = null, drawn = "", timer = 0, asked = 0;

const addressed = () => new URLSearchParams(location.search).get("report");

/** The address with report `id`, or none; one Back step for the pane. */
function address(id) {
  const u = new URL(location.href);
  if (id) u.searchParams.set("report", id); else u.searchParams.delete("report");
  if (u.href === location.href) return;
  if (id && !addressed()) history.pushState({ report: true }, "", u);
  else history.replaceState(history.state, "", u);
}

/** Where the pane stands: under the board's head over its agents, routines and columns; on a phone under the bar. */
function place() {
  const main = $("main"), narrow = matchMedia("(max-width: 860px)").matches;
  const from = narrow ? $("bar") : ["agentbar", "schedbar", "board"].map($).find((e) => e?.getClientRects().length && e.offsetHeight) ?? $("board");
  const top = narrow ? from.getBoundingClientRect().bottom : from.getBoundingClientRect().top;
  pane.style.top = `${Math.max(0, top - main.getBoundingClientRect().top)}px`;
}

/** Reads it in the pane 2 seconds after it shows, the page shown meanwhile (AC-269). */
function readSoon(r) {
  clearTimeout(timer);
  if (r.read || r.deleted_at || document.visibilityState !== "visible") return;
  timer = setTimeout(() => {
    if (shown?.id !== r.id || pane.hidden || document.visibilityState !== "visible") return;
    api(`/api/reports/${encodeURIComponent(r.id)}/read`, { method: "POST", body: {} }).catch((e) => note(`Not marked read: ${e.message}`));
  }, READ_AFTER);
}

function note(text) {
  pane.querySelector(".rpnote")?.replaceChildren(text);
}

/** Opens report `id`, or reads the one shown again (`again`), keeping its scroll. */
export async function open(id, again = false) {
  const ask = ++asked;
  if (!again) address(id);
  pane.hidden = false;
  place();
  document.body.classList.add("reading");
  let r;
  try { r = await api(`/api/reports/${encodeURIComponent(id)}`); }
  catch (e) {
    if (ask !== asked) return;
    shown = null;
    pane.replaceChildren(head(null, id), el("p", "rpnone", e.status === 400 ? `No report ${id}.` : `${id} could not be read: ${e.message}`));
    return;
  }
  if (ask !== asked) return;
  shown = r;
  const same = JSON.stringify(r);
  if (again && same === drawn) return;           // another report changed: this one stands as drawn
  drawn = same;
  const scroll = again ? pane.querySelector(".rpbody")?.scrollTop ?? 0 : 0;
  await draw(r);
  pane.querySelector(".rpbody").scrollTop = scroll;
  if (!again) pane.focus({ preventScroll: true });
  readSoon(r);
}

/** The pane goes; the address keeps or loses the report as the caller says. */
function hide() {
  clearTimeout(timer);
  asked++;
  shown = null;
  drawn = "";
  pane.hidden = true;
  document.body.classList.remove("reading");
}

export function close() {
  hide();
  address(null);
}

function head(r, id) {
  const h = el("header", "rphead");
  const title = el("h2", "rptitle");
  if (r?.critical) {
    const mark = el("span", "rpcritical");
    mark.title = "Critical";
    mark.append(icon("triangle-alert", 16));
    title.append(mark);
  }
  title.append(el("span", null, r?.title ?? id));
  const acts = el("div", "rpacts");
  if (r && !r.deleted_at) {
    acts.append(iconButton("icon", r.read ? "mail" : "mail-open", r.read ? "Mark as unread" : "Mark as read", r.read ? "Mark as unread (U)" : "Mark as read",
      () => act(r, r.read ? "unread" : "read")),
    iconButton("icon", "x", "Delete", "Delete (Backspace)", () => remove(r)));
  }
  if (r) acts.append(iconButton("icon", "copy", "Copy link", "Copy this report's link", () => copyLink(r.id, note)));
  acts.append(iconButton("icon rpclose", "chevron-left", "Close the report", "Close (Esc): the columns again", close));
  h.append(title, acts);
  if (r) {
    const meta = el("p", "rpmeta");
    const src = r.source;
    const bits = [`${r.id}`, r.project_name, `by ${r.author_name}${r.role === "terminum" ? "" : ` (${r.role})`}`, `made ${stamp(r.created_at)}`];
    if (r.updated_at > r.created_at) bits.push(`changed ${stamp(r.updated_at)}`);
    meta.append(el("span", null, bits.join(" · ")));
    if (src.task) meta.append(" · ", link(`from ${src.task}${src.schedule ? ` (${src.name})` : ""}`, () => openTask(src.task)));
    else if (src.incident) meta.append(" · ", link(`incident ${src.incident}`, () => { location.href = `/supervisor/?incident=${src.incident}`; }));
    h.append(meta, el("p", "rpnote"));
  }
  return h;
}

const link = (text, go) => button("linklike", text, go);

/** A task's window over the board, at its own address, which the board's router reads (kanbanroute.js). */
function openTask(id) {
  const u = new URL(location.href);
  u.pathname = `/kanban/${encodeURIComponent(id)}`;
  u.searchParams.delete("report");
  hide();
  history.pushState({ win: true }, "", u);
  dispatchEvent(new PopStateEvent("popstate"));
}

async function draw(r) {
  const body = el("div", "rpbody");
  if (r.deleted_at) {
    body.append(el("p", "rpdeleted", "Deleted: restore it to keep it."),
      Object.assign(el("button", "textkey", "Restore"), { type: "button", onclick: () => act(r, "restore") }));
  }
  if (r.summary) body.append(el("p", "rpsummary", r.summary));
  const md = el("div", "markdown rpmarkdown");
  body.append(md);
  if (r.files.length) {
    const files = el("ul", "rpfiles");
    for (const f of r.files) {
      const li = el("li");
      li.append(link(`${f.name} (${Math.max(1, Math.round(f.bytes / 1024))} KB)`, () => download(f.id, f.name).catch((e) => note(e.message))));
      files.append(li);
    }
    body.append(el("h3", null, "Files"), files);
  }
  const links = el("p", "rplinks");
  if (r.follows) links.append("Follows ", link(r.follows, () => openReport(r.follows)));
  for (const f of r.follow_ups) links.append(links.childNodes.length ? " · " : "", "Follow-up: ", link(f, () => openReport(f)));
  if (links.childNodes.length) body.append(links);
  const notes = el("ol", "rpnotes");
  for (const n of r.notes) {
    const li = el("li", `rpnote-${n.kind} by-${n.author_kind}`);
    const said = el("div", "markdown");
    li.append(el("div", "evhead", `${n.author_name} · ${age(n.at)}`), said);
    if (n.kind === "change") said.textContent = `${n.author_name} ${n.text}`;
    else renderInto(said, n.text, { comment: true });
    notes.append(li);
  }
  if (r.notes.length) body.append(el("h3", null, `Below it · ${r.notes.length}`), notes);
  pane.replaceChildren(head(r), body, foot(r));
  await renderInto(md, r.body);
}

/** The reply (AC-270): a field and Send when its task's conversation can go on, its agent's answer awaited said; else
 *  where to answer. A draft outlives the pane's redraws and a refusal, said under it. */
const drafts = new Map();
function foot(r) {
  const f = el("footer", "rpfoot");
  if (r.answering) f.append(el("p", "rpanswering", `${r.answering} is answering`));
  else if (r.no_answer) f.append(el("p", "rpnoanswer", r.no_answer));
  if (!r.replyable) {
    const why = el("p", "rpwhy", r.why_not ?? "");
    if (r.source.incident) why.append(" ", link("Open the incident", () => { location.href = `/supervisor/?incident=${r.source.incident}`; }));
    else if (r.session) why.append(" ", link("Open its session", () => { location.href = sessionUrl(r.session); }));
    f.append(why);
    return f;
  }
  const area = el("textarea", "field rpreply");
  area.placeholder = `Reply to ${r.author_name}: it goes on in ${r.source.task}'s conversation. Markdown.`;
  area.value = drafts.get(r.id) ?? "";
  area.addEventListener("input", () => drafts.set(r.id, area.value));
  const said = el("p", "rpsaid");
  const send = button("primary", "Send", async () => {
    send.disabled = true;
    try {
      await api(`/api/reports/${encodeURIComponent(r.id)}/reply`, { method: "POST", body: { text: area.value } });
      drafts.delete(r.id);
      area.value = "";
      open(r.id, true);
    } catch (e) { said.textContent = e.message; }      // the reply stays in its field
    finally { send.disabled = false; }
  });
  send.title = "Send (⌘Enter)";
  onSend(area, () => send.click());
  f.append(area, said, send);
  return f;
}

/** The Inbox's own acts, their refusals said in the pane; the pane reads its report again after. */
const act = (r, what) => inboxAct(r.id, what, note).then(() => open(r.id, true));
const remove = async (r) => { if (await inboxRemove(r.id, note)) close(); };

function onKey(e) {
  if (pane.hidden || e.target.closest?.("input, textarea, select, [contenteditable], dialog")) return;
  if (e.key === "Escape") { e.preventDefault(); close(); return; }
  if (!shown || !pane.contains(document.activeElement) || e.metaKey || e.ctrlKey || e.altKey) return;
  if (e.key === "Backspace" || e.key === "Delete") { e.preventDefault(); remove(shown); }
  else if (e.key === "u" || e.key === "U") { e.preventDefault(); act(shown, shown.read ? "unread" : "read"); }
}

$("main").append(pane);
addEventListener(REPORT_EVENT, (e) => open(e.detail));
addEventListener(CHANGED_EVENT, () => { if (shown) open(shown.id, true); });
addEventListener("popstate", () => { const id = addressed(); if (id) { if (shown?.id !== id) open(id, true); } else if (!pane.hidden) hide(); });
addEventListener("keydown", onKey);
addEventListener("resize", () => { if (!pane.hidden) place(); });
document.addEventListener("visibilitychange", () => { if (shown && document.visibilityState === "visible") readSoon(shown); });
if (addressed()) open(addressed(), true);
