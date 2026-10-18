// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Session navigator: tmux sessions (a host's session: the host) as groups, their windows and
// panes as rows.
// A row: its folder's bar (full for the session shown, pulsing while its agent works), an orange
// strip when it has bookmarks (AC-57), the window
// number and title, then host · program · folder · titles, and a coding agent's state in a
// column of its own.
import { folderHues, folderKey } from "./dircolors.js";
import { icon } from "./icons.js";
import { textForm } from "./render.js";

// A coding agent's state: working, waiting for the user, failed; done shows nothing.
const STATES = {
  working: { label: "Working", icon: "hourglass" },
  waiting: { label: "Waiting for your answer", icon: "circle-help" },
  failed: { label: "Failed", icon: "triangle-alert" },
};
export function stateIcon(state, size = 16) {
  const s = STATES[state];
  if (!s) return null;
  const wrap = document.createElement("span");
  wrap.className = `state ${state}`;
  wrap.title = s.label;
  wrap.setAttribute("role", "img");
  wrap.setAttribute("aria-label", s.label);
  wrap.append(icon(s.icon, size));
  return wrap;
}

let hues = new Map();                  // each folder's hue, for the list as last drawn (AC-01)
// a coding agent's own mark (their owners' trademarks, outside terminum's licenses: NOTICE)
const MARKS = { claude: ["agents/claude.svg", "Claude Code"], codex: ["agents/codex.svg", "Codex"] };

/** What runs in a pane, as an icon: a coding agent's mark, else the terminal's (also when a mark
 *  file was left out of the build). */
export function agentMark(item, size = 14) {
  const [src, name] = MARKS[item.agent] ?? [];
  const wrap = el("span", "agmark");
  wrap.title = name ?? "Terminal";
  if (!src) { wrap.append(icon("terminal", size)); return wrap; }
  const img = Object.assign(document.createElement("img"), { src, alt: "", width: size, height: size, draggable: false });
  img.addEventListener("error", () => img.replaceWith(icon("terminal", size)));
  wrap.append(img);
  return wrap;
}
let lastShown;                         // the session shown when the list was last drawn

/** The bar beside a session's two lines in its folder's color: at full strength for the session
 *  the page shows, half otherwise, pulsing while a coding agent in it works. (A dot read as "unread".) */
export function folderBar(item, shown = false, busy = false) {
  const d = el("span", "pbar" + (shown ? " on" : "") + (busy ? " busy" : ""));
  const key = folderKey(item), hue = key && hues.get(key);
  if (hue != null) d.style.setProperty("--dot", `oklch(var(--dir-l) var(--dir-c) ${hue.toFixed(1)})`);
  d.title = (key ? `Folder ${key.split("\n")[1]}` : "No folder") + (busy ? ", working" : "");
  return d;
}

const AGENT_WORDS = { claude: "ag-claude", codex: "ag-codex" };   // the program's own name, in its color

/** "a · b · c" as spans: the last part (a title, else the folder) shortens first, a program's
 *  name (a word right after the host) never. */
export function metaLine(parts, hostFirst) {
  const s = el("span", "s");
  const prog = hostFirst ? 1 : 0;
  parts.forEach((p, i) => {
    if (i) s.append(el("span", "sep", "·"));
    const cls = i === 0 && hostFirst ? "host" : i === parts.length - 1 ? "last" : i === prog && /^[\w.+-]+$/.test(p) ? "prog" : "";
    s.append(el("span", [cls, AGENT_WORDS[p]].filter(Boolean).join(" "), p));
  });
  return s;
}

/** The key a group's collapsed state is remembered by: its window or tmux session, and host. */
const groupKey = (g) => `${g.kind}\n${g.label}${g.session ? " " + g.session : ""}\n${g.host ?? ""}`;

function matches(item, q) {
  if (!q) return true;
  return [item.title, item.host ?? "", ...item.sub].join(" ").toLowerCase().includes(q);
}

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = textForm(String(text));
  return e;
}

