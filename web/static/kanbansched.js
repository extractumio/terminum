// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The board's Scheduled section (terminum-schedules.md AC-163): between the pool and the columns, folded to its
// head by default and remembered by this browser; open, its cards stand on the columns' tracks, four to a row.
// terminumd owns the schedules and says their next runs and words; every page hears of a change.
import { askConfirm } from "./dialogs.js";
import { el, iconButton, pointerKey, read, write } from "./listparts.js";
import { scheduleWindow } from "./kanbanschedwin.js";
import { agentColor, api, button, say, statusBox } from "./kanbanutil.js";

const OPEN_KEY = "tm.board.schedules";

/** `getMeta()`: projects and agents; `chosen()`: the projects shown, none for all; `openTask(id)`. */
export function scheduledSection({ getMeta, chosen, openTask }) {
  const box = el("div", "schedsection");
  const head = el("div", "schedhead"), toggle = el("button", "textkey schedtoggle"), list = el("div", "agentlist trackitems schedlist");
  const grid = el("div", "tracks schedtracks");
  toggle.type = "button";
  toggle.setAttribute("aria-controls", "schedgrid");
  grid.id = "schedgrid";
  const add = button("textkey", "+ Schedule", () => win.open());
  add.id = "newschedule";
  add.title = "A task that runs on a cadence on one agent";
  head.append(pointerKey(toggle), pointerKey(add));   // + Schedule stands with the head: the section may be folded
  grid.append(list);
  box.append(head, grid);
  let schedules = [], drawn = "", opened = read(OPEN_KEY) === true;
  const win = scheduleWindow({ getMeta, chosen, changed: () => load() });


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
      if (s.last.status === "WAIT FOR HUMAN") last.append(el("span", "schedwaits", "waits for you"));
    }
    if (s.skipped) last.append(el("span", "schedskips", `skipped ${s.skipped}`));
    if (s.failed_why) last.append(el("span", "schedfailed", s.failed_why));   // its last run did not start: the next firing tries again
    c.append(...[top, el("div", "ctitle", s.title), foot, last.children.length ? last : null].filter(Boolean));
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
    return pointerKey(b);
  }

  /** Anew only when what it shows changed. */
  function draw() {
    const shown = chosen(), mine = schedules.filter((s) => !shown.length || shown.includes(s.project));
    toggle.textContent = `Scheduled ${mine.length}`;
    toggle.setAttribute("aria-expanded", String(opened));
    grid.hidden = !opened;
    const now = JSON.stringify([mine, getMeta().agents.map((a) => [a.id, a.name, a.color])]);
    if (now === drawn) return;
    drawn = now;
    list.replaceChildren(...mine.map(card));
    if (!mine.length) list.append(el("p", "colempty", "No schedules: + Schedule runs a task on a cadence."));
  }

  toggle.addEventListener("click", () => {
    opened = !opened;
    write(OPEN_KEY, opened || null);
    draw();
  });

  async function load() {
    try {
      schedules = (await api("/api/schedules")).schedules;
      draw();
    } catch (e) { say(`The schedules could not be read: ${e.message}`); }
  }
  /** The window of the schedule `key` names ("SCH-1"), from a task's window. */
  const openKey = (key) => { const s = schedules.find((x) => x.key === key); if (s) win.open(s); };
  return { box, draw, load, openKey };
}
