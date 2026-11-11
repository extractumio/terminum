// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Dashboard's You tab (terminum.md AC-152): how much the user works here, how engaged and how loaded:
// active time by project, the minutes of it they acted or read in, the working day, the week's hours, the
// sessions juggled and the switches between them, what they did on the board, how long tasks waited for them. terminumd's numbers; nothing is added up here.
import { columns } from "./charts.js";
import { heatMap, workingDays } from "./dashcharts.js";
import { bands, card, delta, kindWords, legend, percent, rankHue, ranked, shareOf, shareTile, stackCard, tile, was, words } from "./dashkit.js";
import { plural } from "./listparts.js";
import { NONE, WEEK, clock, count, dayLabel, hours, inkOf, projectLabel } from "./msgpage.js";

/** What the human does on the board: the events' kinds, their words and colors, in the stack's order. */
const BOARD = [["made", "made", rankHue(1)], ["move", "moved", rankHue(4)], ["comment", "commented", rankHue(0)], ["edit", "edited", rankHue(2)], ["archive", "archived", "var(--faint)"]];

/** An active minute by what the user did in it, in the stack's order: the numbers' key, its word, its color. */
const MINUTES = [["acting", "acting", rankHue(2)], ["reading", "reading", rankHue(1)], ["watching", "watching", "var(--faint)"]];
const PAUSE = "A pause ends a stretch when over 5 minutes, or over 15 with a page in front through it; before the pages counted, over 15";

/** A project's color: the session list's; one no longer there takes a hue by its rank, so the stack's
 *  parts stay apart; Other is faint, no project muted. */
function stackColor(data, id) {
  if (id === "other") return "var(--faint)";
  const i = data.projects.findIndex((x) => x.id === id), p = data.projects[i];
  if (!p || !p.id || (p.color !== null && p.color !== undefined)) return inkOf(p);
  return rankHue(i);
}

function tiles({ data, period }) {
  const a = data.tiles, b = data.before, s = data.series;
  const d = (now, before, opts) => delta(period, now, before, opts);
  const time = { small: 7_200_000, fmt: hours }, few = { small: 1000, fmt: String };
  const unit = data.unit === "hour" ? "hour" : "day";
  return [
    tile("Active time", hours(a.active_ms), words(a.active_units ? `${hours(a.active_per_unit_ms)} an active ${unit}` : "", d(a.active_ms, b?.active_ms, time)), s.map((x) => x.active_ms)),
    tile(`Active ${unit}s`, `${a.active_units} of ${a.units}`, d(a.active_units, b?.active_units, few)),
    tile("Engagement", shareOf(a.engaged?.share),
      a.engaged ? words(`acting or reading in ${count(a.engaged.acting + a.engaged.reading)} of ${count(a.engaged.active)} active minutes`, was(b?.engaged?.share)) : "no active time since the pages count",
      s.map((x) => x.engaged?.share ?? 0)),
    tile("Longest stretch", a.longest_ms ? hours(a.longest_ms) : "—", words("no long pause", b ? d(a.longest_ms, b.longest_ms, time) : "")),
    tile("Switches an active hour", a.active_ms ? a.switches_per_hour.toFixed(1) : "—",
      words(`${count(a.switches)} between sessions`, a.peak ? `most sessions in one hour: ${a.peak}` : "", b ? d(Math.round(a.switches_per_hour * 10), Math.round(b.switches_per_hour * 10), { small: 1e9, fmt: (x) => (x / 10).toFixed(1) }) : ""),
      s.map((x) => x.switches)),
    tile("Board actions", count(a.board), words(kindWords(BOARD, a.board_by).join(", "), d(a.board, b?.board)), s.map((x) => x.board_n)),
    tile("Waited for you", count(a.asks), a.asks ? words(a.answer_ms === null ? "" : `moved on after ${hours(a.answer_ms)} (median)`, a.waiting ? `${a.waiting} still wait` : "") : "no task asked for you"),
    shareTile("Off hours", a.off, b?.off, "no active time", "of active time on weekends or 22:00–06:00"),
    shareTile("From the phone", a.phone, b?.phone, "nothing sent from a page", "of what the pages sent"),
  ];
}

function activeCard(ctx) {
  const { data, w } = ctx, s = data.series, stack = [...data.stacked, "other"];
  const c = card("active", "Active time", { wide: true, hint: "by project", legend: true, go: true, label: "Active time: ← and → choose a column, Enter opens its messages" });
  const open = (i) => ctx.historyUrl(data.buckets[i].from, data.buckets[i].to);
  const say = (i) => {
    c.go.hidden = i < 0 || !s[i].sent;
    if (i < 0) { c.say.textContent = `${hours(data.tiles.active_ms)} active`; return; }
    const parts = Object.entries(s[i].by).sort((x, y) => y[1] - x[1]).slice(0, 3).map(([id, ms]) => `${id === "other" ? "Other" : projectLabel(data.projects.find((p) => p.id === id) ?? { id })} ${hours(ms)}`);
    c.say.textContent = words(w.long(i), hours(s[i].active_ms), ...parts);
    c.go.href = open(i);
  };
  c.draw = () => {
    // in hours, or minutes while no column holds an hour: the axis' steps are round then
    const tall = s.some((d) => d.active_ms >= 3_600_000), per = tall ? 3_600_000 : 60_000;
    columns(c.plot, bands(s, w, (d) => stack.map((id) => ({ value: (d.by[id] ?? 0) / per, color: stackColor(data, id) }))),
      { height: 220, fmt: (v) => `${v} ${tall ? "h" : "min"}`, xlabels: w.short, select: say, open: (i) => { if (s[i].sent) location.href = open(i); } });
    say(-1);
  };
  legend(c.legend, data.stacked.map((id) => data.projects.find((p) => p.id === id)).filter(Boolean)
    .map((p) => ({ color: stackColor(data, p.id), name: projectLabel(p), num: hours(p.n), href: ctx.projectHref(p.id || NONE), title: "Only this project" }))
    .concat(data.other_ms ? [{ color: "var(--faint)", name: "Other", num: hours(data.other_ms) }] : []));
  return c;
}

