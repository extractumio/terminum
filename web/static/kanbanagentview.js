// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// An agent's subscription and work (terminum-dispatcher.md AC-168, AC-143, AC-138): two stripes on its rectangle,
// its kind's windows as terminumd last heard them; in its window the same with when each resets, its tokens, and
// each live session with its tasks as links. terminumd's numbers; a window not known, reset or old says so.
import { el } from "./listparts.js";
import { agentName } from "./nav.js";
import { button, countOf, statusBox } from "./kanbanutil.js";

const HOUR = 3600, WEEK = 7 * 24 * 60;
const FINISHED = ["DONE", "FAILED"];

/** A window's name by its length: 5h, week, 1d… */
function label(minutes) {
  if (minutes === WEEK) return "week";
  return minutes % 1440 === 0 ? `${minutes / 1440}d` : `${Math.round(minutes / 60)}h`;
}

const DAY_TIME = new Intl.DateTimeFormat("en", { weekday: "short", hour: "2-digit", minute: "2-digit", hour12: false });
const dayTime = (s) => DAY_TIME.format(s * 1000);

/** The rows its stripes show at `now` (seconds): its windows, and a five-hour and a week row when the reading lacks one. */
function rows(reading, now) {
  const got = (reading?.windows ?? []).slice().sort((a, b) => a.minutes - b.minutes);
  const missing = reading ? "none reported" : "not known";
  const out = got.map((w) => {
    const reset = w.resets_at != null && w.resets_at <= now;
    return { label: label(w.minutes), used: reset ? null : w.used, note: reset ? "reset" : `${Math.round(w.used)}%`, resets: reset ? null : w.resets_at };
  });
  if (!got.some((w) => w.minutes < 1440)) out.unshift({ label: "5h", used: null, note: missing });
  if (!got.some((w) => w.minutes >= 1440)) out.push({ label: "week", used: null, note: missing });
  return out;
}

/** Its stripes: on its rectangle, or `full` in its window with when each resets. */
export function stripes(kind, reading, now, full = false) {
  const old = reading && now - reading.at > HOUR;    // over an hour: its stripes fade
  const box = el("span", `agsubs${old ? " stale" : ""}${full ? " full" : ""}`);
  const from = reading ? `read ${dayTime(reading.at)} from a ${agentName(kind)} session` : `no ${agentName(kind)} session has said it yet`;
  for (const r of rows(reading, now)) {
    const line = el("span", "agsub"), track = el("span", "agtrack"), fill = el("span", "agfill"), pct = r.used ?? 0;
    fill.style.width = `${Math.min(100, pct)}%`;
    track.classList.toggle("high", pct >= 90);
    track.append(fill);
    const words = full && r.resets ? `${r.note} · resets ${dayTime(r.resets)}` : r.note;
    line.append(el("span", "agsublabel", r.label), track, el("span", "agsubpct", words));
    line.title = `${r.label}: ${r.used == null ? r.note : `${r.note} used`}${r.resets ? `, resets ${dayTime(r.resets)}` : ""}; ${from}`;
    box.append(line);
  }
  return box;
}

/** Its last 7 and 30 days (AC-143): of its limit when it has one, else the tokens; past a limit in the danger color. */
function spent(a) {
  const s = a.spent, part = (word, used, limit) => (limit ? `${word} ${Math.round((used / (limit * 1e6)) * 100)}%` : `${word} ${countOf(used)}`);
  const line = el("p", "agspent", `Tokens: ${part("week", s.week, a.week_limit)} · ${part("month", s.month, a.month_limit)}`);
  line.classList.toggle("over", Boolean((a.week_limit && s.week > a.week_limit * 1e6) || (a.month_limit && s.month > a.month_limit * 1e6)));
  line.title = `${s.week.toLocaleString("en")} tokens in 7 days, ${s.month.toLocaleString("en")} in 30, of every project: terminum's own count`;
  return line;
}

/** How a reading comes, when none has. */
const howTo = (kind) => (kind === "codex" ? "Codex's limits come when one of its turns ends." : "Claude Code says its limits only to its status line: put terminum statusline there, before your own command if you have one (docs/USAGE.md).");

/** A task as a link: its status and ID, then its title; a press opens its window. */
function taskLink(t, openTask) {
  const li = el("li", "agtask");
  li.append(statusBox(t, openTask), button("linklike", t.title, () => openTask(t.id)));
  return li;
}

/** One session: its project, what it does, Open and Let go; under it what it works on and what it finished. */
function session(s, { projectName, doing, openTask, openSession, letGo }) {
  const li = el("li", "agsession"), head = el("div", "agsesshead");
  head.append(el("span", "agsessname", [projectName(s.project), doing(s)].filter(Boolean).join(" · ")));
  if (s.key) head.append(button(null, "Open session", () => openSession(s.key)));
  head.append(button(null, "Let go", () => letGo(s, li)));
  li.append(head);
  const list = s.list ?? [];
  for (const [title, tasks] of [["Working on", list.filter((t) => !FINISHED.includes(t.status))], ["Finished, session open", list.filter((t) => FINISHED.includes(t.status))]]) {
    if (!tasks.length) continue;
    const ul = el("ul", "agtasks");
    ul.setAttribute("aria-label", title);
    ul.append(...tasks.map((t) => taskLink(t, openTask)));
    li.append(el("h4", null, title), ul);
  }
  if (!list.length) li.append(el("p", "hint", "+ Session: no task"));
  return li;
}

/** What its window shows over its settings: `sessions` its live ones; `ctx`: projectName, doing, openTask, openSession, letGo. */
export function agentView(a, reading, sessions, ctx) {
  const now = Date.now() / 1000, box = el("div", "agview");
  box.append(el("h3", null, `${agentName(a.kind)} subscription`), stripes(a.kind, reading, now, true));
  box.append(el("p", "hint", reading ? `Read ${dayTime(reading.at)}; one reading for every ${agentName(a.kind)} agent.` : howTo(a.kind)));
  box.append(spent(a), el("h3", null, "Sessions"));
  if (!sessions.length) box.append(el("p", "hint", "No live session."));
  else {
    const ul = el("ul", "agsessions");
    ul.append(...sessions.map((s) => session(s, ctx)));
    box.append(ul);
  }
  return box;
}
