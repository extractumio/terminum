// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A schedule's window (terminum-schedules.md AC-160): its title, text, project, severity, agent, When, time budget
// and Paused; terminumd says its next runs, the prompt that will go and the agent's command as it is typed, and
// refuses what is wrong in the field's words. What was typed is not lost by a slip of Esc (terminum-kanban.md AC-133).
import { askConfirm } from "./dialogs.js";
import { el } from "./listparts.js";
import { DAY_NAMES, KINDS, MINUTE_STEPS, fromCron, localSeconds, toCron } from "./cadence.js";
import { $, DEFAULT, api, fillSelect, renderInto, showError, stamp } from "./kanbanutil.js";

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
    const steps = k === "hours" ? HOUR_STEPS : MINUTE_STEPS, was = Number(f.every.value);
    fillSelect(f.every, steps.map((n) => [String(n), `${n} ${k === "hours" ? (n === 1 ? "hour" : "hours") : "minutes"}`]), String(steps.includes(was) ? was : steps[steps.length > 3 ? 1 : 0]));
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
      const runs = v.next.map((at) => new Date(at * 1000).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }));
      $("schednext").textContent = `${v.words}. Next: ${runs.join(", ")}`;
      $("schedprompt").textContent = v.prompt;
      $("schedline").textContent = v.line;
      showError(error, "");
    } catch (e) {
      if (n !== asked) return;
      $("schednext").textContent = "";
      showError(error, e.message);
    }
  }
  let soon = 0;
  const changedWhen = () => { clearTimeout(soon); soon = setTimeout(preview, 250); };
  for (const k of ["kind", "every", "minute", "day", "date", "time", "cron", "agent", "budget", "project", "title", "text"]) f[k].addEventListener("input", changedWhen);
  f.kind.addEventListener("change", () => { shownParts(); changedWhen(); });

  /** Write or Preview of its text. */
  function view(shown) {
    $("schedwrite").setAttribute("aria-selected", String(!shown));
    $("schedpreview").setAttribute("aria-selected", String(shown));
    f.text.hidden = shown;
    $("schedshown").hidden = !shown;
    if (shown) renderInto($("schedshown"), f.text.value);
  }
  $("schedwrite").addEventListener("click", () => view(false));
  $("schedpreview").addEventListener("click", () => view(true));

  /** Its window: a new one, or `s` to change. */
  function open(s = null) {
    editing = s;
    const meta = getMeta(), shown = chosen();
    fillSelect(f.project, meta.projects.map((p) => [p.id, p.name]), s?.project ?? (shown.length === 1 ? shown[0] : DEFAULT));
    fillSelect(f.agent, [["", "Choose an agent"], ...meta.agents.map((a) => [a.id, a.name])], s?.agent ?? (meta.agents.length === 1 ? meta.agents[0].id : ""));
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
    f.date.value = p.once ? new Date(p.once * 1000 - new Date(p.once * 1000).getTimezoneOffset() * 60000).toISOString().slice(0, 10) : "";
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

  async function save() {
    try {
      await api(editing ? `/api/schedules/${editing.id}` : "/api/schedules", { method: editing ? "PATCH" : "POST", body: body() });
      dialog.close();
      changed();
    } catch (e) { showError(error, e.message); }
  }
  $("schedok").addEventListener("click", save);
  dialog.addEventListener("keydown", (e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); save(); } });
  $("scheddrop").addEventListener("click", async () => {
    if (!await askConfirm(`Delete ${editing.key}? Its tasks stay.`, "Delete")) return;
    try {
      await api(`/api/schedules/${editing.id}`, { method: "DELETE" });
      dialog.close();
      changed();
    } catch (e) { showError(error, e.message); }
  });
  dialog.addEventListener("cancel", async (e) => {
    if (snap() === snapshot) return;
    e.preventDefault();
    if (await askConfirm("Discard what you typed?", "Discard")) dialog.close();
  });
  return { open };
}
