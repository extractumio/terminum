// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A task's window (AC-107, AC-108, AC-109, AC-113, AC-149): the task itself (its fields, when it was made and
// changed, text, files), its subtasks, then its comments and history as a conversation, the comment field last,
// its keys at the window's foot: Save keeps the comment, Run starts its agent with it at once (terminum-dispatcher.md
// AC-122). A task that waits for its human to react (its agent's questions, a result, a failure) shows that first,
// the comment field under it, then the task and the rest. The board and the session page open the same one (AC-150).
import { askConfirm, choose, oneAtATime } from "./dialogs.js";
import { el, iconButton, read, write } from "./listparts.js";
import { $, SEVERITIES, agentChip, agentsOf, api, button, download, fillSelect, guardQueuedRun, knownAgent, onFiles, onSend, madeChanged, renderInto, row, say, sessionOptions, statusTag, upload, withFileLink } from "./kanbanutil.js";
import { taskBoxes } from "./taskboxes.js";
import { activity, editor } from "./kanbanactivity.js";
import { family } from "./kanbansubs.js";
import { expandable } from "./expand.js";

const EDITS = ".tasktext textarea, .markdown textarea";   // a text or a comment being edited
const HELD = `${EDITS}, .xbox.full`;                     // or a box over the window (AC-184): nothing redraws under it

