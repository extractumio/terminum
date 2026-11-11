// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// What the Dashboard's tabs share (terminum.md AC-152, AC-171, AC-172, AC-176): a tile, a card with its plot and the line
// that says a column's values, a card of columns stacked by kind, a bucket's words by the period's unit, a legend,
// rows ranked.
import { el, projectColor } from "./listparts.js";
import { columns, spark } from "./charts.js";
import { WEEK, clock, count, dayDate, dayLabel, dotOf, monthOf, shortDay } from "./msgpage.js";

/** The span before, in the period's words. */
const BEFORE = { day: "the day before", "2w": "the 2 weeks before", month: "the 30 days before", quarter: "the 13 weeks before", year: "the year before" };

/** "▲ 12% vs the 30 days before"; small or missing earlier numbers say so plainly. No color for better or worse. */
export function delta(period, now, before, { small = 20, fmt = count } = {}) {
  if (before === null || before === undefined) return "no earlier data";
  if (now === before) return `same as ${BEFORE[period]}`;
  const arrow = now > before ? "▲" : "▼";
  if (before < small) return `${arrow} ${fmt(Math.abs(now - before))} vs ${BEFORE[period]}`;
  return `${arrow} ${Math.round(Math.abs(now / before - 1) * 100)}% vs ${BEFORE[period]}`;
}

/** A figure: its label, value, a line under it, a sparkline of `line`. */
export function tile(label, value, sub, line) {
  const t = el("div", "tile");
  t.append(el("div", "tlabel", label), el("div", "tvalue", value));
  if (sub) t.append(el("div", "tsub", sub));
  if (line) { const s = el("div", "tspark"); spark(s, line); t.append(s); }
  return t;
}

export const percent = (x) => `${Math.round(x * 100)}%`;
/** Parts that are something, joined by a middle dot. */
export const words = (...parts) => parts.filter(Boolean).join(" · ");
/** "<x> before" of the span before's figure `x`, when it has one. */
export const was = (x, fmt = percent) => (x === null || x === undefined ? "" : `${fmt(x)} before`);
/** A share's tile: `x` of 0…1 or null (said by `none`), `what` it is a share of, the span before's beside it. */
export const shareTile = (label, x, before, none, what) => tile(label, x === null ? "—" : percent(x), x === null ? none : words(what, was(before)));
/** The `kinds` ([key, word, color]) `counts` holds, each with its count: "made 2", "moved 1". */
export const kindWords = (kinds, counts) => kinds.filter(([k]) => counts[k]).map(([k, word]) => `${word} ${counts[k]}`);

/** A card: its title, a hint or a choice of `modes` ([key, word]; `mode` the pressed one, `onMode(key)`), then a
 *  line that says the chosen column (`say`, with a link `go` when asked), a legend when asked, a plot (one key
 *  stop, `label` for a reader), or with `list` rows ranked. `draw()` draws its plot at its width. */
export function card(id, title, { hint = "", hintTitle = "", wide = false, modes = null, mode = "", onMode = () => {}, label = "", list = false, legend = false, go = false } = {}) {
  const box = el("figure", `card${wide ? " wide" : ""}`);
  box.id = `c${id}`;
  const head = el("figcaption", "chead");
  head.append(el("h2", "", title));
  const out = { box, say: null, go: null, legend: null, plot: null, draw: () => {} };
  if (modes) {
    const seg = el("div", "seg");
    seg.setAttribute("role", "group");
    seg.setAttribute("aria-label", "Measure");
    for (const [key, word] of modes) {
      const b = el("button", "", word);
      b.type = "button";
      b.dataset.mode = key;
      b.setAttribute("aria-pressed", String(key === mode));
      b.addEventListener("click", () => onMode(key));
      seg.append(b);
    }
    head.append(seg);
  } else {
    const h = el("span", "chint", hint);
    if (hintTitle) h.title = hintTitle;
    head.append(h);
  }
  box.append(head);
  if (!list) {
    const line = el("div", "readout");
    out.say = el("span");
    out.say.setAttribute("aria-live", "polite");
    line.append(out.say);
    if (go) { out.go = el("a", "textkey", "Open messages"); out.go.hidden = true; line.append(out.go); }
    box.append(line);
  }
  if (legend) { out.legend = el("div", "legend"); box.append(out.legend); }
  out.plot = el("div", list ? "ranked" : "plot");
  out.plot.id = id;
  if (!list) {
    out.plot.tabIndex = 0;
    out.plot.setAttribute("role", "group");
    out.plot.setAttribute("aria-label", label || `${title}: ← and → choose a column`);
  }
  box.append(out.plot);
  return out;
}

/** The buckets' words by the unit (hour, day, week, month): `short(i)` under a column ("" where none stands),
 *  `long(i)` in the line over a chart, `faint(i)` (the one going on), `band(i)` (a weekend's day). */
