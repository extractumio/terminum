// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The board's Routines section (terminum-schedules.md AC-163; "routine" is the board's word for a schedule): between the
// pool and the columns, folded by its rail by default (terminum-kanban.md AC-203) and remembered by this browser; open,
// its cards stand on the columns' tracks, four to a row, its add tile after the last. terminumd owns the schedules and
// says their next runs and words; every page hears of a change.
import { askConfirm } from "./dialogs.js";
import { el, iconButton } from "./listparts.js";
import { rail } from "./kanbanrail.js";
import { scheduleWindow } from "./kanbanschedwin.js";
import { $, agentColor, api, button, say, statusBox } from "./kanbanutil.js";
import { age, openReport } from "./reportlink.js";

/** `getMeta()`: projects and agents; `chosen()`: the projects shown, none for all; `openTask(id)`. */
export function scheduledSection({ getMeta, chosen, openTask }) {
  const list = el("div", "agentlist trackitems schedlist"), box = el("div", "tracks schedtracks schedsection");
  box.id = "schedgrid";
  const add = iconButton("addtile", "plus", "Add routine", "Add a routine: a task that runs on a cadence on one agent", () => win.open(), 16);
  add.id = "newschedule";
  box.append(list);
  let schedules = [], drawn = "";
  const win = scheduleWindow({ getMeta, chosen, changed: () => load() });
  const railed = rail($("schedbar"), box, { key: "tm.board.schedules", open: false, add: () => win.open() });

  /** Its card: SCH-1, severity, title, cadence, next run or why it is paused, agent, last task, skips, why its last run did not start. */
  function card(s) {
    const c = el("article", "item schedcard");
    c.dataset.id = s.id;
    c.tabIndex = 0;
    const a = getMeta().agents.find((x) => x.id === s.agent);
    c.style.setProperty("--ag", a ? agentColor(a) : "var(--muted)");
    const top = el("div", "chead");
    top.append(el("span", "cid", s.key), el("span", `csev ${s.severity}`, s.severity), runNow(s));
    const foot = el("div", "cfoot");
    foot.append(el("span", "schedwords", s.words));
    foot.append(el("span", `schednext${s.paused ? " paused" : ""}`, s.paused ? s.paused_why ?? "paused" : s.next.length ? `next ${s.next_words[0]}` : "never"));
    const agent = el("span", "cagent", a?.name ?? "agent gone");
    foot.append(agent);
    const last = el("div", "schedlast");
    if (s.last) {
      last.append(statusBox({ id: s.last.task, status: s.last.status }, openTask));
      if (s.last.status === "WAIT FOR HUMAN") last.append(el("span", "schedwaits", "waits for you"));   // a question (AC-157)
    }
    // its last report and how many of its reports are unread (terminum-reports.md AC-272)
    if (s.report) {
      // the card's own press opens its window: this one opens the report
      const go = button("linklike", s.report.id, (e) => { e.stopPropagation(); openReport(s.report.id); });
      last.append(go, el("span", "schedage", age(s.report.at)));
    }
    if (s.unread) last.append(el("span", "schedunread", `${s.unread} unread`));
    if (s.skipped) last.append(el("span", "schedskips", `skipped ${s.skipped}`));
    if (s.failed_why) last.append(el("span", "schedfailed", s.failed_why));   // its last run did not start: the next firing tries again
    const title = el("div", "ctitle", s.title);
    title.title = s.title;
    c.append(...[top, title, foot, last.children.length ? last : null].filter(Boolean));
    c.title = `${s.key} · set by ${s.author_name || "you"}`;
    const go = () => win.open(s);
    c.addEventListener("click", go);
    c.addEventListener("keydown", (e) => { if (e.key === "Enter" && e.target === c) go(); });
    return c;
  }

  /** Run now (AC-182), a play key at the head's right: the schedule's task made and started at once, whatever its
   *  cadence; terminumd says why not. */
  function runNow(s) {
    const b = iconButton("icon crun schedrun", "play", `Run ${s.key} now`, "Run now: make its task and start it", async () => {
      if (!await askConfirm(`Run ${s.key} now, on its agent? Its cadence stays as it is.`, "Run now")) return;
      b.disabled = true;
      try {
        const { task } = await api(`/api/schedules/${s.id}/run`, { method: "POST" });
        say(`${s.key} runs now: ${task}`);
        load();
      } catch (err) { say(`${s.key} did not run: ${err.message}`); }
      b.disabled = false;
    });
    b.addEventListener("keydown", (e) => e.stopPropagation());
    return b;
  }

  /** The folded strip's line: how many, and the next run of them ("3 · next SCH-2 today 03:00"), or none. */
  function summary(mine) {
    if (!mine.length) return "none";
    const soonest = mine.filter((s) => !s.paused && s.next.length).reduce((a, s) => (!a || s.next[0] < a.next[0] ? s : a), null);
    return soonest ? `${mine.length} · next ${soonest.key} ${soonest.next_words[0]}` : String(mine.length);
  }

  /** Anew only when what it shows changed. */
  function draw() {
    const shown = chosen(), mine = schedules.filter((s) => !shown.length || shown.includes(s.project));
    box.dataset.n = mine.length;                   // a phone's line says it (kanbanphone.js)
    railed.summary(summary(mine));
    const now = JSON.stringify([mine, getMeta().agents.map((a) => [a.id, a.name, a.color])]);
    if (now === drawn) return;
    drawn = now;
    list.replaceChildren(...mine.map(card), add);
    if (!mine.length) list.append(el("p", "colempty", "No routines yet: + adds a task that runs on a cadence."));
  }

  async function load() {
    try {
      schedules = (await api("/api/schedules")).schedules;
      draw();
    } catch (e) { say(`The routines could not be read: ${e.message}`); }
  }
  /** The window of the schedule `key` names ("SCH-1"), from a task's window. */
  const openKey = (key) => { const s = schedules.find((x) => x.key === key); if (s) win.open(s); };
  return { box, draw, load, openKey };
}
