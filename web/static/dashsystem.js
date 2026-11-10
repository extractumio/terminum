// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Dashboard's System tab (terminum.md AC-170): the sessions Dispatcher started, how their tasks ended, what
// failed and why, the agents' load, the schedules' firings. terminumd's numbers; nothing is added up here.
import { columns } from "./charts.js";
import { bands, card, delta, kindWords, rankHue, ranked, shareTile, stackCard, tile, was, words } from "./dashkit.js";
import { agentColor, agentLabel } from "./kanbanutil.js";
import { plural } from "./listparts.js";
import { count, hours } from "./msgpage.js";

/** How a task of a session ended, in the stack's order: the numbers' key, its word, its color. */
const ENDS = [["done", "done", "var(--ok-fill)"], ["asked", "asked you", "var(--waiting)"], ["failed", "failed", "var(--danger)"]];
/** A schedule's firings by state. */
const FIRES = [["started", "started", "var(--ok-fill)"], ["skipped", "skipped", "var(--faint)"], ["called off", "called off", rankHue(4)], ["failed", "failed", "var(--danger)"]];
/** What a failure was of: its word and color. */
const KINDS = { task: ["Task", "var(--danger)"], write: ["Write", "var(--waiting)"], schedule: ["Schedule", "var(--faint)"] };

function tiles({ data, period }) {
  const a = data.tiles, b = data.before, s = data.series;
  const d = (now, before, opts) => delta(period, now, before, opts);
  return [
    tile("Sessions started", count(a.sessions), d(a.sessions, b?.sessions), s.map((x) => x.started)),
    tile("Tasks done", count(a.done), words(a.asked ? `${a.asked} asked you` : "", d(a.done, b?.done)), s.map((x) => x.done)),
    shareTile("Failure rate", a.failure_rate, b?.failure_rate, "no task ended", `${a.failed} failed of ${a.ended} ended`),
    tile("Failures", count(a.errors), words(`tasks ${a.failed}`, `writes ${a.failed_writes}`, `firings ${a.failed_firings}`, b ? d(a.errors, b.errors) : ""), s.map((x) => x.errors)),
    tile("Agents' load", `${a.load}%`, words(`${hours(a.busy_ms)} busy`, `${plural(a.slots, "slot")}, as set today`, was(b?.load, (x) => `${x}%`)), s.map((x) => x.load)),
    tile("A session lasts", a.session_ms === null ? "—" : hours(a.session_ms), a.session_ms === null ? "none ended" : words("median, start to close", was(b?.session_ms, hours))),
    tile("Schedules fired", count(a.fires.started), words(...kindWords(FIRES.slice(1), a.fires)) || "none skipped or failed"),
  ];
}

function loadCard({ data, w }) {
  const s = data.series, t = data.tiles;
  const c = card("load", "Agents' load", { hint: "busy slots, %", hintTitle: "The time sessions were open, of all the agents' slots as set today; a session waiting for you holds its slot" });
  const say = (i) => { c.say.textContent = i < 0 ? words(`${t.load}% of ${plural(t.slots, "slot")}`, `${hours(t.busy_ms)} busy`) : words(w.long(i), `${s[i].load}%`, `${s[i].started} started`); };
  c.draw = () => {
    columns(c.plot, bands(s, w, (d) => [{ value: d.load, color: "var(--ring)" }]), { height: 150, fmt: (v) => `${v}%`, xlabels: w.short, select: say });
    say(-1);
  };
  return c;
}

function failuresCard({ data }) {
  const c = card("failures", "Failures", { hint: "by reason", list: true });
  ranked(c.plot, data.failures.map((f) => ({ name: `${KINDS[f.kind][0]}: ${f.why}`, title: f.why, n: f.n, color: KINDS[f.kind][1] })), { most: 10, empty: "Nothing failed in this period." });
  return c;
}

function agentsCard({ data }) {
  const c = card("agents", "Agents", { hint: "load, sessions", list: true });
  ranked(c.plot, data.agents.map((a) => ({ name: agentLabel(a), n: a.busy_ms, color: agentColor(a), share: `${a.load}%`, num: String(a.sessions),
    title: words(`${hours(a.busy_ms)} busy`, `${a.slots} at once`, `${plural(a.sessions, "session")} started`) })), { empty: "No session in this period." });
  return c;
}

/** The tab: its tiles and its cards, each with `draw()` for a new width. */
export function system(ctx) {
  const t = ctx.data.tiles;
  return { tiles: tiles(ctx), cards: [
    stackCard("ends", "How tasks ended", ctx, { kinds: ENDS, of: (d) => d, totals: t, sum: words(...ENDS.map(([k, word]) => `${count(t[k])} ${word}`)), hint: "moved by Dispatcher or an agent", wide: true, height: 200 }),
    failuresCard(ctx), agentsCard(ctx), loadCard(ctx),
    stackCard("fires", "Schedules' firings", ctx, { kinds: FIRES, of: (d) => d.fires, totals: t.fires, sum: `${count(t.fires.started)} started`, hint: "by state" }),
  ], note: "The sessions Dispatcher started. This Mac's time." };
}