export function bucketWords(data) {
  const b = data.buckets, unit = data.unit, last = b.length - 1;
  const hour = (i) => clock(b[i].hour * 60);
  const short = (i) => {
    if (unit === "hour") return b[i].hour % 3 === 0 ? hour(i) : "";
    if (unit === "month") return monthOf(b[i].from);
    if (unit === "week") return shortDay(b[i].from);
    if (b.length <= 14) return WEEK[(dayDate(b[i].from).getDay() + 6) % 7];
    return dayDate(b[i].from).getDay() === 1 || i === 0 ? shortDay(b[i].from) : "";
  };
  const long = (i) => {
    const soFar = i === last ? " so far" : "";
    if (unit === "hour") return `${dayLabel(b[i].from)} ${hour(i)}${soFar}`;
    if (unit === "month") return `${monthOf(b[i].from)} ${dayDate(b[i].from).getFullYear()}${soFar}`;
    if (unit === "week") return `${shortDay(b[i].from)} – ${shortDay(b[i].to)}${soFar}`;
    return `${dayLabel(b[i].from)}${soFar}`;
  };
  return { short, long, faint: (i) => i === last, band: (i) => unit === "day" && [0, 6].includes(dayDate(b[i].from).getDay()) };
}

/** A column a bucket for charts.js: `parts(d, i)` its stack, `dot(d)` its dot. */
export function bands(series, w, parts, dot = () => undefined) {
  return series.map((d, i) => ({ parts: parts(d, i), dot: dot(d), band: w.band(i), faint: w.faint(i) }));
}

/** A card of columns stacked by `kinds` ([key, word, color]) over each bucket's counts `of(d)`: the legend says
 *  `totals` (the period's count of each kind), the line a column's kinds, or `sum` with none chosen. */
export function stackCard(id, title, { data, w }, { kinds, of, totals, sum, hint = "", wide = false, height = 150 }) {
  const s = data.series;
  const c = card(id, title, { hint, legend: true, wide });
  const say = (i) => { c.say.textContent = i < 0 ? sum : words(w.long(i), ...kindWords(kinds, of(s[i]))); };
  legend(c.legend, kinds.map(([k, word, color]) => ({ color, name: word, num: count(totals[k] ?? 0) })));
  c.draw = () => {
    columns(c.plot, bands(s, w, (d) => kinds.map(([k, , color]) => ({ value: of(d)[k] ?? 0, color }))), { height, xlabels: w.short, select: say });
    say(-1);
  };
  return c;
}

/** A hue by rank for a stack's part that has no color of its own. */
export const rankHue = (i) => projectColor({ color: i });

/** A legend's chips: [{color, name, num?, href?, title?}]. */
export function legend(box, items) {
  box.replaceChildren(...items.map((it) => {
    const chip = el(it.href ? "a" : "span", "chip");
    chip.append(dotOf(it.color), el("span", "pname", it.name));
    if (it.num !== undefined) chip.append(el("span", "rnum", it.num));
    if (it.href) { chip.href = it.href; chip.title = it.title ?? ""; }
    return chip;
  }));
}

/** Rows ranked: [{name, n, color, num?, share?, parts?, open?, title?}], the widest bar the largest `n` (split by
 *  `parts` [{n, color}] when given); beside it its share
 *  of them all (or `share`) and `num` (or `fmt(n)`); the first `most`, the rest as Other. */
export function ranked(box, rows, { most = 8, empty = "Nothing in this period.", fmt = count } = {}) {
  const shown = rows.slice(0, most), rest = rows.slice(most);
  const sum = (list) => list.reduce((s, r) => s + r.n, 0);
  const total = sum(rows) || 1, max = Math.max(1e-9, ...shown.map((r) => r.n), sum(rest));
  const row = (r) => {
    const b = el(r.open ? "button" : "div", "rrow");
    if (r.open) { b.type = "button"; b.addEventListener("click", r.open); }
    if (r.title) b.title = r.title;
    const bar = el("span", "rbar");
    bar.style.setProperty("--w", `${(r.n / max) * 100}%`);
    bar.style.setProperty("--proj", r.color);
    if (r.parts) {
      // a bar split by its parts, each as wide as its count
      let at = 0;
      const stops = r.parts.map((p) => { const from = at; at += (p.n / max) * 100; return `${p.color} ${from}% ${at}%`; });
      bar.style.background = `linear-gradient(90deg, ${stops.join(", ")}, transparent ${at}%)`;
    }
    b.append(dotOf(r.color), el("span", "rname", r.name), bar, el("span", "rnum", r.share ?? `${Math.round((r.n / total) * 100)}%`), el("span", "rnum", r.num ?? fmt(r.n)));
    return b;
  };
  const list = shown.map(row);
  if (rest.length) list.push(row({ name: `Other (${rest.length})`, n: sum(rest), color: "var(--faint)" }));
  box.replaceChildren(...(list.length ? list : [el("p", "msay", empty)]));
}
