// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-222: the project window's Supervisor tab. The switch belongs to the collector; settings use a revision.
import { oneAtATime } from "./dialogs.js";
import { $, agentsOf, api, button, say as toast, showError } from "./kanbanutil.js";
import { detectorStatus } from "./supervisordetectors.js";
import { hint, stack } from "./supervisorfields.js";
import { portableSettings, probeCommands } from "./supervisorpack.js";
import { criticalLine, preferences } from "./supervisorpreferences.js";

export function supervisorWindow({ getMeta }) {
  const panel = $("supervisorpanel"), body = $("supervisorsettings"), save = $("supervisorok");
  const toggle = $("supervisoron"), scan = $("supervisorscan"), refresh = $("supervisorrefresh");
  const screen = $("supervisorscreen");
  const statusError = $("supervisorswitcherror"), saveError = $("supervisorsaveerror");
  let current = null;

  const controls = () => {
    save.hidden = panel.hidden || !current?.view;
    save.disabled = !!current?.writing;
    toggle.disabled = !current?.status || !!current?.statusReading || !!current?.writing;
    refresh.hidden = !current?.status?.enabled && !current?.statusFailed;
    refresh.disabled = !!current?.statusReading || !!current?.writing;
    screen.inert = body.inert = !!current?.writing;
  };

  function updateScan(state) {
    const { status } = state;
    if (state !== current || !status) return;
    toggle.checked = status.enabled;
    const attached = status.attachment;
    scan.textContent = status.collector_dead ? "The collector is overdue."
      : status.enabled ? !attached ? "Waiting for the first scan."
        : attached.limited_history ? "Scan complete. Less than three weeks of history."
        : "Scan complete."
      : "The Supervisor is off.";
    detectorStatus(body, attached);
  }

  async function readStatus(state) {
    if (state.statusReading) return;
    state.statusReading = true;
    controls();
    try {
      state.status = await state.request();
      state.statusFailed = false;
      if (state === current) showError(statusError, "");
      updateScan(state);
    } catch (e) {
      state.statusFailed = true;
      if (state === current) showError(statusError, e.message);
    } finally {
      state.statusReading = false;
      if (state === current) controls();
    }
  }

  const write = oneAtATime($("projectbox"), async (state, error, work) => {
    state.writing = true;
    controls();
    showError(error, "");
    try { return await work(); }
    catch (e) {
      if (state === current) {
        showError(error, e.message);
        error.scrollIntoView({ block: "nearest" });
      }
    } finally { state.writing = false; }
  });

  async function run(state, error, work) {
    if (state !== current) return;
    const focus = await write(state, error, work);
    if (state === current) { controls(); focus?.focus(); }
  }

  function draw(state) {
    if (state !== current) return;
    const opened = new Set([...body.querySelectorAll("details[open]")].map((d) => d.dataset.section));
    const on = (error, work) => run(state, error, work);
    const fields = preferences(state.view, agentsOf(getMeta(), state.id));
    const probes = probeCommands({ request: state.request, run: on });
    const portable = portableSettings({
      request: state.request, run: on, revision: () => state.view.revision,
      imported: (view) => {
        state.view = view;
        draw(state);
        if (state === current) {
          showError(saveError, "");
          toast("Supervisor settings imported");
          if (state.status?.enabled) readStatus(state);
        }
      },
    });
    probes.draw(state.view);
    body.replaceChildren(stack(fields.node, probes.node, criticalLine(), portable));
    for (const group of body.querySelectorAll("details")) group.open = opened.has(group.dataset.section);
    state.read = fields.read;
    updateScan(state);
    controls();
  }

  async function readSettings(state) {
    showError(saveError, "");
    body.replaceChildren(hint("Loading Supervisor settings…"));
    try {
      state.view = await state.request("/settings");
      draw(state);
    } catch (e) {
      if (state !== current) return;
      showError(saveError, e.message);
      body.replaceChildren(button("textkey", "Reload settings", () => readSettings(state)));
      controls();
    }
  }

  function open(id) {
    if (current?.id === id) { controls(); return; }
    const path = `/api/task-projects/${encodeURIComponent(id)}/supervisor`;
    const state = current = { id, request: (part = "", method, body, asText) => api(path + part, { method, body, asText }) };
    screen.href = `/supervisor/?${new URLSearchParams({ project: id })}`;
    toggle.checked = false;
    scan.textContent = "";
    showError(statusError, "");
    controls();
    readStatus(state);
    readSettings(state);
  }

  toggle.addEventListener("change", () => {
    const state = current, enabled = toggle.checked;
    if (!state?.status) return;
    toggle.checked = state.status.enabled;
    run(state, statusError, async () => {
      const saved = await state.request("", "PUT", { enabled });
      state.status = { ...state.status, ...saved, collector_dead: false };
      updateScan(state);
      await readStatus(state);
    });
  });
  refresh.addEventListener("click", () => { if (current) readStatus(current); });
  save.addEventListener("click", () => {
    const state = current;
    if (!state?.view) return;
    // Validate and focus a bad input before the controls become inert for the request.
    let settings;
    showError(saveError, "");
    try { settings = state.read(); }
    catch (e) { if (!e.field) showError(saveError, e.message); return; }
    run(state, saveError, async () => {
      state.view = await state.request("/settings", "PUT", { revision: state.view.revision, settings });
      draw(state);
      if (state === current) {
        toast("Supervisor settings saved");
        if (state.status?.enabled) await readStatus(state);
      }
    });
  });
  return { open, hide: () => { save.hidden = true; }, reset: () => { current = null; body.replaceChildren(); controls(); } };
}