// The handle a row or a group is dragged by (reorder.js); not while a filter shows a subset.
function grip(kind, label) {
  const g = el("span", "grip");
  g.dataset.kind = kind;
  g.title = `Drag to reorder ${label}`;
  g.setAttribute("aria-hidden", "true");
  g.append(icon("grip-vertical", 14));
  return g;
}

function row(item, current, onPick, movable, child = false, marked = new Set()) {
  const li = document.createElement("li");
  if (movable) li.append(grip("row", "this window"));
  const b = el("button", "item" + (child ? " child" : "") + (item.id === current ? " on" : ""));
  b.type = "button";
  b.dataset.id = item.id;
  if (item.id === current) b.setAttribute("aria-current", "true");
  const txt = el("span", "txt");
  const head = el("span", "head");
  const idx = el("span", "idx", child ? "" : item.index || "");
  if (item.index && !child) idx.title = item.host ?? "This machine";     // where the window runs
  head.append(idx, agentMark(item), el("span", "t", item.title));
  txt.append(head);
  const parts = [...(item.host ? [item.host] : []), ...item.sub];
  if (parts.length) txt.append(metaLine(parts, !!item.host));
  const st = el("span", "st");
  const mark = stateIcon(item.state);
  if (mark) st.append(mark);
  // the bar's column holds the orange strip beside it: the row's grid keeps its three columns
  const bar = folderBar(item, item.id === current, item.state === "working");
  if (marked.has(item.id)) {
    const both = el("span", "bars"), m = el("span", "bmbar");
    m.title = "Has bookmarks";
    both.append(bar, m);
    b.append(both, txt, st);
  } else b.append(bar, txt, st);
  b.addEventListener("click", () => onPick(item.id));
  li.append(b);
  return li;
}

/** A host group's state, on a line under its name (AC-81): "" when attached; terminumd's `why`
 *  says "reconnecting (2 s)" or ssh's last line, which can be long. */
export function hostState(g) {
  if (g.state === "reconnecting") return g.why || "reconnecting";
  if (g.state === "failed") return g.why || "failed";
  if (g.state === "off") return "off";
  if (g.state === "absent") return g.why || "not running";
  return "";
}

