// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Dashboard's Tasks tab (terminum.md AC-176): who does the work on tasks: the operations the user decided
// (a task made, edited, moved, commented on, put under another, run, archived) beside the agents' and
// Dispatcher's, over time, by operation and task by task. terminumd's numbers; nothing is added up here.
import { card, delta, kindWords, percent, rankHue, ranked, shareOf, shareTile, stackCard, tile, was, words } from "./dashkit.js";
import { plural } from "./listparts.js";
import { count } from "./msgpage.js";
import { taskUrl } from "./tasklink.js";

/** Who decided an operation, in the stack's order: the numbers' key, its word, its color. */
const WHO = [["you", "you", rankHue(2)], ["agents", "agents", rankHue(1)], ["dispatcher", "Dispatcher", "var(--faint)"]];
/** An operation's words. */
const KINDS = { made: "Made", edit: "Edited", move: "Moved", comment: "Commented", parent: "Put under a task", start: "Started", archive: "Archived" };
/** The tasks by the user's share of their operations. */
const SPREAD = [["all", "All yours", rankHue(2)], ["more", "Half or more yours", rankHue(4)], ["less", "Under half yours", rankHue(1)], ["none", "None yours", "var(--faint)"]];

const parts = (by) => WHO.map(([k, , color]) => ({ n: by[k], color }));
const said = (by) => words(...kindWords(WHO, by));

function tiles({ data, period }) {
  const a = data.tiles, b = data.before, s = data.series;
  const d = (now, before) => delta(period, now, before);
  return [
    shareTile("By hand", a.share, b?.share, "no operation on a task", `${count(a.by.you)} of ${plural(a.ops, "operation")} were yours`),
    tile("Your operations", count(a.by.you), d(a.by.you, b?.by.you), s.map((x) => x.by.you)),
    tile("Automatic", count(a.by.agents + a.by.dispatcher), words(`agents ${count(a.by.agents)}`, `Dispatcher ${count(a.by.dispatcher)}`, b ? d(a.by.agents + a.by.dispatcher, b.by.agents + b.by.dispatcher) : ""), s.map((x) => x.by.agents + x.by.dispatcher)),
    tile("Tasks worked on", count(a.tasks), words(`${a.made_by_hand} made by hand`, d(a.tasks, b?.tasks))),
    tile("Without you", count(a.spread.none), words(a.tasks ? `${percent(a.spread.none / a.tasks)} of the tasks` : "", was(b?.spread.none, count))),
    tile("All by hand", count(a.spread.all), words(a.tasks ? `${percent(a.spread.all / a.tasks)} of the tasks` : "", was(b?.spread.all, count))),
  ];
}

function kindsCard({ data }) {
  const c = card("kinds", "By operation", { hint: "your share", list: true });
  ranked(c.plot, data.kinds.map((k) => ({ name: KINDS[k.kind], n: k.by.you + k.by.agents + k.by.dispatcher, parts: parts(k.by), color: rankHue(2), share: shareOf(k.share), title: said(k.by) })).filter((r) => r.n),
    { empty: "No operation on a task in this period." });
  return c;
}

function spreadCard({ data }) {
  const c = card("spread", "Tasks by your share", { hint: "of their operations", list: true });
  ranked(c.plot, SPREAD.map(([k, name, color]) => ({ name, n: data.tiles.spread[k], color })).filter((r) => r.n), { empty: "No task was worked on in this period." });
  return c;
}

function tasksCard({ data }) {
  const c = card("tasks", "Tasks", { hint: data.more ? `the ${data.tasks.length} with most operations of ${data.tasks.length + data.more}` : "your share, operations", list: true, wide: true });
  c.plot.classList.add("long");
  ranked(c.plot, data.tasks.map((t) => ({ name: `${t.id} ${t.title}`, n: t.n, parts: parts(t.by), color: rankHue(2), share: shareOf(t.share), title: words(t.title, said(t.by)), open: () => { location.href = taskUrl(t.id); } })),
    { most: data.tasks.length, empty: "No task was worked on in this period." });
  return c;
}

/** The tab: its tiles and its cards, each with `draw()` for a new width. */
export function tasks(ctx) {
  const t = ctx.data.tiles;
  return { tiles: tiles(ctx), cards: [
    stackCard("ops", "Operations on tasks", ctx, { kinds: WHO, of: (d) => d.by, totals: t.by, sum: plural(t.ops, "operation"), hint: "by who decided", wide: true, height: 200 }),
    kindsCard(ctx), spreadCard(ctx), tasksCard(ctx),
  ], note: "An operation is counted once, for who decided it: what Dispatcher does to carry a Run or an end out is none. This Mac's time.", kept: "starts counted from" };
}
