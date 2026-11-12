// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Charts drawn as inline SVG (terminum.md AC-151, AC-152): columns a band each, stacked, with a bar
// inside or a line across, a sparkline with its trend. No library: paths, rects, lines, text. Colors are
// CSS variables, so a theme's change restyles them without a redraw. A plot is one key stop: ← and →
// choose a band, Home/End, Enter opens it, Esc lets it go; the pointer hovers and presses, a finger taps
// (and drags along).
const NS = "http://www.w3.org/2000/svg";

/** An SVG element with `attrs`, into `parent`. */
export function svg(tag, attrs = {}, parent) {
  const e = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) if (v !== undefined && v !== null) e.setAttribute(k, String(v));
  parent?.append(e);
  return e;
}

/** A "nice" top for values up to `max` and its ticks (1, 2, 5 × 10ⁿ apart): the finest step that needs
 *  at most `most` + 1 of them over zero. */
export function niceTicks(max, most = 3) {
  if (!(max > 0)) return { top: 1, ticks: [0] };
  const pow = 10 ** Math.floor(Math.log10(max / (most + 1)));
  const step = [1, 2, 5, 10, 20].map((m) => m * pow).find((s) => Math.ceil(max / s - 1e-9) <= most + 1);
  const top = Math.ceil(max / step - 1e-9) * step;
  const ticks = [];
  for (let v = 0; v <= top + step / 2; v += step) ticks.push(Math.round(v * 1e6) / 1e6);
  return { top, ticks };
}

/** A column's part as a path: `w` wide and `h` tall under its top `y1`, its top corners rounded by `r` (0: square). */
function part(x, y1, w, h, r) {
  r = Math.min(r, w / 2, h);
  if (r <= 0) return `M${x},${y1}h${w}v${h}h${-w}z`;
  return `M${x},${y1 + h}v${r - h}a${r},${r} 0 0 1 ${r},${-r}h${w - 2 * r}a${r},${r} 0 0 1 ${r},${r}v${h - r}z`;
}

/** Columns over `bands` into `box`: each band {label (x), parts: [{value, color}], inner? ({value, color}: a
 *  narrower bar inside the column), faint?, band? (a backdrop: a weekend)}. Options: height, sqrt (a root scale,
 *  no axis), axis (y labels by `fmt`), line ({value, label}: a dashed line across), xlabels(i) → text or "",
 *  select(i | -1) on hover/focus/tap, open(i) on a press or Enter. The top of a column is rounded, the others
 *  dim while one is chosen. */
export function columns(box, bands, opts = {}) {
  const { height = 200, sqrt = false, axis = !sqrt, fmt = String, line = null, xlabels = () => "", select = () => {}, open = () => {} } = opts;
  const width = Math.max(120, box.clientWidth || 600);
  const left = axis ? 34 : 0, bottom = bands.some((_, i) => xlabels(i)) ? 18 : 0, top = 4;
  const plotW = width - left, plotH = height - bottom - top;
  const totals = bands.map((b) => b.parts.reduce((s, p) => s + p.value, 0));
  const max = Math.max(1, ...totals, ...bands.map((b) => b.inner?.value ?? 0), line?.value ?? 0);
  const { top: yTop, ticks } = sqrt ? { top: max, ticks: [] } : niceTicks(max);
  const y = (v) => top + plotH - (sqrt ? Math.sqrt(v / yTop) : v / yTop) * plotH;
  const s = svg("svg", { viewBox: `0 0 ${width} ${height}`, width: "100%", height, class: "chart", "aria-hidden": "true" });
  for (const t of ticks) {
    svg("line", { x1: left, x2: width, y1: y(t), y2: y(t), class: "grid" }, s);
    if (axis) svg("text", { x: left - 6, y: y(t) + 3, class: "ylab", "text-anchor": "end" }, s).textContent = fmt(t);
  }
  // a column at most 28px wide, the rest of its band a gap
  const band = plotW / Math.max(1, bands.length), w = Math.max(1, Math.min(28, band * 0.72)), gap = band - w;
  const marks = [];
  let lastLabel = -Infinity;
  bands.forEach((b, i) => {
    const x = left + i * band;
    if (b.band) svg("rect", { x, y: top, width: band, height: plotH, class: "backband" }, s);
    const g = svg("g", { class: `col${b.faint ? " faint" : ""}` }, s);
    const shown = b.parts.filter((p) => p.value > 0), r = w < 3 ? 0 : Math.min(4, w / 3);
    let base = 0;
    shown.forEach((p, k) => {
      const y0 = y(base), y1 = y(base + p.value), h = Math.max(1, y0 - y1 - (base > 0 ? 1 : 0));
      svg("path", { d: part(x + gap / 2, y1, w, h, k === shown.length - 1 ? r : 0), style: `fill:${p.color}` }, g);
      base += p.value;
    });
    // a light catch along the column's top, where it is wide enough to show one
    if (w >= 6 && y(0) - y(base) > 3) svg("path", { d: part(x + gap / 2, y(base), w, Math.min(3, r), r), class: "sheen" }, g);
    if (b.inner?.value > 0) {
      const iw = Math.max(1, w * 0.45), v = b.inner.value;
      svg("path", { d: part(x + (band - iw) / 2, y(v), iw, Math.max(1, y(0) - y(v)), Math.min(3, iw / 2)), class: "inner", style: `fill:${b.inner.color}` }, g);
    }
    const label = xlabels(i);
    if (label && x - lastLabel >= 44) {
      svg("text", { x: x + band / 2, y: height - 4, class: "xlab", "text-anchor": "middle" }, s).textContent = label;
      lastLabel = x;
    }
    marks.push(svg("rect", { x, y: 0, width: band, height: top + plotH, class: "hit" }, g));   // in its column: the chosen one stays lit
  });
  svg("line", { x1: left, x2: width, y1: y(0), y2: y(0), class: "base" }, s);
  if (line?.value > 0) {
    svg("line", { x1: left, x2: width, y1: y(line.value), y2: y(line.value), class: "usual" }, s);
    if (line.label) svg("text", { x: width - 2, y: y(line.value) - 4, class: "usuallab", "text-anchor": "end" }, s).textContent = line.label;
  }
  box.replaceChildren(s);
  pointer(box, s, marks, { select, open, at: (e) => {
    const r = s.getBoundingClientRect();
    return Math.min(marks.length - 1, Math.floor((((e.clientX - r.left) / r.width) * width - left) / band));
  } });
}

