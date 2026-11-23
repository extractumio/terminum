// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { oneAtATime } from "./dialogs.js";
import { actions, part, text } from "./supervisorfields.js";
import { INCIDENTS, incidentCard, incidentCards, onIncidents } from "./incidents.js";
import { $, api, button, showError, stamp, timeOf } from "./kanbanutil.js";
import { el } from "./listparts.js";

function withTimeline(initial, selected) {
  const { card, update } = incidentCard(initial, "screen");
  card.id = `incident-${initial.id}`;
  const details = el("details");
  const list = el("ol", "incident-events");
  const error = el("p", "incident-error");
  error.setAttribute("role", "alert");
  error.hidden = true;
  const more = button("textkey", "Earlier events", () => load(next));
  more.hidden = true;
  details.append(el("summary", "sv-touch", "Timeline"), list, error, more);
  const ruling = el("details");
  const reason = text("", "area");
  reason.required = true;
  reason.maxLength = 1000;
  const confirmation = el("p", "incident-state");
  confirmation.setAttribute("role", "status");
  const form = part("form", "sv-stack", part("label", "sv-stack", "Reason", reason), confirmation);
  const save = el("button", "textkey", "Save ruling");
  save.type = "submit";
  form.append(actions(save));
  form.addEventListener("submit", oneAtATime(form, async (ev) => {
    ev.preventDefault();
    confirmation.textContent = "";
    try {
      await api(`${INCIDENTS}/${initial.id}/ruling`, { method: "POST", body: { reason: reason.value } });
      reason.value = "";
      confirmation.textContent = "Ruling saved on the incident thread.";
    } catch (e) { confirmation.textContent = e.message; }
  }));
  ruling.append(el("summary", "sv-touch", "Record owner ruling"), form);
  card.append(details, ruling);
  let revision = initial.revision, fetched, next, reading = false;
  function keep(i) {
    if (i.revision >= (saved.get(i.id)?.revision ?? 0)) saved.set(i.id, i);
  }

  async function load(before) {
    if (reading) return;
    reading = more.disabled = true;
    const requested = revision;
    showError(error, "");
    try {
      const page = await api(`${INCIDENTS}/${initial.id}${before ? `?before=${before}` : ""}`);
      const events = page.rows.map((event) => {
        const item = el("li");
        item.append(timeOf(event.at, stamp(event.at)), ` · ${event.actor === "human" ? "Owner" : event.actor.startsWith("supervisor:") ? "Supervisor" : "Terminum"} · ${event.words}`);
        return item;
      });
      if (before) list.append(...events);
      else list.replaceChildren(...events);
      next = page.next;
      more.hidden = !next;
      fetched = requested;
    } catch (e) { showError(error, e.message); }
    finally {
      reading = more.disabled = false;
      if (details.open && requested !== revision) load();
    }
  }
  details.addEventListener("toggle", () => { if (details.open && revision !== fetched) load(); });
  if (selected === String(initial.id)) details.open = true;
  return { card, update(i) {
    update(i);
    revision = i.revision;
    if (details.open && revision !== fetched) load();
  } };
}

export function incidentHistory(project, selected, refresh) {
  const error = $("sv-incidents-error"), more = $("sv-incidents-more");
  const draw = incidentCards($("sv-incidents"), "screen", (i) => withTimeline(i, selected));
  const path = `/api/task-projects/${encodeURIComponent(project)}/supervisor/incidents`;
  const saved = new Map();
  let active = new Set(), next, reading = false, again = false;

  function render() {
    const shown = [...saved.values()].sort((a, b) => Number(!!a.closed_at) - Number(!!b.closed_at) || b.id - a.id);
    draw(shown);
    $("sv-incidents-empty").hidden = shown.length > 0;
  }

  function keep(i) {
    if (i.revision >= (saved.get(i.id)?.revision ?? 0)) saved.set(i.id, i);
  }

  async function load(before) {
    if (reading) { again ||= !before; return; }
    reading = more.disabled = true;
    showError(error, "");
    try {
      const page = await api(`${path}${before ? `?before=${before}` : selected ? `?selected=${encodeURIComponent(selected)}` : ""}`);
      page.rows.forEach(keep);
      if (before || next === undefined) next = page.next;
      more.hidden = !next;
      render();
    } catch (e) { showError(error, e.message); }
    finally {
      reading = more.disabled = false;
      if (again) { again = false; load(); }
    }
  }

  more.addEventListener("click", () => load(next));
  refresh.addEventListener("click", () => load());
  onIncidents((items) => {
    const shown = items.filter((i) => i.project === project);
    const ids = new Set(shown.map((i) => i.id));
    if ([...active].some((id) => !ids.has(id))) load();
    active = ids;
    shown.forEach(keep);
    render();
  });
  load();
}
