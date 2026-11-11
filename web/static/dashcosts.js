// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Dashboard's Costs tab (terminum.md AC-172): what the sessions Dispatcher started spent: dollars at list
// prices and tokens by agent, project and model, and each agent's use against its limits. terminumd's numbers.
import { columns } from "./charts.js";
import { bands, card, delta, legend, percent, ranked, tile, words } from "./dashkit.js";
import { agentColor, agentLabel, countOf, dollars } from "./kanbanutil.js";
import { plural } from "./listparts.js";
import { NONE, inkOf, projectLabel } from "./msgpage.js";

/** What the charts measure, dollars or tokens: kept while the page is open. */
let mode = "usd";

const tokens = (n) => `${countOf(n)} tokens`;
/** A row's dollars, or that none are known: a model with no price is never $0. */
const priced = (r) => (r.unpriced && !r.usd ? "no price" : dollars(r.usd));

function tiles({ data, period }) {
  const a = data.tiles, b = data.before, s = data.series;
  return [
    tile("Spent", dollars(a.usd), words("at API list prices", delta(period, Math.round(a.usd * 100), b ? Math.round(b.usd * 100) : null, { small: 500, fmt: (c) => `$${(c / 100).toFixed(2)}` })), s.map((x) => x.usd)),
    tile("Tokens", countOf(a.counted), words(`${countOf(a.cache_read)} more read from the cache`, delta(period, a.counted, b?.counted, { small: 10_000, fmt: countOf })), s.map((x) => x.counted)),
    tile("A task", a.usd_per_task === null ? "—" : dollars(a.usd_per_task), a.tasks ? `${plural(a.tasks, "task")} in ${plural(a.sessions, "session")}` : "no session reported"),
    a.unpriced ? tile("Not priced", countOf(a.unpriced), "tokens of models with no price known") : null,
  ].filter(Boolean);
}

function spentCard({ data, w }, redraw) {
  const s = data.series, stack = [...data.stacked, "other"], usd = mode === "usd";
  const c = card("spent", "Spent", { wide: true, legend: true, modes: [["usd", "Dollars"], ["tokens", "Tokens"]], mode, onMode: (m) => { mode = m; redraw(); } });
  const ink = (id) => (id === "other" ? "var(--faint)" : agentColor(data.agents.find((a) => a.id === id) ?? {}));
  const say = (i) => {
    c.say.textContent = i < 0 ? words(dollars(data.tiles.usd), tokens(data.tiles.counted)) : words(w.long(i), dollars(s[i].usd), tokens(s[i].counted), s[i].unpriced ? `${countOf(s[i].unpriced)} not priced` : "");
  };
  legend(c.legend, data.agents.filter((a) => data.stacked.includes(a.id)).map((a) => ({ color: agentColor(a), name: agentLabel(a), num: usd ? priced(a) : countOf(a.counted) })));
  c.draw = () => {
    columns(c.plot, bands(s, w, (d) => stack.map((id) => ({ value: (usd ? d.by_usd : d.by_tokens)[id] ?? 0, color: ink(id) }))),
      { height: 220, fmt: usd ? (v) => `$${v}` : countOf, xlabels: w.short, select: say });
    say(-1);
  };
  return c;
}

/** Rows ranked by what the charts measure: dollars, or tokens. */
function rankedCard(id, title, rows, name, color, open = () => undefined) {
  const usd = mode === "usd", n = (r) => (usd ? r.usd : r.counted);
  const c = card(id, title, { hint: usd ? "dollars" : "tokens", list: true });
  ranked(c.plot, [...rows].sort((x, y) => n(y) - n(x)).map((r) => ({ name: name(r), n: n(r), color: color(r), num: usd ? priced(r) : countOf(r.counted),
    title: words(priced(r), tokens(r.counted), `${countOf(r.cache_read)} read from the cache`), open: open(r) })), { empty: "No session reported in this period." });
  return c;
}

/** Each agent's last 7 and 30 days against its limits, as the board's pool says them: not the period's. */
function limitsCard({ data }) {
  const c = card("limits", "Agents' limits", { hint: "the last 7 and 30 days, every project", list: true });
  const rows = data.limits.flatMap((a) => [["week", a.week, a.week_limit, a.week_share], ["month", a.month, a.month_limit, a.month_share]].map(([span, used, limit, share]) => ({
    name: `${a.name} · ${span}`, color: share > 1 ? "var(--danger)" : agentColor(a), n: Math.min(1, share ?? 0),
    share: share === null ? "" : percent(share), num: limit ? `${countOf(used)} of ${limit}M` : countOf(used) })));
  ranked(c.plot, rows, { most: 40, empty: "No agent in the pool." });
  return c;
}

/** The tab: its tiles and its cards, each with `draw()` for a new width; `redraw()` builds it anew. */
export function costs(ctx, redraw) {
  const { data } = ctx;
  return { tiles: tiles(ctx), cards: [
    spentCard(ctx, redraw),
    rankedCard("projects", "Projects", data.projects, projectLabel, inkOf, (p) => () => { location.href = ctx.projectHref(p.id || NONE); }),
    rankedCard("agents", "Agents", data.agents, agentLabel, agentColor),
    rankedCard("models", "Models", data.models, (m) => m.model || "Model not said", () => "var(--ring)"),
    limitsCard(ctx),
  ], note: "Only the sessions Dispatcher started report tokens. Dollars: an estimate at API list prices, not a subscription's bill.", kept: "the first report is of" };
}
