// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The pool over the columns, and its window (terminum-dispatcher.md AC-137, AC-138): what a task is dropped
// on to run: each a rectangle with its color, name, kind, effort, its subscription's stripes (AC-168) and a dot per
// live session; its window shows those, its tokens and its sessions' tasks over its settings. Each is one project's:
// the strip shows the agents of the projects shown.
// terminumd checks what is kept and says what is wrong; every page hears.
import { askConfirm, oneAtATime } from "./dialogs.js";
import { agentMark, agentName, metaLine } from "./nav.js";
import { el, iconButton } from "./listparts.js";
import { agentView, stripes } from "./kanbanagentview.js";
import { asTyped } from "./astyped.js";
import { slotState } from "./cardstate.js";
import { queueKey, queueList } from "./kanbanqueue.js";
import { $, agentColor, agentsOf, api, button, fillSelect, guardCancel, onlyProject, projectOptions, showError } from "./kanbanutil.js";

const KINDS = ["claude", "codex"];                 // their commands are their names: what a kind fills in

/** `getMeta()`: terminumd's projects and agents; `chosen()`: the projects shown, none for all; `slots()`: what each agent's slots hold, as terminumd counts them;
 *  `liveRow(key)`: a session's row; `dragging()`: the tasks of a drag; `given()`: those a press or a drop would run on an
 *  agent; `run(tasks, agent)`; `readings()`: each kind's subscription windows (AC-168); `openTask(id)`. */
