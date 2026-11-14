// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A project's integration settings (terminum-integration.md AC-186): on or off, the target, the commands Check,
// Publish and After, the batch's size, the check's time, and the agent of its own pool that analyses a red batch,
// with its budget, red rounds and prompt. terminumd checks what is kept and says what is wrong; "What runs" is its
// words for the settings as saved.
import { oneAtATime } from "./dialogs.js";
import { el } from "./listparts.js";
import { $, agentsOf, api, fillSelect } from "./kanbanutil.js";

const FIELDS = { target: "integrationtarget", check: "integrationcheck", publish: "integrationpublish", after: "integrationafter" };
const NUMBERS = { batch_most: "integrationbatch", check_minutes: "integrationminutes", agent_budget_minutes: "integrationbudget", red_rounds_most: "integrationrounds" };

export function integrationWindow({ getMeta }) {
  const box = $("integrationbox");
  let project = null, defaultPrompt = "";

  const say = (text) => { $("integrationerror").textContent = text; $("integrationerror").hidden = !text; };
  /** Under an empty line, what it means (AC-186). */
  const hints = () => {
    const target = $("integrationtarget").value.trim() || "origin/main";
    $("integrationpublishhint").textContent = `terminum pushes the candidate to ${target}, never forced.`;
    for (const k of ["check", "publish", "after"]) $(`integration${k}hint`).hidden = !!$(FIELDS[k]).value.trim();
    $("integrationnoagent").hidden = !!$("integrationagent").value;
  };

  async function open(id) {
    project = id;
    say("");
    const meta = getMeta();
    $("integrationhead").textContent = `Integration of ${meta.projects.find((p) => p.id === id)?.name ?? id}`;
    let s;
    try {
      s = await api(`/api/task-projects/${id}/integration`);
    } catch (e) {
      $("integrationfields").hidden = true;
      $("integrationok").hidden = true;
      say(e.message);
      return box.showModal();
    }
    defaultPrompt = s.default_prompt;
    $("integrationelsewhere").hidden = s.here;
    $("integrationfields").hidden = !s.here;   // a project on another server has no switch
    $("integrationok").hidden = !s.here;
    $("integrationon").checked = s.on;
    for (const [k, field] of Object.entries(FIELDS)) $(field).value = s[k];
    for (const [k, field] of Object.entries(NUMBERS)) $(field).value = s[k];
    fillSelect($("integrationagent"), [["", "None"], ...agentsOf(meta, id).map((a) => [a.id, a.name])], s.agent ?? "");
    $("integrationprompt").value = s.prompt || defaultPrompt;
    $("integrationruns").replaceChildren(...s.runs.map((line) => el("li", null, line)));
    hints();
    box.showModal();
  }

  /** Kept, the window closes; refused, it says terminumd's words and keeps what was typed. One at a time. */
  const save = oneAtATime(box, async () => {
    const body = { on: $("integrationon").checked, agent: $("integrationagent").value || null, prompt: $("integrationprompt").value };
    for (const [k, field] of Object.entries(FIELDS)) body[k] = $(field).value;
    for (const [k, field] of Object.entries(NUMBERS)) body[k] = $(field).value === "" ? null : Number($(field).value);
    try {
      await api(`/api/task-projects/${project}/integration`, { method: "PUT", body });
      box.close();
    } catch (e) { say(e.message); }
  });

  $("integrationok").addEventListener("click", save);
  $("integrationfields").addEventListener("input", hints);
  $("integrationagent").addEventListener("change", hints);
  $("integrationpromptreset").addEventListener("click", () => { $("integrationprompt").value = defaultPrompt; });
  box.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.isComposing && e.target.matches("#integrationfields input:not([type=checkbox])")) { e.preventDefault(); save(); }
  });
  return { open };
}