/** The keys, pointer and finger over a plot's `marks` (a grid of `cols` when given, else a row): `at(e)` is the
 *  mark under a pointer event or -1; select(i | -1) on hover, focus, a tap or a key, open(i) on a mouse's
 *  press or Enter (a finger's tap only chooses: a 3px column is no target). */
export function pointer(box, s, marks, { at, select = () => {}, open = () => {}, cols = 0 }) {
  let i = -1;
  const pick = (to) => { if (to === i) return; marks[i]?.classList.remove("on"); i = to; marks[i]?.classList.add("on"); select(i); };
  const under = (e) => { const to = at(e); return to >= 0 && to < marks.length ? to : -1; };
  s.addEventListener("pointermove", (e) => { if (e.pointerType === "mouse" || e.buttons) pick(under(e)); });
  s.addEventListener("pointerdown", (e) => { if (e.pointerType !== "mouse") pick(under(e)); });
  s.addEventListener("pointerleave", (e) => { if (e.pointerType === "mouse" && document.activeElement !== box) pick(-1); });
  s.addEventListener("click", (e) => { const to = under(e); if (to >= 0 && (e.pointerType === "mouse" || e.detail === 0)) open(to); });
  box.onkeydown = (e) => {
    const n = marks.length;
    const to = { ArrowLeft: i - 1, ArrowRight: i + 1, ArrowUp: cols ? i - cols : undefined, ArrowDown: cols ? i + cols : undefined, Home: 0, End: n - 1 }[e.key];
    if (to !== undefined) { e.preventDefault(); pick(i < 0 ? (e.key === "Home" ? 0 : n - 1) : Math.min(n - 1, Math.max(0, to))); }
    else if (e.key === "Enter" && i >= 0) { e.preventDefault(); open(i); }
    else if (e.key === "Escape" && i >= 0) { e.stopPropagation(); pick(-1); }
  };
  box.onblur = () => pick(-1);
}

/** The trend of `values` (null: not measured): a least-squares line over the measured ones but the last, which is
 *  the bucket going on. Null under 4 points; `dir` 1 rising, -1 falling, 0 when the line moves under 5% of the mean
 *  over the span; `at(i)` the line's value at index i. */
export function trend(values) {
  const pts = values.slice(0, -1).map((v, i) => [i, v]).filter(([, v]) => Number.isFinite(v));
  if (pts.length < 4) return null;
  const n = pts.length, mx = pts.reduce((a, [x]) => a + x, 0) / n, my = pts.reduce((a, [, v]) => a + v, 0) / n;
  const sxx = pts.reduce((a, [x]) => a + (x - mx) ** 2, 0), sxy = pts.reduce((a, [x, v]) => a + (x - mx) * (v - my), 0);
  const slope = sxx ? sxy / sxx : 0, change = slope * (pts.at(-1)[0] - pts[0][0]);
  const dir = Math.abs(change) < 0.05 * Math.abs(my) || change === 0 ? 0 : Math.sign(change);
  return { dir, from: pts[0][0], to: pts.at(-1)[0], at: (i) => my + slope * (i - mx) };
}

/** A sparkline of `values` (null: not measured, skipped) into `box`: one line in the accent, its last point a dot,
 *  its trend dotted: green when it goes the way `sense` says is better (1: up, -1: down), red when not, muted
 *  when flat or with no better way (0). */
export function spark(box, values, sense = 0) {
  const w = 120, h = 28, measured = values.filter(Number.isFinite), max = Math.max(1, ...measured);
  const s = svg("svg", { viewBox: `0 0 ${w} ${h}`, preserveAspectRatio: "none", class: "spark", "aria-hidden": "true" });
  const at = (i, v) => [(i / Math.max(1, values.length - 1)) * (w - 4) + 2, h - 3 - (Math.max(0, Math.min(max, v)) / max) * (h - 6)];
  const t = trend(values);
  if (t) {
    const way = t.dir * sense;
    const [x1, y1] = at(t.from, t.at(t.from)), [x2, y2] = at(t.to, t.at(t.to));
    svg("line", { x1, y1, x2, y2, class: `trend ${way > 0 ? "good" : way < 0 ? "bad" : "flat"}` }, s);
  }
  if (measured.length > 1) {
    const pts = values.map((v, i) => (Number.isFinite(v) ? at(i, v) : null)).filter(Boolean);
    svg("polyline", { points: pts.map((p) => p.join(",")).join(" "), class: "sparkline" }, s);
    const [x, y] = pts.at(-1);
    svg("circle", { cx: x, cy: y, r: 2, class: "sparkdot" }, s);
  }
  box.replaceChildren(s);
}