function engagedCard({ data, w }) {
  const s = data.series, t = data.tiles;
  const c = card("engaged", "Engagement", { hint: "active minutes by what you did", legend: true,
    hintTitle: "Acting: a minute with a key, a press, a drag or something sent. Reading: one you only scrolled in. Watching: active time with neither" });
  const say = (i) => {
    const m = i < 0 ? t.engaged : s[i].engaged;
    c.say.textContent = words(i < 0 ? "" : w.long(i), ...(m ? [`${percent(m.share)} engaged`, ...MINUTES.map(([k, word]) => `${word} ${count(m[k])} min`)] : ["not measured"]));
  };
  legend(c.legend, MINUTES.map(([k, word, color]) => ({ color, name: word, num: `${count(t.engaged?.[k] ?? 0)} min` })));
  c.draw = () => {
    columns(c.plot, bands(s, w, (d) => MINUTES.map(([k, , color]) => ({ value: d.engaged?.[k] ?? 0, color }))), { height: 150, fmt: (v) => `${v} min`, xlabels: w.short, select: say });
    say(-1);
  };
  return c;
}

function daysCard({ data, w }) {
  const s = data.series, t = data.tiles;
  const c = card("days", "Working day", { hint: "first to last, active solid", hintTitle: `From the first to the last thing done; solid: active stretches. ${PAUSE}` });
  const say = (i) => {
    const d = s[i];
    c.say.textContent = i < 0 ? (t.starts === null ? "No active day in this period" : `Usually ${clock(t.starts)} to ${clock(t.ends)}`)
      : d.spans.length ? words(w.long(i), `${clock(d.spans[0][0])}–${clock(d.spans.at(-1)[1])}`, `${hours(d.active_ms)} active`) : words(w.long(i), "nothing");
  };
  c.draw = () => { workingDays(c.plot, s, { label: w.short, select: say }); say(-1); };
  return c;
}

function heatCard({ data }) {
  const c = card("heat", "When you work", { hint: "active minutes, an average week", label: "Hour by weekday: arrows choose an hour" });
  const say = (x) => {
    c.say.textContent = x ? words(`${WEEK[x.wd]} ${clock(x.hour * 60)}–${clock((x.hour + x.hours) * 60)}`, `${x.minutes} min`) : "Darker: more active minutes in that hour";
  };
  c.draw = () => { heatMap(c.plot, data.heat, { select: say }); say(null); };
  return c;
}

const switches = (n) => (n === 1 ? "1 switch" : `${count(n)} switches`);

function jugglingCard({ data, w }) {
  const s = data.series, t = data.tiles;
  const c = card("juggling", "Sessions at once", { hint: "columns: sessions, dots: most in an hour", hintTitle: "Columns: the sessions you wrote to in that column's time; dots: the most in one hour of it" });
  const say = (i) => {
    c.say.textContent = i < 0 ? words(`Median ${t.sessions ?? 0} a day`, `most in an hour ${t.peak}`, switches(t.switches))
      : words(w.long(i), plural(s[i].sessions, "session"), `most in an hour ${s[i].peak}`, switches(s[i].switches));
  };
  c.draw = () => {
    columns(c.plot, bands(s, w, (d) => [{ value: d.sessions, color: "color-mix(in srgb, var(--ring) 45%, transparent)" }], (d) => d.peak), { height: 150, xlabels: w.short, select: say });
    say(-1);
  };
  return c;
}

function projectsCard(ctx) {
  const { data } = ctx, b = data.buckets;
  const c = card("projects", "Projects", { hint: "active time", list: true });
  ranked(c.plot, data.projects.map((p) => ({ name: projectLabel(p), n: p.n, color: stackColor(data, p.id), num: hours(p.n), title: "Its messages in this period",
    open: () => { location.href = ctx.historyUrl(b[0].from, b.at(-1).to, p.id || NONE); } })), { empty: "No active time in this period." });
  return c;
}

/** The tab: its tiles and its cards, each with `draw()` for a new width. */
export function you(ctx) {
  const { data } = ctx, t = data.tiles;
  const board = stackCard("board", "On the board", ctx, { kinds: BOARD, of: (d) => d.board, totals: t.board_by, sum: plural(t.board, "action"), hint: "what you did to tasks" });
  // what the pages count begins inside the period: the figures before that day are measured from less
  const from = data.measured_from;
  const measured = !from ? " Engagement shows once a page has counted what you do." : from > data.buckets[0].from ? ` Presses, keys and scrolling are counted from ${dayLabel(from)}: engagement starts there, and nothing is compared across it.` : "";
  return { tiles: tiles(ctx), cards: [activeCard(ctx), engagedCard(ctx), data.unit === "day" ? daysCard(ctx) : null, data.unit === "hour" ? null : heatCard(ctx), projectsCard(ctx), jugglingCard(ctx), board].filter(Boolean),
    note: `You only: what Dispatcher and the agents do is not counted. This Mac's time.${measured}`, kept: "typing counted from" };
}
