// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Auto-Dispatch in the board's bar (terminum-dispatcher.md AC-170), always shown: a switch saying whether it is on
// for the projects shown, and a cog that opens its settings. One project shown: a press turns it on or off. Several:
// a press turns off those that are on; turning on is each project's own, in the window, so no press starts tasks of
// projects nobody looked at. The window: a project's switch, the agent that picks, the agent that runs a task naming
// none, the severities it takes and the wait before a task is taken, with what it does in fixed words. Having turned
// itself off it says why, in the switch and in the window, and only a page that showed why turns it on again.
// terminumd keeps the setting and says what is wrong; every page hears.
import { oneAtATime } from "./dialogs.js";
import { $, SEVERITIES, api, fillSelect, say, showError } from "./kanbanutil.js";
import { el } from "./listparts.js";

/** `getMeta()`: terminumd's projects and agents; `chosen()`: the projects the board's filter names, none: all;
 *  `reload()`: read them anew. */
export function autoDispatcher({ getMeta, chosen, reload }) {
  const box = $("autobox"), key = $("autoswitch");
  let shown = null;

  const projects = () => { const ids = chosen(), all = getMeta().projects; return ids.length ? all.filter((p) => ids.includes(p.id)) : all; };
  const put = (p, body) => api(`/api/task-projects/${p.id}/auto`, { method: "PUT", body: { ...body, off_why: p.auto?.off_why ?? null } });
  const ticks = SEVERITIES.map((s) => {
    const tick = el("input"), label = el("label", null, s);
    tick.type = "checkbox";
    tick.value = s;
    label.prepend(tick);
    $("autoseverities").append(label);
    return tick;
  });

  /** The bar's switch: on while it is on for a project shown, with how many when there are several. */
  function draw() {
    const all = projects(), on = all.filter((p) => p.auto?.on), stopped = all.filter((p) => !p.auto?.on && p.auto?.off_why);
    key.setAttribute("aria-checked", String(on.length > 0));
    key.classList.toggle("stopped", stopped.length > 0);
    $("autostate").textContent = all.length === 1 ? (on.length ? "ON" : "OFF") : `${on.length} of ${all.length}`;
    const why = stopped.map((p) => `It turned itself off for ${p.name}: ${p.auto.off_why}`).join(" ");
    const one = all.length === 1 ? all[0] : null, name = (id) => getMeta().agents.find((a) => a.id === id)?.name ?? "its agent";
    key.title = [why, one && on.length ? `${name(one.auto.agent)} picks ${one.name}'s TODO tasks for the free slots and starts them. Press to turn it off`
      : one ? `No task of ${one.name} starts by itself. Press to turn Auto-Dispatch on`
        : on.length ? `On for ${on.map((p) => p.name).join(", ")}. Press to turn it off for them` : "No task starts by itself. Press to choose a project and turn it on"].filter(Boolean).join(" ");
  }

  /** The window's fields as project `p` has them. */
  function fill(p) {
    shown = p;
    const agents = getMeta().agents.map((a) => [a.id, a.name]), auto = p.auto ?? {};
    $("autoproject").value = p.id;
    $("autoon").checked = !!auto.on;
    fillSelect($("autoagent"), [["", "Choose one"], ...agents], auto.agent ?? "");
    fillSelect($("autodefault"), [["", "None"], ...agents], auto.default ?? "");
    for (const t of ticks) t.checked = (auto.severities ?? []).includes(t.value);
    $("autowait").value = auto.wait_minutes ?? 0;
    showError($("autowhy"), auto.off_why ? `It turned itself off: ${auto.off_why} Save with On ticked turns it on again.` : "");
    showError($("autoerror"), "");
  }

  /** The window, on project `p`, else the first shown; `on`: its switch ticked for the human to save. */
  function open(p = projects()[0], on = false) {
    if (!p) return say("Make a project first.");
    const all = projects();
    fillSelect($("autoproject"), all.map((x) => [x.id, x.name]), p.id);
    $("autoprojectrow").hidden = all.length < 2;
    fill(p);
    if (on) $("autoon").checked = true;
    box.showModal();
  }

  const keep = oneAtATime(box, async () => {
    try {
      await put(shown, { on: $("autoon").checked, agent: $("autoagent").value || null, default: $("autodefault").value || null,
        severities: ticks.filter((t) => t.checked).map((t) => t.value), wait_minutes: Number($("autowait").value) });
      box.close();
      await reload();
    } catch (e) { showError($("autoerror"), e.message); }
  });

  /** The switch's press: one project: on or off, its settings first when no agent picks yet; several: off for those on. */
  async function press() {
    const all = projects(), on = all.filter((p) => p.auto?.on);
    if (!on.length && (all.length !== 1 || !all[0].auto?.agent)) return open(all[0], true);
    try {
      for (const p of on.length ? on : all) await put(p, { on: !on.length });
    } catch (e) { say(e.message); }
    await reload();
  }

  $("autook").addEventListener("click", keep);
  $("autoproject").addEventListener("change", () => fill(getMeta().projects.find((p) => p.id === $("autoproject").value)));
  $("autocog").addEventListener("click", () => open());
  key.addEventListener("click", oneAtATime(key, press));
  return { draw };
}
