// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Auto-Dispatch in the board's bar (terminum-dispatcher.md AC-170), always shown: a switch saying whether it is on
// for the projects shown, and a cog that opens its settings. One project shown: a press turns it on or off. Several:
// a press turns off those that are on; turning on is each project's own, in the window, so no press starts tasks of
// projects nobody looked at. The window: a project's switch, the agent that picks, the agent that runs a task naming
// none, the severities it takes and the wait before a task is taken, with what it does in fixed words. Having turned
// itself off it says why, in the switch and in the window, and only a page that showed why turns it on again.
// terminumd keeps the setting and says what is wrong; every page hears.
import { oneAtATime } from "./dialogs.js";
import { $, agentLabel, api, fillSelect, say, showError } from "./kanbanutil.js";

/** `getMeta()`: terminumd's projects and agents; `chosen()`: the projects the board's filter names, none: all;
 *  `reload()`: read them anew. */
export function autoDispatcher({ getMeta, chosen, reload }) {
  const box = $("autobox"), key = $("autoswitch"), ticks = [...$("autoseverities").querySelectorAll("input")];
  let shown = null;

  const projects = () => { const ids = chosen(), all = getMeta().projects; return ids.length ? all.filter((p) => ids.includes(p.id)) : all; };
  const put = (p, body) => api(`/api/task-projects/${p.id}/auto`, { method: "PUT", body: { ...body, off_why: p.auto?.off_why ?? null } });

  /** What a press of the switch does, and its words for the switch's title. One project shown: on or off, through
   *  its window when no agent picks yet or it turned itself off, which the window says. Several: off for those on. */
  function next() {
    const all = projects(), on = all.filter((p) => p.auto?.on), one = all.length === 1 ? all[0] : null;
    const full = on.filter((p) => p.auto.started_day >= p.auto.day_most).map((p) => `${p.name}'s ${p.auto.day_most} tasks of 24 hours are started: it waits. `).join("");
    const how = (p) => (p.auto.mode === "order" ? `${p.name}'s TODO tasks start by priority and saved order on the free slots` : `${agentLabel(getMeta().agents.find((a) => a.id === p.auto.agent))} picks ${p.name}'s TODO tasks for the free slots and starts them`);
    if (on.length) return { all, on, act: "off", says: full + (one ? `${how(one)}. Press to turn it off` : `On for ${on.map((p) => p.name).join(", ")}. Press to turn it off for them`) };
    if ((one?.auto?.agent || one?.auto?.mode === "order") && !one.auto.off_why) return { all, on, act: "on", says: `No task of ${one.name} starts by itself. Press to turn Auto-Dispatch on` };
    return { all, on, act: "open", says: one ? `No task of ${one.name} starts by itself. Press for its settings, to turn Auto-Dispatch on` : "No task starts by itself. Press to choose a project and turn it on" };
  }

  /** The bar's switch: on while it is on for a project shown, with how many when there are several. */
  function draw() {
    const { all, on, says } = next(), stopped = all.filter((p) => !p.auto?.on && p.auto?.off_why);
    key.setAttribute("aria-checked", String(on.length > 0));
    key.classList.toggle("stopped", stopped.length > 0);
    $("autostate").textContent = all.length === 1 ? (on.length ? "ON" : "OFF") : `${on.length} of ${all.length}`;
    key.title = [...stopped.map((p) => `It turned itself off for ${p.name}: ${p.auto.off_why}`), says].join(" ");
  }

  /** The way of picking the window shows: by priority and saved order no agent picks, and its words say so. */
  const modes = [...$("automode").querySelectorAll("button")];
  const setMode = (mode) => { box.dataset.mode = mode; for (const m of modes) m.setAttribute("aria-pressed", String(m.value === mode)); };

  /** The window's fields as project `p` has them. */
  function fill(p) {
    shown = p;
    const agents = getMeta().agents.map((a) => [a.id, a.name]), auto = p.auto ?? {};
    $("autoproject").value = p.id;
    $("autoon").checked = !!auto.on;
    fillSelect($("autoagent"), [["", "Choose one"], ...agents], auto.agent ?? "");
    fillSelect($("autodefault"), [["", "None"], ...agents], auto.default ?? "");
    for (const t of ticks) t.checked = (auto.severities ?? []).includes(t.value);
    setMode(auto.mode ?? "agent");
    $("autowait").value = auto.wait_minutes ?? 0;
    $("automost").value = auto.day_most ?? "";
    $("autoday").textContent = `Started in the last 24 hours: ${auto.started_day ?? 0}.`;
    showError($("autowhy"), auto.off_why ? `It turned itself off: ${auto.off_why} Save with On ticked turns it on again.` : "");
    showError($("autoerror"), "");
  }

  /** The window, on the first project shown, with what terminumd says now; `on`: its switch ticked for the human to save. */
  async function open(on = false) {
    await reload();
    const all = projects();
    if (!all.length) return say("Make a project first.");
    fillSelect($("autoproject"), all.map((x) => [x.id, x.name]), all[0].id);
    $("autoprojectrow").hidden = all.length < 2;
    fill(all[0]);
    if (on) $("autoon").checked = true;
    box.showModal();
  }

  const keep = oneAtATime(box, async () => {
    try {
      // an empty number is terminumd's to refuse, never a 0 nobody typed
      const number = (id) => ($(id).value === "" ? null : Number($(id).value));
      await put(shown, { on: $("autoon").checked, mode: box.dataset.mode, agent: $("autoagent").value || null, default: $("autodefault").value || null,
        severities: ticks.filter((t) => t.checked).map((t) => t.value), wait_minutes: number("autowait"), day_most: number("automost") });
      box.close();
      await reload();
    } catch (e) { showError($("autoerror"), e.message); }
  });

  async function press() {
    const { all, on, act } = next();
    if (act === "open") return open(true);
    try {
      for (const p of act === "off" ? on : all) await put(p, { on: act === "on" });
    } catch (e) { say(e.message); }
    await reload();
  }

  $("autook").addEventListener("click", keep);
  $("autoproject").addEventListener("change", () => fill(getMeta().projects.find((p) => p.id === $("autoproject").value)));
  $("autocog").addEventListener("click", () => open());
  for (const m of modes) m.addEventListener("click", () => setMode(m.value));
  key.addEventListener("click", oneAtATime(key, press));
  return { draw };
}
