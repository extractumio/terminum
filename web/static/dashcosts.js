// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Dashboard's Costs tab (terminum.md AC-171): what the sessions Dispatcher started spent: dollars at list
// prices and tokens by agent, project and model, and each agent's use against its limits. terminumd's numbers.
import { columns } from "./charts.js";
import { bands, card, delta, dollars, legend, ranked, tile, words } from "./dashkit.js";
import { agentColor, countOf } from "./kanbanutil.js";
import { NONE, inkOf, projectLabel } from "./msgpage.js";

/** What the charts measure: kept while the page is open. */
let mode = "usd";

const tokens = (n) => `${countOf(n)} tokens`;
/** A row's dollars, or that none are known: a model with no price is never $0. */
const priced = (r) => (r.unpriced && !r.usd ? "no price" : dollars(r.usd));
const agentInk = (data, id) => (id === "other" ? "var(--faint)" : agentColor(data.agents.find((a) => a.id === id) ?? {}));

function tiles({ data, period }) {
  const a = data.tiles, b = data.before, s = data.series;
  return [
    tile("Spent", dollars(a.usd), words("at API list prices", delta(period, Math.round(a.usd * 100), b ? Math.round(b.usd * 100) : null, { small: 500, fmt: (c) => `$${(c / 100).toFixed(2)}` })), s.map((x) => x.usd)),
    tile("Tokens", countOf(a.counted), words(`${countOf(a.cache_read)} more read from the cache`, delta(period, a.counted, b?.counted, { small: 10_000, fmt: countOf })), s.map((x) => x.counted)),
    tile("A task", a.usd_per_task === null ? "—" : dollars(a.usd_per_task), a.tasks ? `${a.tasks} tasks in ${a.sessions} sessions` : "no session reported"),
    a.unpriced ? tile("Not priced", countOf(a.unpriced), "tokens of models with no price known") : null,
  ].filter(Boolean);
}

function spentCard(ctx, redraw) {
  const { data, w } = ctx, s = data.series, stack = [...data.stacked, "other"];
  const c = card("spent", "Spent", { wide: true, legend: true, modes: [["usd", "Dollars"], ["tokens", "Tokens"]], onMode: (m) => { mode = m; redraw(); } });
  const of = (d) => (mode === "usd" ? d.by_usd : d.by_tokens), fmt = (v) => (mode === "usd" ? `$${v >= 10 ? Math.round(v) : v.toFixed(1)}` : countOf(v));
  const say = (i) => {
    c.say.textContent = i < 0 ? words(dollars(data.tiles.usd), tokens(data.tiles.counted)) : words(w.long(i), dollars(s[i].usd), tokens(s[i].counted), s[i].unpriced ? `${countOf(s[i].unpriced)} not priced` : "");
  };
  legend(c.legend, data.agents.filter((a) => data.stacked.includes(a.id)).map((a) => ({ color: agentColor(a), name: a.name || "Agent gone", num: mode === "usd" ? priced(a) : countOf(a.counted) })));
  return { box: c.box, draw: () => {
    columns(c.plot, bands(s, w, (d) => stack.map((id) => ({ value: of(d)[id] ?? 0, color: agentInk(data, id) }))), { height: 220, fmt, xlabels: w.short, select: say });
    say(-1);
  } };
}

/** Rows ranked by what the charts measure: dollars, or tokens for a model with no price. */
function rankedCard(id, title, rows, name, color, open = () => undefined) {
  const c = card(id, title, { hint: mode === "usd" ? "dollars, tokens" : "tokens, dollars", list: true });
  const n = (r) => (mode === "usd" ? r.usd : r.counted);
  ranked(c.plot, [...rows].sort((x, y) => n(y) - n(x)).map((r) => ({ name: name(r), n: n(r), color: color(r), num: mode === "usd" ? priced(r) : countOf(r.counted),
    title: words(priced(r), tokens(r.counted), `${countOf(r.cache_read)} read from the cache`), open: open(r) })), { empty: "No session reported in this period." });
  return { box: c.box, draw: () => {} };
}

/** Each agent's last 7 and 30 days against its limits, as the board's pool says them: not the period's. */
function limitsCard({ data }) {
  const c = card("limits", "Agents' limits", { hint: "the last 7 and 30 days, every project", list: true });
  const rows = data.limits.flatMap((a) => [["week", a.week, a.week_limit], ["month", a.month, a.month_limit]].map(([span, used, limit]) => ({
    name: `${a.name} · ${span}`, color: limit && used > limit * 1e6 ? "var(--danger)" : agentColor(a), n: limit ? Math.min(1, used / (limit * 1e6)) : 0,
    share: limit ? `${Math.round((used / (limit * 1e6)) * 100)}%` : "", num: limit ? `${countOf(used)} of ${limit}M` : countOf(used) })));
  ranked(c.plot, rows, { most: 40, empty: "No agent in the pool." });
  return { box: c.box, draw: () => {} };
}

/** The tab: its tiles and its cards, each with `draw()` for a new width; `redraw()` builds it anew. */
export function costs(ctx, redraw) {
  const { data } = ctx;
  return { tiles: tiles(ctx), cards: [
    spentCard(ctx, redraw),
    rankedCard("projects", "Projects", data.projects, projectLabel, inkOf, (p) => () => { location.href = ctx.projectHref(p.id || NONE); }),
    rankedCard("agents", "Agents", data.agents, (a) => a.name || "Agent gone", agentColor),
    rankedCard("models", "Models", data.models, (m) => m.model || "Model not said", () => "var(--ring)"),
    limitsCard(ctx),
  ], note: "Only the sessions Dispatcher started report tokens. Dollars: an estimate at API list prices, not a subscription's bill." };
}