export function taskWindow({ getMeta, runs, openTask, newSubtask, openSchedule }) {
  taskBoxes();
  const box = $("taskbox"), body = $("taskbody");
  let shown = null, drawn = "", opening = 0, reading = 0;
  /** The comment being written: one field per task shown, kept through the window's redraws, with its files on
   *  their way; Save and Run wait for them (AC-133). */
  let note = null;
  const leftWaiting = new Map();                   // the comments of tasks left while their files were on their way
  let commentSave = null, commentRun = null, commentId = null, pendingKeys = null;
  /** Whether the window, as opened for its task, shows first what the human reacts to: kept while it stays open, so
   *  a change of the task turns no conversation round under its reader. */
  let leads = true;

  const flushKeys = () => { const paint = pendingKeys; pendingKeys = null; paint?.(); };
  const keyHeld = () => commentSave?.matches(":active") || commentRun?.matches(":active");
  // A release outside the key (or a cancelled touch) sends no click; give the footer its latest keys too.
  const released = () => {
    if (pendingKeys) requestAnimationFrame(() => { if (!keyHeld()) flushKeys(); });
  };
  for (const type of ["pointerup", "pointercancel", "keyup"]) document.addEventListener(type, released);

  /** A change of a field: whether terminumd took it. The window shows what stands; a text being
   *  typed stays (refresh). The board hears of it from terminumd. */
  const patch = async (id, change) => {
    let took = false;
    try {
      const answered = await api(`/api/tasks/${id}`, { method: "PATCH", body: change });
      if (answered.note) say(answered.note);
      took = true;
    } catch (e) {
      if (e.status === 409) return moveAcross(id, change);
      if (e.status === 412) return textChanged(id, change);
      say(e.message);
    }
    await refresh();
    return took;
  };

  /** A task with a session moving to another project: its session goes too, or is unlinked (AC-113). */
  async function moveAcross(id, change) {
    const how = await choose(`${id} has a session. What becomes of it?`, "A session stands in one project.", [["move", "Move the session too"], ["unlink", "Unlink the session"]]);
    if (how) return patch(id, { ...change, session_choice: how });
    refresh(true);
    return false;
  }

  /** Another changed the text while this one was written: nothing goes unasked; Cancel keeps writing (AC-133). */
  async function textChanged(id, { text_rev: _, ...change }) {
    if (await askConfirm("The text changed meanwhile. Put yours over it? The other text goes.", "Overwrite")) return patch(id, change);
    return false;
  }

  function select(label, options, value, changed) {
    const line = el("label", "fieldrow", label);
    const s = el("select", "field");
    fillSelect(s, options, value);
    s.addEventListener("change", () => changed(s.value));
    line.append(s);
    return line;
  }

  /** The time a RUN's agent has, in minutes; empty: none (terminum-dispatcher.md AC-126). */
  function budgetField(t) {
    const line = el("label", "fieldrow", "Budget, minutes");
    const f = el("input", "field");
    f.type = "number";
    f.min = "5";
    f.step = "5";
    f.value = t.budget_seconds ? String(t.budget_seconds / 60) : "";
    f.placeholder = "None";
    f.title = "Only for a session RUN started: 5 minutes to 7 days";
    f.addEventListener("change", () => patch(t.id, { budget_minutes: f.value ? Number(f.value) : null }));
    line.append(f);
    return line;
  }

  /** Where it stands, read only: its status in its color, the agent on it, what it waits for. */
  function standing(t) {
    const line = el("div", "fieldrow", "Status");
    const value = el("span", "fieldvalue");
    value.append(statusTag(t));
    if (t.agent) value.append(agentChip(t.agent));
    if (t.waits) value.append(el("span", "cstate waiting", `waits for ${t.waits}`));
    else if (t.session?.closed) value.append(el("span", "cstate closed", "session closed"));
    // the conversation RUN resumes, by its ID (terminum-dispatcher.md AC-157)
    if (t.agent?.conversation) value.append(el("span", "cconv", `conversation ${t.agent.conversation}`));
    // a new conversation on an agent chosen in RUN's window, apart from Run: seldom wanted (terminum-dispatcher.md AC-138)
    if ((t.session || t.agent?.conversation) && !t.waits && !t.run_holds) {
      const anew = button("linklike scratch", "Run from scratch", () => runTask(t, true));
      anew.title = "Start it anew on an agent you choose: its conversation is left, your comment goes with it";
      value.append(anew);
    }
    line.append(value);
    return line;
  }

  /** What is set on it, in the new-task window's order: three to a line, two on a phone. */
  function fields(t) {
    const meta = getMeta(), grid = el("div", "fieldgrid three");
    const key = t.session?.key ?? "";
    const sessions = sessionOptions(meta);
    if (key && !sessions.some(([k]) => k === key)) sessions.push([key, "Closed session"]);
    const owners = meta.users.map((u) => [u.id, u.name]);
    if (!owners.some(([u]) => u === t.owner)) owners.push([t.owner, t.owner_name]);
    grid.append(
      select("Project", meta.projects.map((p) => [p.id, p.name]), t.project, (project) => patch(t.id, { project })),
      select("Severity", SEVERITIES.map((s) => [s, s]), t.severity, (severity) => patch(t.id, { severity })),
      // the agent that runs it when Auto-Dispatcher starts it (terminum-dispatcher.md AC-170); none: its project's default
      select("Agent", [["", "Auto-Dispatcher's default"], ...agentsOf(meta, t.project).map((a) => [a.id, a.name])], t.runs_on ?? "", (agent) => api(`/api/tasks/${t.id}/agent`, { method: "PUT", body: { agent: agent || null } }).then(() => refresh(), (e) => say(e.message))),
      select("Owner", owners, t.owner, (owner) => patch(t.id, { owner })),
      select("Session", sessions, key, (session) => patch(t.id, { session: session || null })),
      budgetField(t));
    return grid;
  }

  /** Asked first, then `run`; a refusal is said; the window shows what stands. */
  const confirmed = async (question, yes, run) => {
    if (await askConfirm(question, yes)) {
      await run().catch((e) => say(e.message));
      refresh();
    }
  };

  /** Files for a text: each kept by terminumd, linked in the text (AC-106, AC-109); `each()` after each one. */
  async function addFiles(list, area, kept, each = () => {}) {
    for (const f of list) {
      try {
        const file = await upload(f, getMeta().most_bytes);
        kept(file);
        area.value = withFileLink(area.value, file);
        area.dispatchEvent(new Event("input"));
      } catch (e) { say(e.message); }
      each();
    }
  }

  /** A text field that takes files dropped or pasted, by `add(list)`. */
  function takesFiles(area, add) {
    area.classList.add("droparea");
    onFiles(area, area, add, (on) => area.classList.toggle("dropping", on));
  }

  function text(t) {
    const wrap = el("section", "tasktext");
    const head = el("div", "edithead"), keys = el("span", "seckeys");
    head.append(el("span", null, "Text"), keys);
    const shownText = t.text ? el("div", "markdown") : el("p", "tasknone", "No text");
    if (t.text) renderInto(shownText, t.text);
    /** Its editor, with files already dropped on the text shown. */
    let editing = false;
    const startEdit = (files = []) => {
      editing = true;
      wrap.classList.remove("droparea", "dropping");
      const area = editor(t.text, "Text, Markdown");
      area.rows = 10;
      area.placeholder = "Markdown. Drop or paste files here.";
      const dropped = [], kept = (file) => dropped.push(file.id);
      takesFiles(area, (list) => addFiles(list, area, kept));
      const save = button("primary", "Save", async () => { if (await patch(t.id, { text: area.value, text_rev: t.text_rev, files: dropped })) refresh(true); });
      // a secret pasted by mistake leaves the store for good by this (AC-109): seldom wanted, so only here
      const purge = t.text ? button("danger", "Purge text", () => confirmed(`Purge the text of ${t.id}? It leaves the store for good. Use this for a secret pasted by mistake.`, "Purge",
        () => api(`/api/tasks/${t.id}/purge`, { method: "POST" }).then(() => refresh(true)))) : null;
      keys.remove();
      wrap.replaceChildren(head, expandable(area), row(purge, button("fromright", "Cancel", () => refresh(true)), save));
      area.focus();
      addFiles(files, area, kept);
    };
    const edit = iconButton("icon", "pencil", "Edit", "Edit the text", () => startEdit());
    wrap.classList.add("droparea");
    onFiles(wrap, wrap, (list) => { if (!editing) startEdit(list); }, (on) => wrap.classList.toggle("dropping", on && !editing));
    keys.append(edit);
    wrap.append(head, t.text ? expandable(shownText) : shownText);
    return wrap;
  }

  function filesOf(t) {
    if (!t.files.length) return null;
    const wrap = el("div", "fieldrow", "Files"), list = el("ul", "filelist");
    list.append(...t.files.map((f) => {
      const li = el("li");
      const open = button("linklike", f.name, () => download(f.id, f.name).catch((e) => say(e.message)));
      const del = iconButton("icon", "x", `Delete ${f.name}`, "Delete this file", () => confirmed(`Delete ${f.name}? Its bytes leave the store for good.`, "Delete", () => api(`/api/task-files/${f.id}`, { method: "DELETE" })));
      li.append(open, el("small", null, `${Math.ceil(f.bytes / 1024)} KB`), del);
      return li;
    }));
    wrap.append(list);
    return wrap;
  }

  /** Task `id`'s comment field, kept as typed (its draft) with its files: the one already written in, else a new one. */
  function noteFor(id) {
    if (note?.id === id) return note;
    if (note?.uploads) leftWaiting.set(note.id, note);
    if (leftWaiting.has(id)) { note = leftWaiting.get(id); leftWaiting.delete(id); return note; }
    const draft = `tm.comment.${id}`, filesKey = `tm.commentfiles.${id}`;
    const area = editor(read(draft) ?? "", "Comment");
    area.rows = 4;
    area.addEventListener("input", () => { write(draft, area.value); n.wait?.(); });   // Save: only with something to keep
    // the files of a comment being written: kept with its text, or the sweep would take them
    const wrap = el("section", "newcomment");
    wrap.append(expandable(area));
    const n = note = { id, area, wrap, uploads: 0, keys: [], dropped: () => read(filesKey) ?? [] };
    /** What was sent goes: the field is empty, the draft and its files too. */
    n.sent = () => { write(draft, null); write(filesKey, null); area.value = area.defaultValue = ""; };
    takesFiles(area, async (list) => {
      if (box.dataset.busy) return say("Wait until this action finishes, then add the files.");
      n.uploads += list.length;
      n.wait();
      await addFiles(list, area, (file) => write(filesKey, [...n.dropped(), file.id]), () => {
        n.uploads--;
        n.wait();
        if (!n.uploads) leftWaiting.delete(n.id);
      });
    });
    onSend(area, () => n.keys[0]?.click());        // the Save shown now: off while its files are on their way
    return n;
  }

  /** Save keeps the comment once: while terminumd answers, Save and ⌘Enter send nothing more, Esc waits and the
   *  text stands still (AC-165); refused, it stays as it was. The window stays, the comment in its conversation. */
  async function keepComment(n, t) {
    if (!n.area.value.trim()) return;
    try {
      await api(`/api/tasks/${t.id}/comments`, { method: "POST", body: { text: n.area.value, files: n.dropped() } });
    } catch (e) { say(e.message); return; }
    n.sent();
  }

  const commentWork = oneAtATime(box, (work) => work());
  /** An action freezes its draft; when its lock ends the current queue and upload state still own the keys. */
  async function commentAction(n, work) {
    const focused = document.activeElement, mine = opening;
    const current = () => box.open && opening === mine && shown?.id === n.id;
    try { await commentWork(() => work(current)); } finally {
      await refresh(false, true);                  // the action held any task changes it heard while busy
      note?.wait();
      if (current() && box.contains(focused) && focused.isConnected && !focused.matches(":disabled")
        && (document.activeElement === document.body || document.activeElement === box)) focused.focus({ preventScroll: true });
    }
  }

  /** Run with the comment being written: on in its session or its conversation, or a first run on its agent, at
   *  once; RUN's window opens only for what is left to choose (terminum-dispatcher.md AC-122). `scratch`: a new
   *  conversation on an agent chosen there. Started, the comment is sent and the window closes. */
  function runTask(t, scratch = false) {
    const n = noteFor(t.id);
    return commentAction(n, async (current) => {
      try {
        if (await runs.open([t], { words: n.area.value, files: n.dropped(), current, scratch, prefer: t.runs_on, atOnce: !scratch }) !== true) return;
      } catch (e) { say(e.message); return; }
      n.sent();
      if (current()) await tryClose();
    });
  }

  /** The agent a first Run of `t` starts on at once, by name: its own, its project's last, the pool's only one;
   *  null for a later run, or with none known: RUN's window asks (kanbanrun.js). */
  function firstOn(t) {
    if (t.session || t.agent?.conversation) return null;
    const meta = getMeta();
    return knownAgent(agentsOf(meta, t.project), [t.runs_on, meta.last_agents?.[t.project]])?.name ?? null;
  }

  /** The window's keys at its foot, for `t` as it stands: what is seldom pressed at the left (Archive, Stop waiting,
   *  Give up in the danger color), Save and Run at the right. */
  function commentKeys(t, n) {
    // A refresh keeps the keys under the press; a different task gets keys of its own.
    if (commentId !== t.id) {
      commentSave = button("fromright", "Save");
      commentRun = button(null, "Run");
      for (const key of [commentSave, commentRun]) key.addEventListener("click", flushKeys); // before its action handler
      commentId = t.id;
    }
    const save = commentSave, run = commentRun;
    const busy = t.waits || (t.run_holds && t.status === "IN PROGRESS"), on = firstOn(t);
    save.classList.toggle("primary", Boolean(busy));
    save.onclick = () => commentAction(n, () => keepComment(n, t));
    run.classList.toggle("primary", !busy);
    run.onclick = () => runTask(t);
    n.keys = [save, run];
    n.wait = () => {
      const waiting = n.uploads > 0 || Boolean(box.dataset.busy);
      save.disabled = waiting || !n.area.value.trim();
      run.title = on ? `Run it on ${on}, with this comment` : "Run it with this comment";
      guardQueuedRun(run, t.waits);
      run.disabled ||= waiting;
    };
    n.wait();
    save.title = "Keep the comment: nothing moves, the session hears nothing";
    // its run cannot be reached or is no longer wanted: its tasks fail, the window stays (terminum-dispatcher.md AC-138)
    const giveUp = t.run_holds ? button("danger", "Give up", () => confirmed(`Give up on this run? ${t.run_holds} go to FAILED. Dispatcher lets go of its window, which stays open, and an agent in it may go on.`, "Give up",
      () => api(`/api/tasks/${t.id}/give-up`, { method: "POST" }))) : null;
    // a RUN waiting for its agent's room is called off by this; the task stays where it is (AC-158)
    const stopWaiting = t.waits ? button(null, "Stop waiting", () => api(`/api/tasks/${t.id}/give-up`, { method: "POST" }).then(() => refresh(), (e) => say(e.message))) : null;
    if (stopWaiting) stopWaiting.title = `Run waits for ${t.waits}: call it off, nothing starts`;
    // a task nobody needs leaves the board by this (AC-118); it stands where it stood when it comes back
    const shelve = button(null, t.archived_at ? "Bring back" : "Archive", () => api(`/api/tasks/${t.id}/archive`, { method: "POST", body: { on: !t.archived_at } }).then(() => refresh(), (e) => say(e.message)));
    // its keys at the window's foot, in sight while the task scrolls above them
    const acts = $("taskacts"), paint = () => {
      for (const key of [...acts.children]) if (key !== save && key !== run) key.remove();
      if (save.parentNode !== acts) acts.append(save);
      save.before(...[t.run_holds ? null : shelve, stopWaiting, giveUp].filter(Boolean));
      if (save.nextElementSibling !== run) save.after(run);
    };
    // A pressed footer key keeps its place until the click or cancellation.
    pendingKeys = keyHeld() ? () => { if (save.isConnected && box.open) paint(); } : null;
    if (!pendingKeys) paint();
  }

  /** A change terminumd must take, or it throws its refusal (a dialog says it); the window shows what stands. */
  const send = async (id, change) => {
    await api(`/api/tasks/${id}`, { method: "PATCH", body: change });
    refresh();
  };

  /** The task itself, set apart from what was said of it: its fields, who has it, when it was made and changed (AC-149). */
  function own(t) {
    const wrap = el("section", "tasksec taskown");
    const stamps = el("p", "taskstamps", `${t.uid} · ${madeChanged(t)}`);
    stamps.title = `${t.id} is also found by ${t.uid}, whatever its ID becomes`;
    // made by a schedule: its name, a press opening its window (terminum-schedules.md AC-163)
    const from = t.schedule ? el("p", "taskfrom") : null;
    if (from) from.append("From schedule ", button("linklike", `${t.schedule.key}: ${t.schedule.title}`, () => openSchedule(t.schedule.key)));
    const when = el("div", "taskwhen");
    when.append(...[stamps, from].filter(Boolean));
    wrap.append(...[when, standing(t), fields(t), text(t), filesOf(t)].filter(Boolean));
    return wrap;
  }

  function draw(t) {
    shown = t;
    drawn = JSON.stringify(t);
    $("tasktitle").textContent = `${t.id} ${t.title}`;
    const scroll = body.scrollTop, n = noteFor(t.id), kin = family(t, { openTask, newSubtask, patch, send });
    const { lead, rest } = activity(t, { refresh, confirmed, leads });
    leads = Boolean(lead);
    n.area.placeholder = `${lead && t.status === "WAIT FOR HUMAN" ? "Your answer" : "Comment"}, Markdown. Drop or paste files here.`;
    // the comment being written stays where it is, never taken out: its focus, cursor and files on their way stay too
    if (n.wrap.parentNode !== body) body.replaceChildren(n.wrap);
    for (const c of [...body.children]) if (c !== n.wrap) c.remove();
    // what the human reacts to first and the field to answer in under it, then the task and the rest; else read in order
    if (lead) {
      n.wrap.before(lead);
      n.wrap.after(own(t), rest, kin);
    } else n.wrap.before(own(t), kin, rest);
    commentKeys(t, n);
    body.scrollTop = scroll;
  }

  /** The newest asked is the one that shows, whichever answers last; whether this read opened it. */
  async function open(id) {
    const mine = ++opening;
    reading++;                                     // a read of what showed before stands over nothing
    let t;
    try { t = await api(`/api/tasks/${id}`); } catch (e) {
      if (mine !== opening) return false;
      throw e;
    }
    if (mine !== opening) return false;
    const anew = !box.open || shown?.id !== t.id;
    if (anew) leads = true;
    if (!box.open) {                              // reopening is a new press target, even for the same task,
      commentId = null;
      if (!note?.uploads) note = null;            // and its comment is read from its draft again, unless files are on their way
    }
    draw(t);
    if (!box.open) box.showModal();
    if (anew) body.scrollTop = 0;                 // a task opened is read from its start
    return true;
  }

  /** A text being edited, changed from what shows. */
  const edited = () => [...body.querySelectorAll(EDITS)].some((a) => a.value !== a.defaultValue);

  /** Closes it; an edit not saved is asked about first. Whether it closed. */
  async function tryClose() {
    if (box.open && edited() && !await askConfirm("Discard your edit? It is not saved.", "Discard")) return false;
    opening++;                                  // a read still waiting cannot reopen what the user closed
    pendingKeys = null;
    box.close();
    return true;
  }

  /** Reads it again; a text being edited holds until it is saved (`saved`). What the page only heard
   *  of (`news`) and answers the same draws nothing: no Markdown again, no picture fetched again. */
  async function refresh(saved = false, news = false) {
    if (!box.open || !shown) return;
    const editing = () => box.dataset.busy || (!saved && body.querySelector(HELD));   // a comment being kept: it closes
    if (editing()) return;
    const mine = ++reading;
    try {
      const t = await api(`/api/tasks/${shown.id}`);
      if (mine !== reading) return;              // a newer read, or another task opened meanwhile: that one stays
      if (!editing() && (!news || JSON.stringify(t) !== drawn)) draw(t);   // an editor opened while it was read stays
    } catch (e) { say(e.message); }
  }

  $("taskclose").addEventListener("click", tryClose);
  box.addEventListener("cancel", (e) => { if (e.defaultPrevented) return; e.preventDefault(); tryClose(); });   // held while a comment is kept (AC-165)
  box.addEventListener("close", () => { if (!box.open) pendingKeys = null; });
  return { open, refresh, tryClose, shows: (id) => box.open && shown?.id === id, shownId: () => shown?.id };
}
