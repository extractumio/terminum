// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Dashboard's own charts (terminum.md AC-152), inline SVG as charts.js's: the working day (a day a
// column, midnight on top as a calendar's, a thin line from the first to the last event and the
// active stretches solid on it, light blue in the morning to dark blue at night), the hour × weekday heat map.
import { pointer, svg } from "./charts.js";
import { WEEK } from "./msgpage.js";

/** Working days: `series` [{spans: [[from, to] minutes]}], `label(i)` under a column, `night` [from, to] hours
 *  (terminumd's: shaded, and the morning starts at its end); select(i) as charts.js's. */
export function workingDays(box, series, { label = () => "", select = () => {}, night: [dusk, dawn] = [22, 6] } = {}) {
  // how late a minute is: 0 when the night ends, 100 at midnight and through the night after it
  const morning = dawn * 60, lateness = (m) => (m < morning ? 100 : Math.round(((m - morning) / (1440 - morning)) * 100));
  const width = Math.max(160, box.clientWidth || 600), height = 220, left = 34, bottom = 18, top = 4;
  const plotW = width - left, plotH = height - top - bottom, y = (min) => top + (min / 1440) * plotH;
  const s = svg("svg", { viewBox: `0 0 ${width} ${height}`, width: "100%", height, class: "chart", "aria-hidden": "true" });
  for (const h of [0, 6, 12, 18, 24]) {
    svg("line", { x1: left, x2: width, y1: y(h * 60), y2: y(h * 60), class: "grid" }, s);
    svg("text", { x: left - 6, y: y(h * 60) + 3, class: "ylab", "text-anchor": "end" }, s).textContent = `${String(h).padStart(2, "0")}:00`;
  }
  // the night (the Off hours tile's), shaded
  for (const [a, b] of [[0, morning], [dusk * 60, 1440]]) svg("rect", { x: left, y: y(a), width: plotW, height: y(b) - y(a), class: "night" }, s);
  const band = plotW / Math.max(1, series.length), w = Math.max(2, Math.min(10, band * 0.6));
  const marks = [];
  let lastLabel = -Infinity;
  series.forEach((d, i) => {
    const cx = left + i * band + band / 2;
    if (d.spans.length) {
      const first = d.spans[0][0], last = d.spans.at(-1)[1];
      svg("line", { x1: cx, x2: cx, y1: y(first), y2: y(Math.min(1440, last)), class: "span" }, s);
      // a stretch an hour a piece, each the blue of its hour
      for (const [a, b] of d.spans) {
        const end = Math.min(1440, Math.max(b, a + 1));
        for (let m = a, to; m < end; m = to) {
          to = Math.min(end, (Math.floor(m / 60) + 1) * 60);
          svg("rect", { x: cx - w / 2, y: y(m), width: w, height: Math.max(1.5, y(to) - y(m)), rx: 1, class: "stretch",
            style: `fill:color-mix(in oklch, var(--day-late) ${lateness(m)}%, var(--day-early))` }, s);
        }
      }
    }
    const t = label(i);
    if (t && cx - lastLabel >= 44) { svg("text", { x: cx, y: height - 4, class: "xlab", "text-anchor": "middle" }, s).textContent = t; lastLabel = cx; }
    marks.push(svg("rect", { x: left + i * band, y: 0, width: band, height: top + plotH, class: "hit" }, s));
  });
  box.replaceChildren(s);
  pointer(box, s, marks, { select, at: (e) => { const r = s.getBoundingClientRect(); return Math.floor(((e.clientX - r.left) / r.width * width - left) / band); } });
}

/** The heat map: `heat[weekday 0 = Sunday][hour]` minutes, Monday first; select({wd, hour} | null). */
export function heatMap(box, heat, { select = () => {} } = {}) {
  const narrow = (box.clientWidth || 600) < 420;
  const cols = narrow ? 12 : 24, per = 24 / cols;       // a phone: two hours a cell
  const width = Math.max(200, box.clientWidth || 600), left = 34, top = 14, cell = (width - left) / cols, h = Math.min(22, cell * 0.9);
  const height = top + h * 7 + 4;
  const rows = [1, 2, 3, 4, 5, 6, 0].map((wd) => Array.from({ length: cols }, (_, c) => heat[wd].slice(c * per, (c + 1) * per).reduce((a, b) => a + b, 0)));
  const values = rows.flat().filter((v) => v > 0).sort((a, b) => a - b);
  const q = (p) => values[Math.min(values.length - 1, Math.floor(p * values.length))] ?? 0;
  const steps = [q(0.2), q(0.4), q(0.6), q(0.8)];
  const level = (v) => (v <= 0 ? 0 : 1 + steps.filter((t) => v > t).length);
  const s = svg("svg", { viewBox: `0 0 ${width} ${height}`, width: "100%", height, class: "chart heat", "aria-hidden": "true" });
  for (let c = 0; c < cols; c += 3) svg("text", { x: left + c * cell + 1, y: 10, class: "xlab" }, s).textContent = String(c * per).padStart(2, "0");
  const marks = [];
  rows.forEach((row, r) => {
    svg("text", { x: left - 6, y: top + r * h + h / 2 + 3, class: "ylab", "text-anchor": "end" }, s).textContent = WEEK[r];
    row.forEach((v, c) => {
      const m = svg("rect", { x: left + c * cell + 1, y: top + r * h + 1, width: Math.max(1, cell - 2), height: Math.max(1, h - 2), rx: 2, class: `cell l${level(v)}` }, s);
      m.dataset.at = `${r},${c}`;
      marks.push(m);
    });
  });
  box.replaceChildren(s);
  const info = (i) => (i < 0 ? null : { wd: Math.floor(i / cols), hour: (i % cols) * per, hours: per, minutes: rows[Math.floor(i / cols)][i % cols] });
  pointer(box, s, marks, { cols, select: (i) => select(info(i)), at: (e) => marks.indexOf(document.elementFromPoint(e.clientX, e.clientY)?.closest?.("rect.cell")) });
}
