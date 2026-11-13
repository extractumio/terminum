// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// + Session (terminum-dispatcher.md AC-142): a project's agent with no task, its session shown once it opens; the
// agent last used there first, the only one at once.
import { choose } from "./dialogs.js";
import { sessionUrl } from "./tasklink.js";
import { agentsOf, api, say } from "./kanbanutil.js";

/** `slots()`: who is at a limit; `addAgent(project)`: the agents' window, for a new one of that project; `askFolder(id, words)`: a project with no folder. */
export function sessionStarter({ getMeta, slots, addAgent, askFolder }) {
  return async (p) => {
    if (!p.dir) return askFolder(p.id, `${p.name} has no folder: set it in its window.`);
    const meta = getMeta(), now = slots(), last = meta.last_agents?.[p.id], own = agentsOf(meta, p.id), all = own.filter((a) => !now.full.includes(a.id));
    if (!own.length) return addAgent(p.id);
    if (now.full_projects.includes(p.id) || !all.length) return say(now.full_projects.includes(p.id) ? `${p.name} is at its limit of agents at once.` : "Every agent is at its limit.");
    const ordered = [...all.filter((a) => a.id === last), ...all.filter((a) => a.id !== last)];
    const agent = ordered.length === 1 ? ordered[0].id : await choose(`A session in ${p.name}`, "Which agent? Nothing is typed in it: it is yours.", ordered.map((a) => [a.id, a.name]));
    if (!agent) return;
    try {
      const { key } = await api(`/api/task-projects/${p.id}/session`, { method: "POST", body: { agent } });
      (meta.last_agents ??= {})[p.id] = agent;
      location.href = sessionUrl(key);
    } catch (e) { say(e.message); }
  };
}
