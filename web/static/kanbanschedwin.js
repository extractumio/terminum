// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A schedule's window (terminum-schedules.md AC-160): its title, text, project, severity, agent, When, time budget
// and Paused; terminumd says its next runs, the prompt that will go, where it runs and the agent's command as it is typed, and
// refuses what is wrong in the field's words. What was typed is not lost by a slip of Esc (terminum-kanban.md AC-133).
import { askConfirm, oneAtATime } from "./dialogs.js";
import { el, plural } from "./listparts.js";
import { DAY_NAMES, KINDS, MINUTE_STEPS, fromCron, localSeconds, toCron } from "./cadence.js";
import { $, agentsOf, api, fillSelect, knownAgent, onlyProject, projectOptions, guardCancel, onSend, showError, stamp, writePreview } from "./kanbanutil.js";

const HOUR_STEPS = [1, 2, 3, 4, 6, 8, 12];

/** `getMeta()`: terminumd's projects and agents; `chosen()`: the board's projects; `changed()`: after a save. */
export function scheduleWindow({ getMeta, chosen, changed }) {
  const dialog = $("schedbox"), error = $("schederror");
  const f = { title: $("schedtitle"), text: $("schedtext"), project: $("schedproj"), severity: $("schedsev"), agent: $("schedagent"), kind: $("schedkind"),
    every: $("schedevery"), minute: $("schedminute"), day: $("schedday"), date: $("scheddate"), time: $("schedtime"), cron: $("schedcron"), budget: $("schedbudget"), paused: $("schedpaused") };
  let editing = null, snapshot = "", days = new Set([1]), asked = 0;
  fillSelect(f.kind, KINDS, "daily");
  const dayKeys = DAY_NAMES.map((name, i) => {
    const b = el("button", null, name);
    b.type = "button";
    b.addEventListener("click", () => { if (!days.delete(i)) days.add(i); if (!days.size) days.add(i); shownDays(); changedWhen(); });
    return b;
  });
  $("whendays").replaceChildren(...[1, 2, 3, 4, 5, 6, 0].map((i) => dayKeys[i]));
  const shownDays = () => dayKeys.forEach((b, i) => b.setAttribute("aria-pressed", String(days.has(i))));

  /** The picker as it stands: its kind's parts shown, its cron fields. */
  function state() {
    return { kind: f.kind.value, every: Number(f.every.value), minute: Number(f.minute.value || 0), time: f.time.value || "09:00", days: [...days], day: Number(f.day.value || 1), cron: f.cron.value };
  }
  function shownParts() {
    const k = f.kind.value, on = { whenevery: ["minutes", "hours"], whenminute: ["hours"], whenday: ["monthly"], whendate: ["once"], whentime: ["daily", "weekdays", "weekly", "monthly", "once"], schedcron: ["cron"], whendays: ["weekly"] };
    for (const [id, kinds] of Object.entries(on)) $(id).hidden = !kinds.includes(k);
    // another kind's number is not carried over: 1 hour would become every minute
    const hours = k === "hours", steps = hours ? HOUR_STEPS : MINUTE_STEPS, was = f.every.dataset.kind === k ? Number(f.every.value) : 0;
    f.every.dataset.kind = k;
    fillSelect(f.every, steps.map((n) => [String(n), plural(n, hours ? "hour" : "minute")]), String(steps.includes(was) ? was : hours ? 2 : 20));
  }

  /** What goes to terminumd. */
  function body() {
    const s = state(), cron = toCron(s);
    return { title: f.title.value, body: f.text.value, project: f.project.value, severity: f.severity.value, agent: f.agent.value,
      cron, once_at: cron ? null : localSeconds(f.date.value, f.time.value), budget_minutes: f.budget.value === "" ? null : Number(f.budget.value), paused: f.paused.checked };
  }
  const snap = () => JSON.stringify(body());

  /** terminumd's word on what is typed: the next runs, the prompt and the command, or what is wrong. */
  async function preview() {
    const n = ++asked, b = body();
    $("schedfields").textContent = b.cron ?? "";
    try {
      const v = await api("/api/schedules/preview", { method: "POST", body: { ...b, title: b.title || "Untitled" } });
      if (n !== asked) return;
      $("schednext").textContent = `${v.words}. Next: ${v.next_words.join(", ")}`;   // the Mac's clock, terminumd's words
      $("schedprompt").textContent = v.prompt;
      $("schedline").textContent = v.line;
      $("schedplace").textContent = v.place;
      showError(error, "");
    } catch (e) {
      if (n !== asked) return;
      $("schednext").textContent = "";
      showError(error, e.message);
    }
  }
  let soon = 0;
  /** Its fields at once; terminumd's words when typing pauses. */
  const changedWhen = () => {
    const cron = toCron(state());
    if ($("schedfields").textContent !== (cron ?? "")) { $("schedfields").textContent = cron ?? ""; $("schednext").textContent = ""; }
    clearTimeout(soon);
    soon = setTimeout(preview, 250);
  };
  for (const k of ["minute", "day", "date", "time", "cron", "budget"]) f[k].addEventListener("input", changedWhen);
  // a long text goes once typed, not at each pause; a choice is made once
  for (const k of ["title", "text", "every", "agent", "project"]) f[k].addEventListener("change", changedWhen);
  f.kind.addEventListener("change", () => { shownParts(); changedWhen(); });

  const view = writePreview("sched", f.text, $("schedshown"));

  /** The agents of the project chosen: a schedule runs on one of its own project's (terminum-dispatcher.md AC-137). */
  function fillAgentsOf(wanted = f.agent.value) {
    const own = agentsOf(getMeta(), f.project.value);
    fillSelect(f.agent, [["", "Choose an agent"], ...own.map((a) => [a.id, a.name])], knownAgent(own, [wanted])?.id ?? "");
  }
  f.project.addEventListener("change", () => fillAgentsOf());

  /** Its window: a new one, or `s` to change. */
  function open(s = null) {
    editing = s;
    const meta = getMeta(), shown = chosen();
    const project = s?.project ?? onlyProject(shown, meta);
    fillSelect(f.project, projectOptions(meta, project), project);
    fillAgentsOf(s?.agent);
    f.title.value = s?.title ?? "";
    f.text.value = s?.body ?? "";
    f.severity.value = s?.severity ?? "P2";
    f.budget.value = s ? String(s.budget_seconds / 60) : "60";
    f.paused.checked = !!s?.paused;
    const p = fromCron(s?.cron ?? (s ? null : "0 9 * * *"), s?.once_at ?? null);
    f.kind.value = p.kind;
    f.minute.value = String(p.minute);
    f.day.value = String(p.day);
    f.time.value = p.once ? new Date(p.once * 1000).toTimeString().slice(0, 5) : p.time;
    f.date.value = p.once ? new Date(p.once * 1000).toLocaleDateString("sv") : "";   // the local day as YYYY-MM-DD
    f.cron.value = p.cron;
    days = new Set(p.days);
    shownDays();
    shownParts();
    f.every.value = String(p.every);
    $("schedhead").textContent = s ? `${s.key} · ${s.title}` : "New schedule";
    $("scheddrop").hidden = !s;
    $("schedby").hidden = !s;
    if (s) $("schedby").textContent = `Set by ${s.author_name || "you"} · changed ${stamp(s.updated_at)}${s.paused_why ? ` · ${s.paused_why}` : ""}`;
    view(false);
    showError(error, "");
    snapshot = snap();
    dialog.showModal();
    f.title.focus();
    preview();
  }

  /** A change to terminumd, one at a time (AC-165): the window closes when it is kept, and says why when it is not. */
  const send = oneAtATime(dialog, async (path, options) => {
    try {
      await api(path, options);
      dialog.close();
      changed();
    } catch (e) { showError(error, e.message); }
  });
  const save = () => send(editing ? `/api/schedules/${editing.id}` : "/api/schedules", { method: editing ? "PATCH" : "POST", body: body() });
  $("schedok").addEventListener("click", save);
  onSend(dialog, save);
  $("scheddrop").addEventListener("click", async () => {
    if (await askConfirm(`Delete ${editing.key}? Its tasks stay.`, "Delete")) send(`/api/schedules/${editing.id}`, { method: "DELETE" });
  });
  guardCancel(dialog, () => snap() === snapshot);
  return { open };
}
