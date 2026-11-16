// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Message history's quick filters (terminum.md AC-151): the rail's groups (Type, From, When, Project), a
// row a choice with how many messages it would show (terminumd's counts: a group's own choice never narrows
// its own rows), and a chip for each filter in force. A chosen row pressed again lets its filter go.
import { el } from "./listparts.js";
import { icon } from "./icons.js";
import { agentMark, agentName } from "./nav.js";
import { $, NONE, addDays, dayLabel, dotOf, inkOf, projectLabel, shortDay, today } from "./msgpage.js";

const SENDERS = [["human", "You"], ["dispatcher", "Dispatcher"], ["manager", "Manager"]];
const count = (n) => n.toLocaleString("en-US");

/** What a kind of the address is called: any agent's prompts, a shell's commands, one agent's by its name. */
export const kindLabel = (kind) => ({ agent: "Prompts", shell: "Commands" }[kind] ?? agentName(kind) ?? kind);
export const senderLabel = (sender) => Object.fromEntries(SENDERS)[sender] ?? sender;
/** The days chosen, in a few words. */
export const whenLabel = ({ from, to }) => (from === to ? dayLabel(from) : `${from ? shortDay(from) : "…"} – ${to ? shortDay(to) : "today"}`);

/** A group's rows into `box`: {label, n?, on, pick, mark?, sub?}. */
function rows(box, list) {
  box.replaceChildren(...list.map(({ label, n, on, pick, mark, sub }) => {
    const b = el("button", `frow${sub ? " sub" : ""}${n === 0 ? " none" : ""}`);
    b.type = "button";
    b.setAttribute("aria-pressed", String(on));
    if (mark) b.append(mark);
    b.append(el("span", "pname", label));
    if (n !== undefined) b.append(el("span", "fnum", count(n)));
    b.addEventListener("click", pick);
    return b;
  }));
}

/** The rail for `state`, with `facets` (the list's counts by sender, agent and project) and the projects by id. */
export function drawRail({ state, facets, projects, change }) {
  const one = (key, value, label, n, more = {}) => ({ label, n, on: state[key] === value, pick: () => change({ [key]: state[key] === value ? "" : value }), ...more });
  const agents = Object.entries(facets.agent ?? {}).filter(([a]) => a).sort((a, b) => b[1] - a[1]);
  if (state.kind && !["agent", "shell"].includes(state.kind) && !agents.some(([a]) => a === state.kind)) agents.push([state.kind, 0]);
  rows($("fkind"), [one("kind", "agent", "Prompts", agents.reduce((s, [, n]) => s + n, 0)),
    ...agents.map(([a, n]) => one("kind", a, kindLabel(a), n, { sub: true, mark: agentMark({ agent: a }) })),
    one("kind", "shell", "Commands", facets.agent?.[""] ?? 0, { mark: agentMark({}) })]);
  const sent = (s) => facets.sender?.[s] ?? 0;
  rows($("fsender"), SENDERS.filter(([s]) => s !== "manager" || sent(s) || state.sender === s).map(([s, label]) => one("sender", s, label, sent(s))));
  const t = today(), days = (label, from, to) => { const on = state.from === from && state.to === to; return { label, on, pick: () => change(on ? { from: "", to: "" } : { from, to }) }; };
  rows($("fwhen"), [days("Today", t, t), days("Last 7 days", addDays(t, -6), t), days("Last 30 days", addDays(t, -29), t)]);
  $("hfrom").value = state.from;
  $("hto").value = state.to;
  const held = Object.entries(facets.project ?? {}).map(([id, n]) => [id || NONE, n]);
  if (state.project && !held.some(([id]) => id === state.project)) held.push([state.project, 0]);
  rows($("fproject"), held.sort((a, b) => b[1] - a[1]).map(([id, n]) => {
    const p = projects.get(id) ?? { id: id === NONE ? "" : id };
    return one("project", id, projectLabel(p), n, { mark: dotOf(inkOf(p)) });
  }));
}

/** A chip for each filter in force (`session`: the title of the pane the list is narrowed to), and Clear all. */
export function drawChips({ state, projects, session, change }) {
  const chips = [];
  const chip = (text, gone) => chips.push([text, gone]);
  if (state.from || state.to) chip(whenLabel(state), { from: "", to: "" });
  if (state.kind) chip(kindLabel(state.kind), { kind: "" });
  if (state.sender) chip(`From ${senderLabel(state.sender)}`, { sender: "" });
  if (state.project) chip(projectLabel(projects.get(state.project) ?? { id: state.project === NONE ? "" : state.project }), { project: "" });
  if (state.pane) chip(session ? `Session: ${session}` : "One session", { pane: "" });
  if (state.task) chip(`Task ${state.task}`, { task: "" });
  const box = $("hchips");
  box.hidden = !chips.length;
  box.replaceChildren(...chips.map(([text, gone]) => {
    const b = el("button", "hchip");
    b.type = "button";
    b.title = "Remove this filter";
    b.append(el("span", "", text), icon("x", 12));
    b.addEventListener("click", () => change(gone));
    return b;
  }));
  if (chips.length > 1) {
    const all = el("button", "textkey", "Clear all");
    all.type = "button";
    all.addEventListener("click", () => change(Object.assign({}, ...chips.map(([, gone]) => gone))));
    box.append(all);
  }
}
