// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// What the board's parts share: terminumd's routes, small DOM helpers, dialogs and files.
import * as signin from "./signin.js";
import { PROJECT_HUES, el, hueColor, iconButton } from "./listparts.js";
import { renderers } from "./renderlib.js";
import { toast } from "./toast.js";
import { askConfirm } from "./dialogs.js";
import { dropLit } from "./droplit.js";
import { cardState } from "./cardstate.js";

export const COLUMNS = ["TODO", "IN PROGRESS", "WAIT FOR HUMAN", "FAILED", "DONE"];
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
 *  message, with its status. */
export async function api(path, { method = "GET", body } = {}) {
  const sent = method === "GET" ? undefined : JSON.stringify(body ?? {});
  const r = await fetch(path, { method, headers: sent ? { "Content-Type": "application/json" } : undefined, body: sent });
  const m = await r.json().catch(() => ({}));
  if (r.status === 401) refused();
  if (!r.ok) {
    const e = new Error(m.error ?? `terminumd answered ${r.status}`);
    e.status = r.status;
    throw e;
  }
  return m;
}

/** A date and time as the windows say them. */
export const stamp = (at) => new Date(at * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

/** A time as a conversation says it: today's by its hour, another day's with its date; `stamp` says it in full. */
export function shortStamp(at) {
  const d = new Date(at * 1000), time = { hour: "2-digit", minute: "2-digit" };
  return d.toLocaleString(undefined, d.toDateString() === new Date().toDateString() ? time : { month: "short", day: "numeric", ...time });
}

/** "made …, changed …": when a task was made and last changed (AC-149). */
export const madeChanged = (t) => `made ${stamp(t.created_at)}${t.updated_at > t.created_at ? ` · changed ${stamp(t.updated_at)}` : ""}`;

/** A box in a status's color (AC-112). */
const statusClass = (status) => `stbox st-${status.toLowerCase().replaceAll(" ", "-")}`;

/** A task's status in words on its color; an archived task says so, in no status's color. */
export const statusTag = (t) => (t.archived_at ? el("span", "stbox word", "Archived") : el("span", `${statusClass(t.status)} word`, t.status));

/** A task as a small box: its ID on its status's color, the status in words too (AC-112). */
export function statusBox(t, open) {
  const b = el(open ? "button" : "span", statusClass(t.status), t.id);
  const said = [t.id, t.status, t.title].filter(Boolean).join(" · ");
  b.title = said;
  b.setAttribute("aria-label", said);
  if (open) {
    b.type = "button";
    b.addEventListener("click", (e) => { e.stopPropagation(); open(t.id); });
  }
  return b;
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

/** A text pressed ⌘Enter in goes where its button would. */
export const onSend = (area, go) => area.addEventListener("keydown", (e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); go(); } });

/** A queued RUN stays visible, but cannot replace the press already waiting (AC-122, AC-169). */
export function guardQueuedRun(key, waits) {
  key.disabled = !!waits;
  if (waits) key.title = `Already queued for ${waits}: Stop waiting before running it again`;
  return key;
}

export function button(cls, text, onClick) {
  const b = el("button", cls, text);
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

/** A project's `agents` for `select`, a busy one said so by `busy` and chosen all the same: a run waits for its
 *  room (terminum-dispatcher.md AC-158); the first of `wanted`, else the first. `none`: the label of a choice of
 *  no agent, first; "" in `wanted` picks it. */
export function fillAgents(select, agents, busy, wanted, none = null) {
  const pick = wanted.find((id) => (id === "" ? none : agents.some((a) => a.id === id))) ?? agents[0]?.id ?? "";
  fillSelect(select, [...(none ? [["", none]] : []), ...agents.map((a) => [a.id, busy(a) ? `${a.name} (${busy(a)})` : a.name])], pick);
}

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

/** Saves a file through a fetch with the sign-in, never its address (terminum.md AC-09). */
export async function download(id, name) {
  const r = await fetch(fileUrl(id));
  if (!r.ok) throw new Error(`${name}: terminumd answered ${r.status}`);
  const url = URL.createObjectURL(await r.blob());
  const a = el("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 60000);
}

/** Markdown by the Panel's rules (raw HTML off, javascript: links dropped, remote images not
 *  loaded); a task's own pictures are fetched with the sign-in, its files download. */
export async function renderInto(box, text) {
  box.textContent = text;
  try {
    box.innerHTML = (await renderers()).taskMarkdownHtml(text);
  } catch { return; }
  for (const img of box.querySelectorAll("img[data-raw]")) {
    signin.blobUrl(img.dataset.raw).then((got) => {
      if (!got.url) return;
      const done = () => URL.revokeObjectURL(got.url);   // a picture decoded needs its address no more
      img.addEventListener("load", done, { once: true });
      img.addEventListener("error", done, { once: true });
      img.src = got.url;
    });
  }
  for (const a of box.querySelectorAll("a")) {
    const id = /^\/api\/task-files\/([0-9a-f]{32})$/.exec(a.getAttribute("href") ?? "")?.[1];
    if (id) a.addEventListener("click", (e) => { e.preventDefault(); download(id, a.textContent).catch((err) => say(err.message)); });
    else { a.target = "_blank"; a.rel = "noopener noreferrer"; }
  }
}

/** 1,234 as 1.2k, 4,800,000 as 4.8M. */
export function countOf(n) {
  return n >= 1e6 ? `${(n / 1e6).toFixed(1).replace(/\.0$/, "")}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1).replace(/\.0$/, "")}k` : String(n);
}

/** A text's Write and Preview keys (`${prefix}write`, `${prefix}preview`) over its area and its rendered Markdown; the view. */
export function writePreview(prefix, area, shown) {
  const view = (preview) => {
    $(`${prefix}write`).setAttribute("aria-selected", String(!preview));
    $(`${prefix}preview`).setAttribute("aria-selected", String(preview));
    area.hidden = preview;
    shown.hidden = !preview;
    if (preview) renderInto(shown, area.value);
  };
  $(`${prefix}write`).addEventListener("click", () => view(false));
  $(`${prefix}preview`).addEventListener("click", () => view(true));
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
