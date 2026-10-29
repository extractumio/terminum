// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Going through a session's bookmarks and reminders (AC-33): the bookmarks' button's pane is a
// navigator, the list at its left (the session's own top to bottom, its project's others' after,
// newest first), the chosen one's content at its right, rendered as the terminal renders Markdown
// (an agent's record, AC-98, its own Markdown; the user's, the terminal's text as it was). A press
// or ↑ ↓ chooses one; Show in terminal (Enter) brings it into view, Quote in chat pastes it, Resolve
// (or Delete, ⌫) settles it at once in the list, a reminder resolved and a bookmark deleted, with
// Undo for 5 s: only then terminumd is told. The bar after a jump has the same Resolve.
import { icon } from "./icons.js";
import { chatText, el, INSERT, openReminder, when } from "./marks.js";
import { agentMark } from "./nav.js";
import { markdownView } from "./widgetui.js";

const $ = (id) => document.getElementById(id);
const UNDO = 5000;             // ms a ✓ waits before terminumd is told
const MOUSE = globalThis.matchMedia?.("(hover: hover) and (pointer: fine)");   // none in node (unit tests)
const firstLine = (text, max = 90) => { const l = text.trim().split("\n")[0]; return l.length > max ? `${l.slice(0, max)}…` : l; };

/** A record's content as Markdown: an agent's as it wrote it, the user's selection in a fence (the
 *  terminal's text exactly), longer than any run of backticks in it. */
