// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The pool over the columns, and its window (terminum-dispatcher.md AC-137, AC-138): what a task is dropped
// on to run: each a rectangle with its color, name, kind, effort, its subscription's stripes (AC-167) and a dot per
// live session; its window shows those, its tokens and its sessions' tasks over its settings. None is a project's.
// terminumd checks what is kept and says what is wrong; every page hears.
import { askConfirm, oneAtATime } from "./dialogs.js";
import { agentMark, agentName, metaLine } from "./nav.js";
import { el, iconButton } from "./listparts.js";
import { agentView, stripes } from "./kanbanagentview.js";
import { $, agentColor, api, fillSelect, guardCancel, showError } from "./kanbanutil.js";

const KINDS = ["claude", "codex"];                 // their commands are their names: what a kind fills in

/** `getMeta()`: terminumd's agents; `chosen()`: the projects shown, none for all; `slots()`: what each agent's slots hold, as terminumd counts them;
 *  `liveRow(key)`: a session's row; `dragging()`: the tasks of a drag; `given()`: those a press or a drop would run on an
 *  agent; `run(tasks, agent)`; `readings()`: each kind's subscription windows (AC-167); `openTask(id)`. */
export function agentsSection({ getMeta, chosen, reload, slots: counted, liveRow, dragging, given, run, openSession, readings, openTask }) {
  // on the columns' tracks: an agent a column wide, its keys in the fifth (kanban.css .tracks)
  const box = el("section", "agents tracks"), list = el("div", "agentlist trackitems"), side = el("div", "trackside");
  const addKey = iconButton("icon", "plus", "Add agent", "Add an agent to the pool", () => open());
  addKey.id = "addagent";
  const hint = el("p", "pickhint");
  hint.hidden = true;
  side.append(addKey, hint);
  box.append(list, side);
  const field = { name: $("agentname"), kind: $("agentkind"), effort: $("agenteffort"), command: $("agentcommand"), at_once: $("agentatonce"), week_limit: $("agentweek"), month_limit: $("agentmonth") };
  const dialog = $("agentbox"), error = $("agenterror");
  let editing = null, snapshot = "";

  const slots = (a) => counted().agents[a.id] ?? [];
  /** At a limit, as terminumd says: its own, or the one project shown's. */
  const full = (a) => counted().full.includes(a.id) || (chosen().length === 1 && counted().full_projects.includes(chosen()[0]));

  const WORDS = { starting: "starting", working: "working", waits: "waits for you", idle: "idle" };
  /** What a session does now. */
  function doing(s) {
    if (s.idle) return "idle";
    const row = s.key ? liveRow(s.key) : null;
    return ["starting", "typed"].includes(s.state) ? "starting" : s.state === "asking" || row?.state === "waiting" ? "waits" : "working";
  }

  /** A session's dot: blinking while it works, still when it waits or idles; a press on it opens it. */
  function dot(s) {
    const now = doing(s), d = el("span", `agdot ${now === "working" || now === "starting" ? "on" : now}`);
    d.dataset.key = s.key ?? "";
    d.title = [s.tasks, getMeta().projects.find((p) => p.id === s.project)?.name, WORDS[now]].filter(Boolean).join(" · ");
    return d;
  }

  /** Its rectangle: its color, name, kind, effort and a dot for each session of what is shown; its limit counts them all. */
  function row(a) {
    const shown = chosen(), mine = slots(a).filter((s) => !shown.length || shown.includes(s.project));
    const b = el("button", `item agentcard${full(a) ? " busy" : ""}`);
    b.type = "button";
    b.dataset.agent = a.id;
    const bar = el("span", `pbar${slots(a).length ? " busy" : ""}`);
    bar.style.setProperty("--dot", agentColor(a));
    b.style.setProperty("--ag", agentColor(a));
    const txt = el("span", "txt"), line = el("span", "head"), dots = el("span", "agdots");
    // a clock for each slot its schedules keep, before its sessions' dots (terminum-schedules.md AC-161)
    const kept = counted().kept?.[a.id] ?? [];
    for (const k of kept) { const m = el("span", "agkept"); m.title = `kept for ${k}`; m.setAttribute("aria-label", `kept for ${k}`); dots.append(m); }
    dots.append(...mine.map(dot));
    const used = slots(a).filter((s) => !s.kept).length + kept.length;
    if (a.at_once > 1 || kept.length) dots.append(el("small", "agcount", `${used}/${a.at_once}`));
    line.append(agentMark({ agent: a.kind }), el("span", "t", a.name));
    txt.append(line, metaLine([agentName(a.kind), ...(a.effort ? [a.effort] : [])], false), stripes(a.kind, readings()[a.kind], Date.now() / 1000), dots);
    b.append(bar, txt);
    b.title = [a.name, `${used} of ${a.at_once} at once`, ...(kept.length ? [`kept for ${kept.join(", ")}`] : []), a.line].join(" · ");
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
      if (!full(a)) b.classList.add("over");
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
  /** Anew only when what it shows changed, and not under a drag: a press spanning a read keeps its key. */
  function draw() {
    const tasks = given(), agents = getMeta().agents;
    // the minute: a window's reset and a reading's age show without news
    const now = JSON.stringify([agents, chosen(), counted().full, counted().full_projects, counted().kept, agents.map((a) => slots(a).map((s) => [s, doing(s)])), tasks?.length, readings(), Math.floor(Date.now() / 60000)]);
    if (dragging() || now === drawn) return;
    drawn = now;
    list.replaceChildren(...agents.map(target));
    if (!agents.length) list.append(el("p", "colempty", "No agents yet: add one to run tasks."));
    hint.hidden = !tasks;
    if (tasks) hint.textContent = `${tasks.length} chosen: press an agent or drop them on it. Esc lets them go.`;
  }

  /** The pool's rows, for a project's window: a press opens one. */
  const rows = () => getMeta().agents.map((a) => {
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

  // what a kind fills in follows the kind; what the user wrote stays
  field.kind.addEventListener("change", () => {
    const kind = field.kind.value;
    if (!field.name.value.trim() || KINDS.some((k) => field.name.value.startsWith(agentName(k)))) field.name.value = freeName(kind);
    if (!field.command.value.trim() || KINDS.includes(field.command.value)) field.command.value = kind;
    fillEfforts(field.effort.value);
  });

  /** A kind's name not yet taken in the pool: Claude Code, Claude Code 2… */
  function freeName(kind) {
    const taken = new Set(getMeta().agents.map((x) => x.name));
    const base = agentName(kind);
    if (!taken.has(base)) return base;
    let n = 2;
    while (taken.has(`${base} ${n}`)) n++;
    return `${base} ${n}`;
  }

  /** Its window: a new one, or `a` to change. */
  function open(a = null) {
    editing = a;
    const v = a ?? { kind: "claude", name: freeName("claude"), command: "claude", effort: "", at_once: 1, week_limit: null, month_limit: null };
    for (const k of ["kind", "name", "command", "at_once", "week_limit", "month_limit"]) field[k].value = v[k] ?? "";
    fillEfforts(v.effort);
    showView(a);
    $("agenthead").textContent = a ? a.name : "Add agent";
    $("agentok").textContent = a ? "Save" : "Add";
    $("agentdrop").hidden = !a;
    say("");
    snapshot = JSON.stringify(values());
    dialog.showModal();
    field.name.select();
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

  /** Over its settings: its stripes, tokens and live sessions with their tasks (AC-138, AC-143, AC-167); each with Let go:
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
      projectName: (id) => getMeta().projects.find((p) => p.id === id)?.name, doing: (s) => WORDS[doing(s)], letGo,
      openTask: leave(openTask), openSession: leave(openSession) }));
  }

  /** A new one; once its window closes. */
  const add = () => {
    open();
    return new Promise((resolve) => dialog.addEventListener("close", resolve, { once: true }));
  };
  return { box, draw, add, rows };
}
