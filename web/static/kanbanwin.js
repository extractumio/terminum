// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A task's window (AC-107, AC-108, AC-109, AC-113, AC-149): the task itself (its fields, when it was made and
// changed, text, files), its subtasks, then its comments and history apart. A comment by a human closes with
// CLOSE; RUN comes with Dispatcher. The board and the session page open the same one (AC-150).
import { askConfirm, choose, oneAtATime } from "./dialogs.js";
import { el, iconButton, read, write } from "./listparts.js";
import { $, SEVERITIES, agentChip, api, button, download, fileMarkdown, fillSelect, onFiles, onSend, madeChanged, renderInto, row, say, sessionOptions, statusTag, upload } from "./kanbanutil.js";
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
   *  their way; Close and Run wait for them (AC-133). */
  let note = null;
  const leftWaiting = new Map();                   // the comments of tasks left while their files were on their way

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
      select("Agent", [["", "Auto-Dispatcher's default"], ...meta.agents.map((a) => [a.id, a.name])], t.runs_on ?? "", (agent) => api(`/api/tasks/${t.id}/agent`, { method: "PUT", body: { agent: agent || null } }).then(() => refresh(), (e) => say(e.message))),
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
        area.value += `${area.value && !area.value.endsWith("\n") ? "\n" : ""}${fileMarkdown(file)}\n`;
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
      keys.remove();
      wrap.replaceChildren(head, expandable(area), row(button(null, "Cancel", () => refresh(true)), save));
      area.focus();
      addFiles(files, area, kept);
    };
    const edit = button("textkey", "Edit", () => startEdit());
    wrap.classList.add("droparea");
    onFiles(wrap, wrap, (list) => { if (!editing) startEdit(list); }, (on) => wrap.classList.toggle("dropping", on && !editing));
    const purge = button("textkey", "Purge text", () => confirmed(`Purge the text of ${t.id}? It leaves the store for good. Use this for a secret pasted by mistake.`, "Purge",
      () => api(`/api/tasks/${t.id}/purge`, { method: "POST" })));
    keys.append(edit);
    if (t.text) keys.append(purge);
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
    area.placeholder = "Comment, Markdown. Drop or paste files here.";
    area.addEventListener("input", () => write(draft, area.value));
    // the files of a comment being written: kept with its text, or the sweep would take them
    const n = note = { id, area, uploads: 0, keys: [], dropped: () => read(filesKey) ?? [] };
    n.wait = () => { for (const k of n.keys) k.disabled = n.uploads > 0; };
    /** What was sent goes: the field is empty, the draft and its files too. */
    n.sent = () => { write(draft, null); write(filesKey, null); area.value = area.defaultValue = ""; if (note === n) note = null; };
    takesFiles(area, async (list) => {
      if (box.dataset.busy) return say("Wait until the comment is kept, then add the files.");
      n.uploads += list.length;
      n.wait();
      await addFiles(list, area, (file) => write(filesKey, [...n.dropped(), file.id]), () => {
        n.uploads--;
        n.wait();
        if (!n.uploads) leftWaiting.delete(n.id);
      });
    });
    onSend(area, () => n.keys[0]?.click());        // the Close shown now: off while its files are on their way
    return n;
  }

  /** Close keeps the comment once: while terminumd answers, Close and ⌘Enter send nothing more, Esc waits and the
   *  text stands still (AC-165); refused, it stays as it was. */
  const keepComment = oneAtATime(box, async (n, t) => {
    if (n.area.value.trim()) {
      try {
        await api(`/api/tasks/${t.id}/comments`, { method: "POST", body: { text: n.area.value, files: n.dropped() } });
      } catch (e) { say(e.message); return; }
      n.sent();
    }
    await tryClose();                              // an edit of its text or of a comment not saved is asked about
  });

  function commentForm(t) {
    const wrap = el("section", "newcomment"), n = noteFor(t.id), area = n.area;
    const close = button(t.run_holds ? "primary" : null, "Close", () => keepComment(n, t));
    // Run: the comment goes with it, and on in its session or a new one (terminum-dispatcher.md AC-122)
    const run = button(t.run_holds ? null : "primary", "Run", async () => {
      try {
        if (await runs.open([t], { words: area.value, files: n.dropped() }) !== true) return;
      } catch (e) { say(e.message); return; }
      n.sent();
      tryClose();
    });
    n.keys = [close, run];
    n.wait();
    close.title = "Keep the comment and close: nothing moves, the session hears nothing";
    // its run cannot be reached or is no longer wanted: its tasks fail, the window stays (terminum-dispatcher.md AC-138)
    const giveUp = t.run_holds ? button(null, "Give up", () => confirmed(`Give up on this run? ${t.run_holds} go to FAILED. Dispatcher lets go of its window, which stays open, and an agent in it may go on.`, "Give up",
      () => api(`/api/tasks/${t.id}/give-up`, { method: "POST" }))) : null;
    // a RUN waiting for its agent's room is called off by this; the task stays where it is (AC-158)
    const stopWaiting = t.waits ? button(null, "Stop waiting", () => api(`/api/tasks/${t.id}/give-up`, { method: "POST" }).then(() => refresh(), (e) => say(e.message))) : null;
    if (stopWaiting) stopWaiting.title = `Run waits for ${t.waits}: call it off, nothing starts`;
    // a task nobody needs leaves the board by this (AC-118); it stands where it stood when it comes back
    const shelve = button(null, t.archived_at ? "Bring back" : "Archive", () => api(`/api/tasks/${t.id}/archive`, { method: "POST", body: { on: !t.archived_at } }).then(() => refresh(), (e) => say(e.message)));
    shelve.classList.add("evenleft");
    wrap.append(expandable(area));
    // its keys at the window's foot, in sight while the task scrolls above them
    $("taskacts").replaceChildren(...[t.run_holds ? null : shelve, giveUp, stopWaiting, close, t.run_holds || t.waits ? null : run].filter(Boolean));
    return wrap;
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
    const scroll = body.scrollTop, typing = note && document.activeElement === note.area ? [note.area.selectionStart, note.area.selectionEnd] : null;
    const kin = family(t, { openTask, newSubtask, patch, send });
    body.replaceChildren(own(t), kin, activity(t, { refresh, confirmed }), commentForm(t));
    body.scrollTop = scroll;
    if (typing && note?.id === t.id) { note.area.focus({ preventScroll: true }); note.area.setSelectionRange(...typing); }
  }

  /** The newest asked is the one that shows, whichever answers last; one closed or called off (`forget`) meanwhile
   *  shows nothing. */
  async function open(id) {
    const mine = ++opening;
    reading++;                                     // a read of what showed before stands over nothing
    const t = await api(`/api/tasks/${id}`);
    if (mine !== opening) return;
    draw(t);
    if (!box.open) box.showModal();
  }

  /** A text being edited, changed from what shows. */
  const edited = () => [...body.querySelectorAll(EDITS)].some((a) => a.value !== a.defaultValue);

  /** Closes it; an edit not saved is asked about first. Whether it closed. */
  async function tryClose() {
    if (edited() && !await askConfirm("Discard your edit? It is not saved.", "Discard")) return false;
    shut();
    return true;
  }

  /** Closed, or called off before it showed (Back): an open still on its way shows nothing. */
  const forget = () => { opening++; };
  const shut = () => { forget(); box.close(); };

  /** Reads it again; a text being edited holds until it is saved (`saved`). What the page only heard
   *  of (`news`) and answers the same draws nothing: no Markdown again, no picture fetched again. */
  async function refresh(saved = false, news = false) {
    if (!box.open || !shown) return;
    const editing = () => box.dataset.busy || (!saved && body.querySelector(HELD));   // a comment being kept: it closes
    if (editing()) return;
    const mine = ++reading;
    try {
      const t = await api(`/api/tasks/${shown.id}`);
      if (mine !== reading || t.id !== shown.id) return;   // a newer read, or another task opened meanwhile: that one stays
      if (!editing() && (!news || JSON.stringify(t) !== drawn)) draw(t);   // an editor opened while it was read stays
    } catch (e) { say(e.message); }
  }

  $("taskclose").addEventListener("click", tryClose);
  box.addEventListener("cancel", (e) => { if (e.defaultPrevented) return; if (edited()) { e.preventDefault(); tryClose(); } else forget(); });
  return { open, refresh, tryClose, forget, shows: (id) => box.open && shown?.id === id, shownId: () => shown?.id };
}
