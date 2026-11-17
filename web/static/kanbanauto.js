// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Auto-Dispatch (terminum-dispatcher.md AC-170): a switch in the board's bar, always shown, saying whether it is on
// for the projects shown, and its settings, a tab of the project's window. One project shown: a press turns it on or
// off. Several: a press turns off those that are on; turning on is each project's own, in its window, so no press
// starts tasks of projects nobody looked at. The tab: the project's switch, the agent that picks, the agent that runs
// a task naming none, the severities it takes and the wait before a task is taken, with what it does in fixed words.
// Having turned itself off it says why, in the switch and in the tab, and only a page that showed why turns it on
// again. terminumd keeps the setting and says what is wrong; every page hears.
import { oneAtATime } from "./dialogs.js";
import { $, agentLabel, agentsOf, api, fillSelect, oneProject, say, showError } from "./kanbanutil.js";

/** `getMeta()`: terminumd's projects and agents; `shownProjects()`: the projects the board shows; `reload()`: read them anew;
 *  `openProject(p)`: the project's window on this tab. */
export function autoDispatcher({ getMeta, shownProjects, reload, openProject }) {
  const box = $("autopanel"), key = $("autoswitch"), ticks = [...$("autoseverities").querySelectorAll("input")];
  let shown = null;

  const put = (p, body) => api(`/api/task-projects/${p.id}/auto`, { method: "PUT", body: { ...body, off_why: p.auto?.off_why ?? null } });

  /** What a press of the switch does, and its words for the switch's title. One project shown: on or off, through
   *  its window when no agent picks yet or it turned itself off, which the window says. Several: off for those on. */
  function next() {
    const all = shownProjects(), on = all.filter((p) => p.auto?.on), one = all.length === 1 ? all[0] : null;
    const full = on.filter((p) => p.auto.started_day >= p.auto.day_most).map((p) => `${p.name}'s ${p.auto.day_most} tasks of 24 hours are started: it waits. `).join("");
    const how = (p) => (p.auto.mode === "order" ? `${p.name}'s TODO tasks start by priority and saved order on the free slots` : `${agentLabel(getMeta().agents.find((a) => a.id === p.auto.agent))} picks ${p.name}'s TODO tasks for the free slots and starts them`);
    if (on.length) return { all, on, act: "off", says: full + (one ? `${how(one)}. Press to turn it off` : `On for ${on.map((p) => p.name).join(", ")}. Press to turn it off for them`) };
    if ((one?.auto?.agent || one?.auto?.mode === "order") && !one.auto.off_why) return { all, on, act: "on", says: `No task of ${one.name} starts by itself. Press to turn Auto-Dispatch on` };
    return { all, on, act: "open", says: one ? `No task of ${one.name} starts by itself. Press for its settings, to turn Auto-Dispatch on` : "No task starts by itself. Select one project to turn it on" };
  }

  /** The bar's switch: on while it is on for a project shown, with how many when there are several. */
  function draw() {
    const { all, on, says } = next(), stopped = all.filter((p) => !p.auto?.on && p.auto?.off_why);
    key.setAttribute("aria-checked", String(on.length > 0));
    key.classList.toggle("stopped", stopped.length > 0);
    $("autostate").textContent = all.length === 1 ? (on.length ? "ON" : "OFF") : `${on.length} of ${all.length}`;
    key.title = [...stopped.map((p) => `It turned itself off for ${p.name}: ${p.auto.off_why}`), says].join(" ");
  }

  /** The way of picking the tab shows, one of two radios: by priority and saved order no agent picks, and its words say so. */
  const modes = [...$("automode").querySelectorAll("input")];
  const setMode = (mode) => { box.dataset.mode = mode; for (const m of modes) m.checked = m.value === mode; };

  /** The tab's fields as project `p` has them. */
  function fill(p) {
    shown = p;
    const agents = agentsOf(getMeta(), p.id).map((a) => [a.id, a.name]), auto = p.auto ?? {};
    $("autoon").checked = !!auto.on;
    fillSelect($("autoagent"), [["", "Choose one"], ...agents], auto.agent ?? "");
    fillSelect($("autodefault"), [["", "None: such tasks are not started"], ...agents], auto.default ?? "");
    for (const t of ticks) t.checked = (auto.severities ?? []).includes(t.value);
    setMode(auto.mode ?? "agent");
    $("autowait").value = auto.wait_minutes ?? 0;
    $("automost").value = auto.day_most ?? "";
    $("autoday").textContent = `Started in the last 24 hours: ${auto.started_day ?? 0}.`;
    showError($("autowhy"), auto.off_why ? `It turned itself off: ${auto.off_why} Save with On ticked turns it on again.` : "");
    showError($("autoerror"), "");
  }

  /** Its tab, in the window of the one project shown (read anew), its switch ticked for the human
   *  to save; several shown: which one is the human's to say. */
  async function open() {
    const p = oneProject(shownProjects(), "turn Auto-Dispatch on");
    if (!p) return;
    await openProject(p);
    $("autoon").checked = true;
  }

  /** Save: what the tab holds, for its project; refused, it says why in the tab. True once kept. */
  async function save() {
    try {
      // an empty number is terminumd's to refuse, never a 0 nobody typed
      const number = (id) => ($(id).value === "" ? null : Number($(id).value));
      await put(shown, { on: $("autoon").checked, mode: box.dataset.mode, agent: $("autoagent").value || null, default: $("autodefault").value || null,
        severities: ticks.filter((t) => t.checked).map((t) => t.value), wait_minutes: number("autowait"), day_most: number("automost") });
      await reload();
      return true;
    } catch (e) { showError($("autoerror"), e.message); return false; }
  }

  async function press() {
    const { all, on, act } = next();
    if (act === "open") return open();
    try {
      for (const p of act === "off" ? on : all) await put(p, { on: act === "on" });
    } catch (e) { say(e.message); }
    await reload();
  }

  for (const m of modes) m.addEventListener("change", () => setMode(m.value));
  key.addEventListener("click", oneAtATime(key, press));
  return { draw, fill, save };
}