// A group's header collapses and expands it; while a filter is typed every match is shown. A
// host's group has a menu of its own (onHost(group, button)), also while it has no rows, and
// while its tmux runs other sessions an icon that offers them (onImport(group), AC-89).
export function renderNav(list, groups, current, query, onPick, collapsed, onToggle, onNew, marked = new Set(), onHost = null, onCopy = null, onImport = null) {
  const q = query.trim().toLowerCase();
  const frag = document.createDocumentFragment();
  // a session of the user's tmux on a host that is down waits for that host: the same state, the same look
  hues = folderHues(groups.flatMap((g) => g.items.map(folderKey)));
  const down = new Map(groups.filter((g) => g.host && !g.attached && g.state && g.state !== "attached").map((g) => [g.host, g.state]));
  let shown = 0;
  for (const g of groups) {
    const items = g.items.filter((it) => matches(it, q));
    const own = g.host || g.attached;            // a host's group, or a session of the user's tmux (AC-89)
    if (!items.length && !(own && !q)) continue;
    shown += items.length;
    const key = groupKey(g);
    const closed = !q && collapsed.has(key);
    const gs = g.state === "absent" && g.host && down.has(g.host) ? down.get(g.host) : g.state;
    const sec = el("section", `grp ${g.kind}` + (g.host ? " hostgrp" : "") + (gs && gs !== "attached" ? ` ${gs}` : ""));
    if (g.wid) sec.dataset.wid = g.wid;
    const h = el("h2");
    if (g.wid && !q) h.append(grip("group", "this session"));
    const t = el("button", "toggle");
    t.type = "button";
    t.setAttribute("aria-expanded", String(!closed));
    t.addEventListener("click", () => onToggle(key));
    t.append(icon("chevron-down", 14), el("span", "label", g.label));
    if (g.session) t.append(el("span", "tsess", g.session));      // a tmux group: its session's name; the rows name the host
    else if (g.host && g.host !== g.label) t.append(el("span", "host", g.host));
    if (g.where) t.append(el("span", "where", g.where));
    const count = el("span", "count", g.items.length ? String(g.items.length) : "");   // a group without rows says its state instead
    count.title = `${g.items.length} pane${g.items.length === 1 ? "" : "s"}`;
    if (closed && items.some((it) => it.id === current)) count.classList.add("on");   // the shown session is inside
    t.append(count);
    h.append(t);
    if (g.attach && onCopy) {                    // attach from another terminal (tmux -L … / ssh -t …)
      const c = el("button", "attach");
      c.type = "button";
      c.title = `Copy: ${g.attach}`;
      c.setAttribute("aria-label", `Copy the command that attaches to ${g.session ?? g.label}`);
      c.append(icon("copy", 13));
      c.addEventListener("click", (e) => { e.stopPropagation(); onCopy(g.attach); });
      h.append(c);
    }
    if (g.host && !g.attached && g.others > 0 && onImport) {
      const n = el("button", "import");
      n.type = "button";
      n.title = `Attach tmux sessions running on ${g.host} (${g.others})`;
      n.setAttribute("aria-label", n.title);
      n.append(icon("import", 14));
      n.addEventListener("click", (e) => { e.stopPropagation(); onImport(g); });
      h.append(n);
    }
    if (g.wid && g.new) {
      const n = el("button", "new");
      n.type = "button";
      n.title = g.new;
      n.setAttribute("aria-label", `${g.new} in ${g.label}`);
      n.append(icon("plus", 14));
      n.addEventListener("click", () => onNew(g.wid));
      h.append(n);
    }
    if (own && onHost) {
      const m = el("button", "hostbtn");
      m.type = "button";
      m.title = g.attached ? `${g.label}: detach from terminum` : `${g.host}: retry, off, on, remove`;
      m.setAttribute("aria-label", g.attached ? `Session ${g.label} of your tmux${g.host ? ` on ${g.host}` : ""}` : `Host ${g.host}`);
      m.setAttribute("aria-haspopup", "true");
      m.append(icon("ellipsis", 14));
      m.addEventListener("click", (e) => { e.stopPropagation(); onHost(g, m); });
      h.append(m);
    }
    sec.append(h);
    const state = own ? hostState(g) : "";
    if (state) sec.append(el("p", "gstate", state));   // under the name, wrapped: ssh's words can be long
    if (!closed) {
      const ul = el("ul");
      if (g.wid) ul.dataset.wid = g.wid;
      // a tab's panes after its first (a coding agent's subagents) go under it, and a pane's quick
      // shell (AC-88) under that pane wherever tmux has its window; a filter shows them flat
      const quickOf = new Map();
      if (!q) for (const it of items) if (it.quickOf && items.some((x) => x.id === it.quickOf)) quickOf.set(it.quickOf, [...(quickOf.get(it.quickOf) ?? []), it]);
      let parent = null;
      const kid = (it) => {
        if (!parent.kids) parent.append(parent.kids = el("ul", "kids"));
        parent.kids.append(row(it, current, onPick, false, true, marked));
      };
      const quicks = (it) => { for (const k of quickOf.get(it.id) ?? []) kid(k); };
      for (const it of items) {
        if (quickOf.has(it.quickOf)) continue;     // listed under its pane
        if (!q && it.pane > 1 && parent) { kid(it); quicks(it); continue; }
        parent = row(it, current, onPick, !!g.wid && !q, false, marked);
        ul.append(parent);
        quicks(it);
      }
      sec.append(ul);
    }
    frag.append(sec);
  }
  if (!shown) {
    frag.append(el("p", "empty", q ? "No session matches. Clear the filter to see all." : "No tmux sessions."));
  }
  if (lastShown !== undefined && current !== lastShown) frag.querySelector(".item.on")?.classList.add("fresh");   // it settles once (motion.css)
  lastShown = current;
  list.replaceChildren(frag);
}

export function findItem(groups, id) {
  for (const g of groups) for (const it of g.items) if (it.id === id) return { group: g, item: it };
  return null;
}