export function agentsSection({ getMeta, chosen, reload, refreshQueue, slots: counted, liveRow, dragging, given, run, openSession, readings, openTask }) {
  // on the columns' tracks: an agent a column wide, its keys in the fifth (kanban.css .tracks)
  const box = el("section", "agents tracks"), list = el("div", "agentlist trackitems"), side = el("div", "trackside");
  const addKey = iconButton("icon", "plus", "Add agent", "Add an agent to a project's pool", () => open());
  addKey.id = "addagent";
  const hint = el("p", "pickhint");
  hint.hidden = true;
  side.append(addKey, hint);
  box.append(list, side);
  const field = { project: $("agentproject"), name: $("agentname"), kind: $("agentkind"), effort: $("agenteffort"), command: $("agentcommand"), at_once: $("agentatonce"), week_limit: $("agentweek"), month_limit: $("agentmonth") };
  const dialog = $("agentbox"), error = $("agenterror");
  asTyped(field.command);
  let editing = null, snapshot = "";

  const slots = (a) => counted().agents[a.id] ?? [];
  /** The agents of the projects shown, all of them when none is chosen. */
  const shownAgents = () => { const shown = chosen(); return getMeta().agents.filter((a) => !shown.length || shown.includes(a.project)); };
  /** More than one project's agents stand together: each card says whose it is. */
  const several = () => chosen().length !== 1 && getMeta().projects.length > 1;
  const whose = (a) => (several() ? [projectName(a.project)].filter(Boolean) : []);
  const projectName = (id) => getMeta().projects.find((p) => p.id === id)?.name;
  const queueOf = (a) => counted().queue?.[a.id] ?? [];
  const MOST_FREE = 8;                             // an agent of more slots than this says them by its count alone
  /** At a limit, as terminumd says: its own, or its project's. */
  const full = (a) => counted().full.includes(a.id) || counted().full_projects.includes(a.project);

  const WORDS = { starting: "starting", working: "working", waits: "waits for you", idle: "idle" };
  /** What a session does now. */
  const doing = (s) => slotState(s, s.key ? liveRow(s.key) : null);

  /** A session's dot: blinking while it works, still when it waits or idles; a press on it opens it. */
  function dot(s) {
    const now = doing(s), d = el("span", `agdot ${now === "working" || now === "starting" ? "on" : now}${s.round ? " round" : ""}`);
    d.dataset.key = s.key ?? "";
    d.title = [s.tasks, projectName(s.project), WORDS[now], s.round ? "Auto-Dispatcher's round: it takes no slot" : ""].filter(Boolean).join(" · ");
    return d;
  }

  /** Its rectangle: its color, name, kind, effort, its project when several are shown, and a dot for each session of what is shown; its limit counts them all. */
  function row(a) {
    const shown = chosen(), mine = slots(a).filter((s) => !shown.length || shown.includes(s.project)), of = whose(a);
    const b = button(`item agentcard${full(a) ? " busy" : ""}`);
    b.dataset.agent = a.id;
    const bar = el("span", `pbar${slots(a).length ? " busy" : ""}`);
    bar.style.setProperty("--dot", agentColor(a));
    b.style.setProperty("--ag", agentColor(a));
    const txt = el("span", "txt"), line = el("span", "head"), dots = el("span", "agdots");
    // a dashed ring for each slot its schedules keep, before its sessions' dots (terminum-schedules.md AC-161); a plain
    // ring for each slot still free; then a pale dot for each task of its queue, in the key that opens it (AC-169)
    const kept = counted().kept?.[a.id] ?? [];
    for (const k of kept) { const m = el("span", "agkept"); m.title = `kept for ${k}`; m.setAttribute("aria-label", `kept for ${k}`); dots.append(m); }
    dots.append(...mine.map(dot));
    const used = slots(a).filter((s) => !s.kept && !s.round).length + kept.length;   // a round's session takes no slot (AC-170)
    if (a.at_once <= MOST_FREE) for (let i = used; i < a.at_once; i++) { const m = el("span", "agfree"); m.title = "a free slot"; dots.append(m); }
    if (a.at_once > 1 || kept.length) dots.append(el("small", "agcount", `${used}/${a.at_once}`));
    const queued = queueOf(a);
    if (queued.length) dots.append(queueKey(a, queued, () => open(a, true)));
    line.append(agentMark({ agent: a.kind }), el("span", "t", a.name));
    txt.append(line, metaLine([agentName(a.kind), ...(a.effort ? [a.effort] : []), ...of], false), stripes(a.kind, readings()[a.kind], Date.now() / 1000), dots);
    b.append(bar, txt);
    b.title = [a.name, ...of, `${used} of ${a.at_once} at once`, ...(kept.length ? [`kept for ${kept.join(", ")}`] : []), a.line].join(" · ");
    return b;
  }

  /** Over the columns it takes tasks: dropped or pressed, terminumd says if it cannot; a dot opens its session, else a press opens its window. */
  function target(a) {
    const b = row(a);
    b.append(iconButton("agedit", "pencil", `Edit ${a.name}`, `Edit ${a.name}`, () => open(a)));
    b.addEventListener("click", (e) => {
      const tasks = given(), key = e.target.closest?.(".agdot")?.dataset.key;
      if (tasks) run(tasks, a.id);
      else if (key) openSession(key);
      else open(a);
    });
    b.addEventListener("dragover", (e) => {
      if (!dragging()) return;
      e.preventDefault();
      b.classList.add("over");                     // at its limit too: a drop there is offered its queue (AC-169)
    });
    b.addEventListener("dragleave", () => b.classList.remove("over"));
    b.addEventListener("drop", (e) => {
      const tasks = dragging();
      if (!tasks) return;                          // a file: the page's
      e.preventDefault();
      run(tasks, a.id);
    });
    return b;
  }

  let drawn = "";
  // a card under a press is not drawn anew: a read between the press and its release would take the card from under
  // the pointer, and the click with it (its window, its session); what came meanwhile is drawn once it lets go
  let pressed = false;
  list.addEventListener("pointerdown", () => { pressed = true; });
  for (const end of ["pointerup", "pointercancel"]) window.addEventListener(end, () => { if (pressed) { pressed = false; setTimeout(draw); } });
  /** Anew only when what it shows changed, and not under a press or a drag. */
  function draw() {
    const tasks = given(), agents = shownAgents();
    // the minute: a window's reset and a reading's age show without news
    const now = JSON.stringify([agents, chosen(), several(), counted().full, counted().full_projects, counted().kept, counted().queue, agents.map((a) => slots(a).map((s) => [s, doing(s)])), tasks?.length, readings(), Math.floor(Date.now() / 60000)]);
    if (dialog.open && editing) queue.show(editing, queueOf(editing));   // its window follows what other pages did
    if (pressed || dragging() || now === drawn) return;
    drawn = now;
    list.replaceChildren(...agents.map(target));
    if (!agents.length) list.append(el("p", "colempty", chosen().length === 1 ? `${projectName(chosen()[0]) ?? "This project"} has no agents yet: add one to run its tasks.` : "No agents yet: add one to run tasks."));
    hint.hidden = !tasks;
    if (tasks) hint.textContent = `${tasks.length} chosen: press an agent or drop them on it. Esc lets them go.`;
  }

  /** `project`'s pool as rows, for its window: a press opens one. */
  const rows = (project) => agentsOf(getMeta(), project).map((a) => {
    const b = row(a);
    b.addEventListener("click", () => open(a));
    return b;
  });

  /** The efforts of the kind chosen; one it does not take goes by the agent's own setting. */
  function fillEfforts(value) {
    const levels = getMeta().efforts?.[field.kind.value] ?? [];
    fillSelect(field.effort, [["", "Its own setting"], ...levels.map((e) => [e, e])], levels.includes(value) ? value : "");
  }

  const values = () => Object.fromEntries(Object.entries(field).map(([k, e]) => [k, e.value]));
  const say = (text) => showError(error, text);

  /** The name is still a kind's own, not the user's words. */
  const kindNamed = () => KINDS.some((k) => field.name.value.startsWith(agentName(k)));
  // what a kind fills in follows the kind; what the user wrote stays
  field.kind.addEventListener("change", () => {
    const kind = field.kind.value;
    if (!field.name.value.trim() || kindNamed()) field.name.value = freeName(kind, field.project.value);
    if (!field.command.value.trim() || KINDS.includes(field.command.value)) field.command.value = kind;
    fillEfforts(field.effort.value);
  });
  // a new one's name is free in the project chosen for it
  field.project.addEventListener("change", () => {
    if (kindNamed()) field.name.value = freeName(field.kind.value, field.project.value);
  });

  /** A kind's name not yet taken in `project`'s pool: Claude Code, Claude Code 2… */
  function freeName(kind, project) {
    const taken = new Set(agentsOf(getMeta(), project).map((x) => x.name));
    const base = agentName(kind);
    if (!taken.has(base)) return base;
    let n = 2;
    while (taken.has(`${base} ${n}`)) n++;
    return `${base} ${n}`;
  }

  const queue = queueList($("agentqueue"), { projectName, say, refresh: refreshQueue });

  /** Its window: a new one, in `project` or the one project shown (several: the human chooses), or `a` to change, which
   *  never moves it; `atQueue`: its queue shown first. */
  function open(a = null, atQueue = false, project = null) {
    editing = a;
    const meta = getMeta(), home = a?.project ?? project ?? onlyProject(chosen(), meta);
    fillSelect(field.project, projectOptions(meta, home), home);
    field.project.disabled = !!a;
    const v = a ?? { kind: "claude", name: freeName("claude", home), command: "claude", effort: "", at_once: 1, week_limit: null, month_limit: null };
    for (const k of ["kind", "name", "command", "at_once", "week_limit", "month_limit"]) field[k].value = v[k] ?? "";
    fillEfforts(v.effort);
    showView(a);
    say("");
    queue.show(a, a ? queueOf(a) : [], true);
    $("agenthead").textContent = a ? a.name : "Add agent";
    $("agentok").textContent = a ? "Save" : "Add";
    $("agentdrop").hidden = !a;
    snapshot = JSON.stringify(values());
    dialog.showModal();
    if (atQueue) queue.at();
    else field.name.select();
  }

  /** A change to terminumd, one at a time: the window closes when it is kept, and says why when it is not. */
  const kept = oneAtATime(dialog, async (path, options) => {
    await api(path, options);
    dialog.close();
    return true;
  });
  async function send(path, options) {
    try { if (await kept(path, options)) await reload(); } catch (e) { say(e.message); }
  }

  function keep() {
    const limit = (f) => (f.value === "" ? null : Number(f.value));
    const body = { ...values(), at_once: Number(field.at_once.value), week_limit: limit(field.week_limit), month_limit: limit(field.month_limit) };
    return send(editing ? `/api/task-agents/${editing.id}` : "/api/task-agents", { method: editing ? "PATCH" : "POST", body });
  }

  $("agentok").addEventListener("click", keep);
  dialog.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.isComposing && e.target.matches("input")) { e.preventDefault(); keep(); }
  });
  $("agentdrop").addEventListener("click", async () => {
    if (await askConfirm(`Delete the agent ${editing.name}? Its tasks and sessions stay.`, "Delete")) send(`/api/task-agents/${editing.id}`, { method: "DELETE" });
  });
  guardCancel(dialog, () => JSON.stringify(values()) === snapshot);   // Cancel means it

  /** Over its settings: its stripes, tokens and live sessions with their tasks (AC-138, AC-143, AC-168); each with Let go:
   *  a + Session's, or one on a host out of reach (AC-142). A task's link closes it and opens the task. */
  function showView(a) {
    const view = $("agentview");
    view.hidden = !a;
    if (!a) return view.replaceChildren();
    view.style.setProperty("--ag", agentColor(a));   // its stripes in its color, as on its rectangle
    const letGo = async (s, li) => {
      if (!await askConfirm(`Let go of ${s.tasks || "this session"}? Its open tasks fail; the window stays.`, "Let go")) return;
      try { await api(`/api/runs/${s.run}/let-go`, { method: "POST" }); li.remove(); } catch (e) { say(e.message); }
    };
    // what the user typed in its settings is asked about first, as Cancel does
    const leave = (go) => async (arg) => {
      if (JSON.stringify(values()) !== snapshot && !await askConfirm("Discard what you typed?", "Discard")) return;
      dialog.close();
      go(arg);
    };
    view.replaceChildren(agentView(a, readings()[a.kind], slots(a), {
      projectName, doing: (s) => WORDS[doing(s)], letGo,
      openTask: leave(openTask), openSession: leave(openSession) }));
  }

  /** A new one of `project` (none: of the project shown); once its window closes. */
  const add = (project = null) => {
    open(null, false, project);
    return new Promise((resolve) => dialog.addEventListener("close", resolve, { once: true }));
  };
  return { box, draw, add, rows };
}
