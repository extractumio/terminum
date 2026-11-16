// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Message history's quick filters (terminum.md AC-151): the rail's groups (Type, From, When, Project), a
// row a choice with how many messages it would show (terminumd's counts: a group's own choice never narrows
// its own rows), and a chip for each filter in force. A chosen row pressed again lets its filter go.
import { el } from "./listparts.js";
import { icon } from "./icons.js";
import { agentMark, agentName } from "./nav.js";
import { button } from "./kanbanutil.js";
import { SENDERS } from "./historyrows.js";
import { $, NONE, addDays, count, dayLabel, dotOf, inkOf, projectLabel, shortDay, today } from "./msgpage.js";

/** What a kind of the address is called: any agent's prompts, a shell's commands, one agent's by its name. */
const kindLabel = (kind) => ({ agent: "Prompts", shell: "Commands" }[kind] ?? agentName(kind) ?? kind);
const senderLabel = (sender) => SENDERS[sender]?.[0] ?? sender;
/** A project of the address, as the session list has it; one it no longer has, or none. */
const projectOf = (projects, id) => projects.get(id) ?? { id: id === NONE ? "" : id };

/** A group's rows into `box`: {label, n?, on, pick, mark?, sub?}. The row in focus stays in focus: a press by the
 *  keys draws the group anew. */
function rows(box, list) {
  const focused = box.contains(document.activeElement) ? document.activeElement.dataset.row : null;
  box.replaceChildren(...list.map(({ label, n, on, pick, mark, sub }) => {
    const b = el("button", `frow${sub ? " sub" : ""}${n === 0 ? " none" : ""}`);
    b.type = "button";
    b.dataset.row = label;
    b.setAttribute("aria-pressed", String(on));
    if (mark) b.append(mark);
    b.append(el("span", "pname", label));
    if (n !== undefined) b.append(el("span", "fnum", count(n)));
    b.addEventListener("click", pick);
    return b;
  }));
  if (focused !== null) [...box.children].find((b) => b.dataset.row === focused)?.focus();
}

/** The rail for `state`, with `facets` (the list's counts by sender, agent and project), `days` (its counts a day,
 *  the dates left out) and the projects by id. */
export function drawRail({ state, facets, days, projects, change }) {
  const one = (key, value, label, n, more = {}) => ({ label, n, on: state[key] === value, pick: () => change({ [key]: state[key] === value ? "" : value }), ...more });
  const agents = Object.entries(facets.agent ?? {}).filter(([a]) => a).sort((a, b) => b[1] - a[1]);
  if (state.kind && !["agent", "shell"].includes(state.kind) && !agents.some(([a]) => a === state.kind)) agents.push([state.kind, 0]);
  rows($("fkind"), [one("kind", "agent", "Prompts", agents.reduce((s, [, n]) => s + n, 0)),
    ...agents.map(([a, n]) => one("kind", a, kindLabel(a), n, { sub: true, mark: agentMark({ agent: a }) })),
    one("kind", "shell", "Commands", facets.agent?.[""] ?? 0, { mark: agentMark({}) })]);
  const sent = (s) => facets.sender?.[s] ?? 0;
  rows($("fsender"), Object.keys(SENDERS).filter((s) => s !== "manager" || sent(s) || state.sender === s).map((s) => one("sender", s, senderLabel(s), sent(s))));
  const t = today();
  const since = (label, from) => {
    const on = state.from === from && state.to === t;
    return { label, n: days.reduce((s, d) => s + (d.day >= from && d.day <= t ? d.n : 0), 0), on, pick: () => change(on ? { from: "", to: "" } : { from, to: t }) };
  };
  rows($("fwhen"), [since("Today", t), since("Last 7 days", addDays(t, -6)), since("Last 30 days", addDays(t, -29))]);
  for (const [id, day] of [["hfrom", state.from], ["hto", state.to]]) if (document.activeElement !== $(id)) $(id).value = day;
  const held = Object.entries(facets.project ?? {}).map(([id, n]) => [id || NONE, n]);
  if (state.project && !held.some(([id]) => id === state.project)) held.push([state.project, 0]);
  rows($("fproject"), held.sort((a, b) => b[1] - a[1]).map(([id, n]) => {
    const p = projectOf(projects, id);
    return one("project", id, projectLabel(p), n, { mark: dotOf(inkOf(p)) });
  }));
}

/** A chip for each filter in force (`session`: the title of the pane the list is narrowed to), each pressed to let
 *  it go, and Clear all (`clear`) when there are two. */
export function drawChips({ state, projects, session, change, clear }) {
  const chips = [];
  const { from, to } = state;
  if (from || to) chips.push([from === to ? dayLabel(from) : `${from ? shortDay(from) : "…"} – ${to ? shortDay(to) : "today"}`, { from: "", to: "" }]);
  if (state.kind) chips.push([kindLabel(state.kind), { kind: "" }]);
  if (state.sender) chips.push([`From ${senderLabel(state.sender)}`, { sender: "" }]);
  if (state.project) chips.push([projectLabel(projectOf(projects, state.project)), { project: "" }]);
  if (state.pane) chips.push([session ? `Session: ${session}` : "One session", { pane: "" }]);
  if (state.task) chips.push([`Task ${state.task}`, { task: "" }]);
  const box = $("hchips");
  box.hidden = !chips.length;
  box.replaceChildren(...chips.map(([text, gone]) => {
    const b = button("hchip", "", () => change(gone));
    b.title = "Remove this filter";
    b.append(el("span", "pname", text), icon("x", 12));
    return b;
  }));
  if (chips.length > 1) box.append(button("textkey", "Clear all", clear));
}
