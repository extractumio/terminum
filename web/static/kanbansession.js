// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// + Session (terminum-dispatcher.md AC-142): a project's agent with no task, its session shown once it opens; the
// agent last used there first, the only one at once. Several are offered as tiles, the board's agent cards, each
// with its kind, effort, slots and command, in a window as wide as they need and centered.
import { agentMark, agentName, metaLine } from "./nav.js";
import { el } from "./listparts.js";
import { sessionUrl } from "./tasklink.js";
import { $, agentColor, agentsOf, api, say, slotsUsed } from "./kanbanutil.js";

const MOST_ACROSS = 4;                             // tiles on a line: more go to the next, rows kept even

/** An agent's tile: a submit key whose value is its id. */
function tile(a, counted, last) {
  const b = el("button", "item agentcard");
  b.type = "submit";
  b.value = a.id;
  const bar = el("span", "pbar");
  bar.style.setProperty("--dot", agentColor(a));
  const txt = el("span", "txt"), head = el("span", "head"), line = el("span", "agline", a.command);
  head.append(agentMark({ agent: a.kind }), el("span", "t", a.name));
  if (last) head.append(el("small", "aglast", "last used"));
  const used = slotsUsed(counted, a);
  txt.append(head, metaLine([agentName(a.kind) ?? a.kind, ...(a.effort ? [a.effort] : []), `${used} of ${a.at_once} at once`], false), line);
  b.append(bar, txt);
  b.title = [a.name, `${used} of ${a.at_once} at once`, a.line].join(" · ");
  return b;
}

/** The agent pressed, or null for Cancel, × and Esc. */
function pickAgent(title, agents, counted, last) {
  const box = $("agentpick"), list = $("agentpicklist");
  $("agentpicktitle").textContent = title;
  list.replaceChildren(...agents.map((a) => tile(a, counted, a.id === last)));
  box.style.setProperty("--across", Math.ceil(agents.length / Math.ceil(agents.length / MOST_ACROSS)));
  box.returnValue = "";
  box.showModal();
  list.firstChild.focus();
  return new Promise((resolve) => box.addEventListener("close", () => {
    resolve(box.returnValue && box.returnValue !== "cancel" ? box.returnValue : null);
  }, { once: true }));
}

/** `slots()`: who is at a limit; `addAgent(project)`: the agents' window, for a new one of that project; `askFolder(id, words)`: a project with no folder. */
export function sessionStarter({ getMeta, slots, addAgent, askFolder }) {
  return async (p) => {
    if (!p.dir) return askFolder(p.id, `${p.name} has no folder: set it in its window.`);
    const meta = getMeta(), now = slots(), last = meta.last_agents?.[p.id], own = agentsOf(meta, p.id), all = own.filter((a) => !now.full.includes(a.id));
    if (!own.length) return addAgent(p.id);
    if (now.full_projects.includes(p.id) || !all.length) return say(now.full_projects.includes(p.id) ? `${p.name} is at its limit of agents at once.` : "Every agent is at its limit.");
    const ordered = [...all.filter((a) => a.id === last), ...all.filter((a) => a.id !== last)];
    const agent = ordered.length === 1 ? ordered[0].id : await pickAgent(`A session in ${p.name}`, ordered, now, last);
    if (!agent) return;
    try {
      const { key } = await api(`/api/task-projects/${p.id}/session`, { method: "POST", body: { agent } });
      (meta.last_agents ??= {})[p.id] = agent;
      location.href = sessionUrl(key);
    } catch (e) { say(e.message); }
  };
}
