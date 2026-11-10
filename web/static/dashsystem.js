// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Dashboard's System tab (terminum.md AC-170): the sessions Dispatcher started, how their tasks ended, what
// failed and why, the agents' load, the schedules' firings. terminumd's numbers; nothing is added up here.
import { columns } from "./charts.js";
import { bands, card, delta, legend, percent, rankHue, ranked, tile, words } from "./dashkit.js";
import { agentColor } from "./kanbanutil.js";
import { count, hours } from "./msgpage.js";

/** How a task of a session ended, in the stack's order: the series' key, its word, its color. */
const ENDS = [["done", "done", "var(--ok-fill)"], ["asked", "asked you", "var(--waiting)"], ["failed", "failed", "var(--danger)"]];
/** A schedule's firings by state. */
const FIRES = [["started", "started", "var(--ok-fill)"], ["skipped", "skipped", "var(--faint)"], ["called off", "called off", rankHue(4)], ["failed", "failed", "var(--danger)"]];
/** What a failure was of. */
const KINDS = { task: "Task", write: "Write", schedule: "Schedule" };

function tiles({ data, period }) {
  const a = data.tiles, b = data.before, s = data.series;
  const d = (now, before, opts) => delta(period, now, before, opts);
  return [
    tile("Sessions started", count(a.sessions), d(a.sessions, b?.sessions), s.map((x) => x.started)),
    tile("Tasks done", count(a.done), words(a.asked ? `${a.asked} asked you` : "", d(a.done, b?.done)), s.map((x) => x.done)),
    tile("Failure rate", a.failure_rate === null ? "—" : percent(a.failure_rate), a.failure_rate === null ? "no task ended" : words(`${a.failed} failed of ${a.done + a.failed} ended`, b?.failure_rate !== null && b?.failure_rate !== undefined ? `${percent(b.failure_rate)} before` : "")),
    tile("Failures", count(a.errors), words(`tasks ${a.failed}`, `writes ${a.failed_writes}`, `firings ${a.failed_firings}`, b ? d(a.errors, b.errors) : ""), s.map((x) => x.errors)),
    tile("Agents' load", `${a.load}%`, words(`${hours(a.busy_ms)} busy`, `${a.slots} slots, as set today`, b ? `${b.load}% before` : ""), s.map((x) => x.load)),
    tile("A session lasts", a.session_ms === null ? "—" : hours(a.session_ms), a.session_ms === null ? "none ended" : words("median, start to close", b?.session_ms ? `${hours(b.session_ms)} before` : "")),
    tile("Schedules fired", count(a.fires.started), words(...FIRES.slice(1).filter(([k]) => a.fires[k]).map(([k, word]) => `${word} ${a.fires[k]}`)) || "none skipped or failed"),
  ];
}

/** A card of columns stacked by `kinds` over the series' `of(d)`; the line says a column's parts. */
function stackCard(id, title, hint, kinds, of, { data, w }, sum) {
  const s = data.series;
  const c = card(id, title, { hint, legend: true, wide: id === "ends" });
  const say = (i) => {
    c.say.textContent = i < 0 ? sum : words(w.long(i), ...kinds.filter(([k]) => of(s[i])[k]).map(([k, word]) => `${word} ${of(s[i])[k]}`)) || w.long(i);
  };
  legend(c.legend, kinds.map(([k, word, color]) => ({ color, name: word, num: count(s.reduce((n, d) => n + (of(d)[k] ?? 0), 0)) })));
  return { box: c.box, draw: () => {
    columns(c.plot, bands(s, w, (d) => kinds.map(([k, , color]) => ({ value: of(d)[k] ?? 0, color }))), { height: id === "ends" ? 200 : 150, xlabels: w.short, select: say });
    say(-1);
  } };
}

function loadCard({ data, w }) {
  const s = data.series, t = data.tiles;
  const c = card("load", "Agents' load", { hint: "busy slots, %", hintTitle: "The time sessions were open, of all the agents' slots as set today; a session waiting for you holds its slot" });
  const say = (i) => { c.say.textContent = i < 0 ? words(`${t.load}% of ${t.slots} slots`, `${hours(t.busy_ms)} busy`) : words(w.long(i), `${s[i].load}%`, `${s[i].started} started`); };
  return { box: c.box, draw: () => {
    columns(c.plot, bands(s, w, (d) => [{ value: d.load, color: "var(--ring)" }]), { height: 150, fmt: (v) => `${v}%`, xlabels: w.short, select: say });
    say(-1);
  } };
}

function failuresCard({ data }) {
  const c = card("failures", "Failures", { hint: "by reason", list: true });
  ranked(c.plot, data.failures.map((f) => ({ name: `${KINDS[f.kind]}: ${f.why}`, title: f.why, n: f.n, color: f.kind === "task" ? "var(--danger)" : f.kind === "write" ? "var(--waiting)" : "var(--faint)" })),
    { most: 10, empty: "Nothing failed in this period." });
  return { box: c.box, draw: () => {} };
}

function agentsCard({ data }) {
  const c = card("agents", "Agents", { hint: "load, sessions", list: true });
  ranked(c.plot, data.agents.map((a) => ({ name: a.name || "Agent gone", n: a.busy_ms, color: agentColor(a), share: `${a.load}%`, num: String(a.sessions),
    title: words(`${hours(a.busy_ms)} busy`, `${a.slots} at once`, `${a.sessions} sessions started`) })), { empty: "No session in this period." });
  return { box: c.box, draw: () => {} };
}

/** The tab: its tiles and its cards, each with `draw()` for a new width. */
export function system(ctx) {
  const t = ctx.data.tiles;
  return { tiles: tiles(ctx), cards: [
    stackCard("ends", "How tasks ended", "moved by Dispatcher or an agent", ENDS, (d) => d, ctx, words(`${count(t.done)} done`, `${count(t.asked)} asked you`, `${count(t.failed)} failed`)),
    failuresCard(ctx), agentsCard(ctx), loadCard(ctx),
    stackCard("fires", "Schedules' firings", "by state", FIRES, (d) => d.fires, ctx, `${count(t.fires.started)} started`),
  ], note: "The sessions Dispatcher started. This Mac's time." };
}