export function recordMarkdown(item) {
  if (item.by) return item.text;
  const fence = "`".repeat(Math.max(3, ...[...item.text.matchAll(/`+/g)].map((m) => m[0].length + 1)));
  return `${fence}text\n${item.text}\n${fence}`;
}

/** The mark before an entry's title: an agent's for its record, a bell for a reminder, none else. */
export function kindMark(item) {
  if (item.by) return agentMark({ agent: item.by }, 13);
  if (item.remind) { const s = el("span", "mkbell"); s.append(icon("bell", 12)); return s; }
  return null;
}

/** A Resolve key as its item's state says: Undo while pending, else ✓ Resolve. */
function resolveKey(b, pending) {
  b.replaceChildren(...(pending ? ["Undo"] : [icon("check", 14), "Resolve"]));
  b.classList.toggle("pending", pending);
}

export class MarkNav {
  /** marks: marks.js's Marks (here, sid, jump, send, togglePane, shown, hideBar). */
  constructor(marks) {
    this.m = marks;
    this.pending = new Map();      // id -> timer: settled in the list, terminumd told when it fires
    this.chosen = null;            // the entry the pane is on
    this.drawn = "";               // what the view shows: its entry's id and text
    this.view = this.makeView();
    const pane = $("markpane");
    pane.tabIndex = -1;
    pane.addEventListener("keydown", (e) => this.key(e));
    $("mbdone").onclick = () => { const s = this.m.shown; if (s) (this.pending.has(s.id) ? this.undo(s.id) : this.done(s)); };
    this.sends = new Map();          // id -> what terminumd is told when its Undo runs out
    // the page closing or reloading within the Undo's 5 s: what was done is told at once, never lost
    addEventListener("pagehide", () => { for (const [id, msg] of this.sends) { clearTimeout(this.pending.get(id)); this.m.send(msg); } this.sends.clear(); });
  }

  /** The session's bookmarks and open reminders: its own by line (an agent's record not found yet
   *  by its time after them), then its project's others' newest first. */
  entries() {
    const m = this.m, own = (b) => b.sid === m.sid();
    const here = m.here().filter((b) => !b.remind || openReminder(b) || this.pending.has(b.id));
    const byLine = (a, b) => Number(a.n < 0) - Number(b.n < 0) || (a.n < 0 ? a.at - b.at : a.n - b.n);
    return [...here.filter(own).sort(byLine), ...here.filter((b) => !own(b)).sort((a, b) => b.at - a.at)];
  }

  draw() {
    const pane = $("markpane");
    if (pane.hidden) return;
    const all = this.entries(), project = this.m.hereProject();
    if (!all.some((b) => b.id === this.chosen)) this.chosen = all[0]?.id ?? null;   // something is shown at once
    const others = all.some((b) => b.sid !== this.m.sid());
    const ul = el("ul", "mknav");
    for (const item of all) ul.append(this.row(item, others));
    // a head as View's (panewin.css): the title, × at the right
    const head = el("div", "pwhead");
    const close = el("button", "icon");
    Object.assign(close, { type: "button", title: "Close (Esc)", onclick: () => this.m.togglePane(false) });
    close.setAttribute("aria-label", "Close");
    close.append(icon("x", 16));
    head.append(el("h2", "pwtitle", project ? `Bookmarks and reminders of ${project.name}` : "Bookmarks and reminders of this session"), close);
    const hint = el("p", "mkhint", "↑ ↓ choose · Enter: show in terminal · Delete: resolve · Esc: close");
    if (!all.length) return pane.replaceChildren(head, el("p", "empty pwbody", "None here."));
    const cols = el("div", "mkcols");
    cols.append(ul, this.view.box);
    pane.replaceChildren(head, cols, hint);
    this.show(all.find((b) => b.id === this.chosen), others);
    pane.querySelector(".chosen")?.scrollIntoView({ block: "nearest" });
  }

  /** The view at the right: its head (title, where and when), its content, three actions. */
  makeView() {
    const box = el("section", "mkview"), head = el("div", "mkvhead"), body = el("div", "mkvbody"), acts = el("div", "mkvacts");
    const act = (cls, label, fn, title) => { const b = el("button", cls, label); b.type = "button"; b.title = title; b.onclick = fn; acts.append(b); return b; };
    const item = () => this.entries().find((b) => b.id === this.chosen);
    act("mkvgo", "Show in terminal", () => { const it = item(); if (it) { this.m.togglePane(false); void this.m.jump(it); } }, "Bring it into view in the terminal (Enter)");
    const done = act("mkvdone", "Resolve", () => { const it = item(); if (it) (this.pending.has(it.id) ? this.undo(it.id) : this.done(it)); }, "");
    act("tochat", INSERT, () => { const it = item(); if (it) { this.m.togglePane(false); this.m.toChat(chatText(it)); } }, "Paste its note and its text quoted into the input, no Enter");
    box.append(head, body, acts);
    return { box, head, body, done };
  }

  /** `item` in the view: its content drawn again only when it changed (a list redraw keeps it). */
  show(item, others) {
    const { head, body, done } = this.view;
    const pending = this.pending.has(item.id);
    resolveKey(done, pending);
    done.title = pending ? "Take it back" : item.remind ? "Resolve this reminder (Undo for 5 s; Delete)" : "Delete this bookmark (Undo for 5 s; Delete)";
    const key = `${item.id}\n${item.note}\n${item.text}`;
    if (key === this.drawn) return;
    this.drawn = key;
    const title = el("span", "mkvtitle" + (item.note ? "" : " quiet"));
    const mark = kindMark(item);
    if (mark) title.append(mark);
    title.append(item.note || firstLine(item.text));
    const now = this.m.alive(item.sid), theirs = others && item.sid !== this.m.sid();
    head.replaceChildren(title, el("span", "mkvmeta", [theirs && (now ?? `${item.title} (closed)`), item.host, when(item.at)].filter(Boolean).join(" · ")));
    body.scrollTop = 0;
    void markdownView(body, recordMarkdown(item));
  }

  row(item, others) {
    const pending = this.pending.has(item.id);
    const li = el("li", [item.remind ? "remind" : "", item.by ? "agent" : "", item.id === this.chosen ? "chosen" : "", pending ? "pending" : ""].filter(Boolean).join(" "));
    li.dataset.id = item.id;
    const b = el("button", "mkitem");
    b.type = "button";
    const title = el("span", "note" + (item.note ? "" : " quiet"));
    const mark = kindMark(item);
    if (mark) title.append(mark);
    title.append(item.note || firstLine(item.text));
    b.append(title);
    // another session's: by its name now, a closed one by its last and "closed" (as marks.js listOf)
    const now = this.m.alive(item.sid), theirs = others && item.sid !== this.m.sid();
    const meta = el("span", "meta", (theirs ? [now ?? item.title, when(item.at)] : [when(item.at)]).join(" · "));
    if (theirs && now === null) meta.append(el("span", "closed", "closed"));
    b.append(meta);
    b.onclick = () => this.choose(item);
    li.append(b);
    if (pending) {
      const undo = el("button", "mkundo", "Undo");
      undo.type = "button";
      undo.onclick = (e) => { e.stopPropagation(); this.undo(item.id); };
      li.append(el("span", "mkdonetext", item.remind ? "Resolved" : "Deleted"), undo);
    }
    return li;
  }

  /** Shows `item` in the view; the keys stay the pane's on a computer. */
  choose(item) {
    this.chosen = item.id;
    this.draw();
    if (MOUSE?.matches && !$("markpane").hidden) $("markpane").focus({ preventScroll: true });
  }

  /** Settled in the list at once; terminumd told after UNDO unless undone. */
  done(item) {
    if (this.pending.has(item.id)) return;
    const msg = item.remind ? { t: "remind", id: item.id, action: "resolve" } : { t: "unmark", id: item.id };
    this.sends.set(item.id, msg);
    this.pending.set(item.id, setTimeout(() => {
      this.pending.delete(item.id);
      this.sends.delete(item.id);
      this.m.send(msg);
      if (this.m.shown?.id === item.id) this.m.hideBar();
      this.draw();
    }, UNDO));
    this.sync(item.id);
  }

  undo(id) {
    clearTimeout(this.pending.get(id));
    this.pending.delete(id);
    this.sends.delete(id);
    this.sync(id);
  }

  /** The list and the bar's ✓ after a change of what is pending. */
  sync(id) {
    this.draw();
    if (this.m.shown?.id === id) this.bar(this.m.shown);
  }

  /** The bar shows `item`: its ✓ as its pending state says. */
  bar(item) {
    resolveKey($("mbdone"), this.pending.has(item.id));
    $("mbdone").title = item.remind ? "Resolve this reminder (Undo for 5 s)" : "Resolve: delete this bookmark (Undo for 5 s)";
  }

  key(e) {
    const all = this.entries(), i = all.findIndex((b) => b.id === this.chosen);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!all.length) return;
      const next = i < 0 ? (e.key === "ArrowDown" ? 0 : all.length - 1) : Math.max(0, Math.min(all.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)));
      if (next !== i) this.choose(all[next]);
    } else if ((e.key === "Delete" || e.key === "Backspace") && i >= 0) {
      e.preventDefault();
      const item = all[i];
      this.pending.has(item.id) ? this.undo(item.id) : this.done(item);
    } else if (e.key === "Enter" && i >= 0) {
      e.preventDefault();
      this.m.togglePane(false);
      void this.m.jump(all[i]);
    }
  }
}
