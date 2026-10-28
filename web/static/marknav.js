// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Going through a session's bookmarks and reminders (AC-33): the bookmarks' button's pane lists
// them together, the session's own top to bottom, its project's others' after, newest first. A
// press (or ↑ ↓ in the pane) brings one into view and keeps the pane open on it, so the next goes
// on; Enter or Esc closes it. ✓ (or Delete, ⌫) settles one at once in the list, a reminder
// resolved and a bookmark deleted, with Undo there for 5 s: only then terminumd is told. The bar
// after a jump has the same ✓. An agent's record (AC-98) shows its agent's mark.
import { icon } from "./icons.js";
import { el, openReminder, when } from "./marks.js";
import { agentMark } from "./nav.js";

const $ = (id) => document.getElementById(id);
const UNDO = 5000;             // ms a ✓ waits before terminumd is told
const MOUSE = globalThis.matchMedia?.("(hover: hover) and (pointer: fine)");   // none in node (unit tests)
const firstLine = (text, max = 90) => { const l = text.trim().split("\n")[0]; return l.length > max ? `${l.slice(0, max)}…` : l; };

/** The mark before an entry's title: an agent's for its record, a bell for a reminder, none else. */
export function kindMark(item) {
  if (item.by) return agentMark({ agent: item.by }, 13);
  if (item.remind) { const s = el("span", "mkbell"); s.append(icon("bell", 12)); return s; }
  return null;
}

export class MarkNav {
  /** marks: marks.js's Marks (here, sid, jump, send, togglePane, shown, hideBar). */
  constructor(marks) {
    this.m = marks;
    this.pending = new Map();      // id -> timer: settled in the list, terminumd told when it fires
    this.chosen = null;            // the entry the pane is on
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
    const ul = el("ul", "mknav");
    for (const item of all) ul.append(this.row(item, all.some((b) => b.sid !== this.m.sid())));
    const head = el("h2", "", project ? `Bookmarks and reminders of ${project.name}` : "Bookmarks and reminders of this session");
    const hint = el("p", "mkhint", "↑ ↓ go through them · ✓ or Delete: done · Enter: close");
    pane.replaceChildren(head, all.length ? ul : el("p", "empty", "None here."), hint);
    pane.querySelector(".chosen")?.scrollIntoView({ block: "nearest" });
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
    if (item.note) b.append(el("span", "snip", firstLine(item.text)));
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
    } else {
      const done = el("button", "mkdone");
      done.type = "button";
      done.title = item.remind ? "Done: resolve this reminder" : "Done: delete this bookmark";
      done.setAttribute("aria-label", done.title);
      done.append(icon("check", 14));
      done.onclick = (e) => { e.stopPropagation(); this.done(item); };
      li.append(done);
    }
    return li;
  }

  /** Brings `item` into view; the pane stays open on it, with the keys on a computer (showing a
   *  session gives the terminal the keys: app.js pick). */
  choose(item) {
    this.chosen = item.id;
    this.draw();
    void this.m.jump(item);
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
    if (this.m.shown?.id !== id) return;
    const on = this.pending.has(id);
    $("mbdone").classList.toggle("pending", on);
    $("mbdone").replaceChildren(...(on ? ["Undo"] : [icon("check", 14), "Done"]));
  }

  /** The bar shows `item`: its ✓ as its pending state says. */
  bar(item) {
    const on = this.pending.has(item.id);
    $("mbdone").classList.toggle("pending", on);
    $("mbdone").replaceChildren(...(on ? ["Undo"] : [icon("check", 14), "Done"]));
    $("mbdone").title = item.remind ? "Done: resolve this reminder (Undo for 5 s)" : "Done: delete this bookmark (Undo for 5 s)";
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
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (i < 0 && all.length) return this.choose(all[0]);
      this.m.togglePane(false);
    }
  }
}
