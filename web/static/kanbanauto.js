// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Auto-Dispatcher's setting (terminum-dispatcher.md AC-170): a key in the board's bar for the one project shown, "Auto-
// Dispatcher: OFF" or its agent, and the window it opens: the agent that picks, or OFF, and the agent that runs a
// task naming none. Having turned itself off it says why, in the key and in the window, and only a Save from a window
// that showed why turns it on again. terminumd keeps the setting
// and says what is wrong; every page hears.
import { oneAtATime } from "./dialogs.js";
import { $, api, fillSelect, showError } from "./kanbanutil.js";

/** `getMeta()`: terminumd's projects and agents; `chosen()`: the projects shown; `reload()`: read them anew. */
export function autoDispatcher({ getMeta, chosen, reload }) {
  const box = $("autobox"), key = $("autokey");
  let shown = null;

  const nameOf = (id) => getMeta().agents.find((a) => a.id === id)?.name;
  const one = () => (chosen().length === 1 ? getMeta().projects.find((p) => p.id === chosen()[0]) : null);

  /** The bar's key: only with one project shown, whose setting it is. */
  function draw() {
    const p = one(), auto = p?.auto;
    key.hidden = !p;
    if (!p) return;
    const on = auto?.agent ? nameOf(auto.agent) : null;
    key.textContent = `Auto-Dispatcher: ${on ?? "OFF"}`;
    key.classList.toggle("on", !!on);
    key.classList.toggle("stopped", !on && !!auto?.off_why);
    key.title = on ? `${on} picks ${p.name}'s TODO tasks for the free slots and starts them. Press to change`
      : auto?.off_why ? `Auto-Dispatcher turned itself off for ${p.name}: ${auto.off_why} Press to turn it on again`
        : `No task of ${p.name} starts by itself. Press to choose the agent that picks them`;
  }

  /** The window for project `p`. */
  function open(p) {
    shown = p;
    const agents = getMeta().agents.map((a) => [a.id, a.name]);
    fillSelect($("autoagent"), [["", "OFF"], ...agents], p.auto?.agent ?? "");
    fillSelect($("autodefault"), [["", "None: only tasks that name an agent start"], ...agents], p.auto?.default ?? "");
    $("autofor").textContent = `For ${p.name}.`;
    showError($("autowhy"), p.auto?.off_why ? `It turned itself off: ${p.auto.off_why} Save turns it on again.` : "");
    showError($("autoerror"), "");
    box.showModal();
  }

  const keep = oneAtATime(box, async () => {
    try {
      await api(`/api/task-projects/${shown.id}/auto`, { method: "PUT", body: { agent: $("autoagent").value || null, default: $("autodefault").value || null, off_why: shown.auto?.off_why ?? null } });
      box.close();
      await reload();
    } catch (e) { showError($("autoerror"), e.message); }
  });
  $("autook").addEventListener("click", keep);
  key.addEventListener("click", () => { const p = one(); if (p) open(p); });
  return { draw, open };
}
