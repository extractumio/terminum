// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The new-task window (AC-106): a title, Markdown with Preview, files dropped or pasted into the
// text, in TODO unless said. A draft survives closing and a reload on this browser.
import { askConfirm, oneAtATime } from "./dialogs.js";
import { el, iconButton, read, write } from "./listparts.js";
import { $, AT_LIMIT, agentsOf, api, fileMarkdown, FILE_GONE, fillAgents, fillSelect, goneFiles, goneSaid, onFiles, onSend, onlyProject, projectOptions, sessionOptions, showError, upload, withFileLink, withoutFileLinks, writePreview } from "./kanbanutil.js";

const DRAFT = "tm.taskdraft";

/** `onRun(task, agent)`: a task made by Run starts at once on the agent chosen (terminum-dispatcher.md AC-138);
 *  `slots()`: who is at a limit, as terminumd counts it. */
export function taskForm({ getMeta, getFilter, onMade, onRun, slots }) {
  const box = $("newbox");
  let files = [], offered = "", parent = null, chose = "";   // chose: the agent the user picked in this window, "" for none
  /** Each its own draft: a subtask's never takes a new task's. */
  const draftKey = () => (parent ? `${DRAFT}.${parent}` : DRAFT);
  /** Each draft's files on their way, and its generation: one sent or discarded is another draft from then on, so an
   *  upload that ends late never lands in it (AC-133). */
  const uploads = new Map(), ended = new Map();
  const pending = (key = draftKey()) => uploads.get(key) ?? 0;
  const end = () => { ended.set(draftKey(), (ended.get(draftKey()) ?? 0) + 1); uploads.delete(draftKey()); files = []; write(draftKey(), null); };
  /** Create waits for the files on their way: pressed earlier, they would not be on the task (AC-133). */
  const canCreate = () => { if (!box.dataset.busy) $("newcreate").disabled = $("newrun").disabled = pending() > 0; };

  const error = (text) => showError($("newerror"), text);
  /** Said as the window opens with no project chosen, terminumd's words for it (tasks/ops.rs TASK_NEEDS_PROJECT). */
  let unchosen = false;
  const draft = () => ({ title: $("newtitle").value, text: $("newtext").value, project: $("newproj").value, severity: $("newsev").value, owner: $("newowner").value, session: $("newsession").value, files });
  const keep = () => write(draftKey(), draft());

  function showFiles() {
    $("newfiles").replaceChildren(...files.map((f) => {
      const li = el("li", null, f.name);
      li.append(iconButton("icon", "x", `Remove ${f.name}`, "Remove this file", () => { forget([f]); keep(); }));
      return li;
    }));
  }

  /** Files out of the draft, their links too. */
  function forget(gone) {
    files = files.filter((o) => !gone.includes(o));
    $("newtext").value = withoutFileLinks($("newtext").value, gone.map((f) => f.id)).text;
    showFiles();
  }

  /** Its files terminumd let go leave the draft shown, and the window names them; Create keeps the rest (AC-106).
   *  Whether any went. */
  async function prune() {
    const key = draftKey(), gone = await goneFiles(files.map((f) => f.id));
    const lost = files.filter((f) => gone.includes(f.id));
    if (!lost.length || key !== draftKey() || !box.open) return false;
    forget(lost);
    keep();
    error(goneSaid(lost.map((f) => f.name)));
    return true;
  }

  function insert(text) {
    const t = $("newtext"), at = t.selectionStart ?? t.value.length;
    t.value = `${t.value.slice(0, at)}${text}${t.value.slice(t.selectionEnd ?? at)}`;
    t.selectionStart = t.selectionEnd = at + text.length;
    keep();
  }

  /** A file kept for a draft that is not shown: it goes into that draft as stored, linked at its end. */
  function intoStored(key, kept) {
    const d = read(key);
    if (d) write(key, { ...d, files: [...(d.files ?? []), kept], text: withFileLink(d.text ?? "", kept) });
  }

  /** Kept by terminumd at once, linked at the cursor of the draft it was added to; a refusal keeps the window and
   *  says why. None while the task is being made: what is sent is what was shown. */
  async function add(list) {
    if (box.dataset.busy) return error("Wait until the task is made, then add the files.");
    error(null);
    const key = draftKey(), gen = ended.get(key) ?? 0;
    const mine = () => (ended.get(key) ?? 0) === gen;
    uploads.set(key, pending(key) + list.length);
    canCreate();
    for (const f of list) {
      let kept = null;
      try { kept = await upload(f, getMeta().most_bytes); } catch (e) { if (mine() && key === draftKey()) error(e.message); }
      if (!mine()) continue;                         // its draft was sent or discarded meanwhile: the store's sweep takes the file
      uploads.set(key, pending(key) - 1);
      if (kept && key !== draftKey()) intoStored(key, kept);
      else if (kept) { files.push(kept); insert(`${fileMarkdown(kept)}\n`); }
      if (key === draftKey()) { showFiles(); keep(); }
    }
    canCreate();
  }

  const view = writePreview("new", $("newtext"), $("newshown"));

  /** The chosen project's pool: only an explicit choice picks an agent (AC-106); otherwise assignment chooses later.
   *  One at a limit is said so: Run offers its queue (terminum-dispatcher.md AC-169). */
  function showAgents() {
    const meta = getMeta(), project = $("newproj").value, all = agentsOf(meta, project), now = slots();
    const busy = (a) => (now.full.includes(a.id) ? AT_LIMIT : now.full_projects.includes(project) ? `the project is ${AT_LIMIT}` : "");
    fillAgents($("newagent"), all, busy, [chose, ""], !all.length ? "No agents yet" : "Choose when it runs");
  }

  /** `given.parent`: a subtask of that task, in its project (AC-112). */
  function open(given = {}) {
    parent = given.parent ?? null;
    const meta = getMeta(), saved = read(draftKey()) ?? {};
    $("newhead").textContent = parent ? `New subtask of ${parent}` : "New task";
    $("newproj").disabled = !!parent;
    const filter = getFilter().project;   // a draft with nothing typed says nothing of where it goes
    const project = given.project ?? (saved.title || saved.text ? saved.project : null) ?? onlyProject(filter, meta);
    fillSelect($("newproj"), projectOptions(meta, project), project);
    chose = "";
    showAgents();
    fillSelect($("newsession"), sessionOptions(meta), given.session ?? saved.session ?? "");
    $("newsev").value = saved.severity ?? "P2";
    fillSelect($("newowner"), meta.users.map((u) => [u.id, u.name]), saved.owner ?? "human");
    $("newtitle").value = given.title ?? saved.title ?? "";
    $("newtext").value = given.text ? `${saved.text ? `${saved.text}\n\n` : ""}${given.text}` : saved.text ?? "";
    files = saved.files ?? [];
    showFiles();
    canCreate();
    view(false);
    unchosen = !project;
    error(unchosen ? "Choose the project to make it in." : null);
    box.showModal();
    $("newtitle").focus();
    prune();
  }

  /** Made once: while terminumd answers, a second press, Enter or ⌘Enter sends nothing, Esc waits and the text
   *  stands still (AC-165); kept, that draft ends; refused, it stays as it was, saying why. */
  const send = oneAtATime(box, async (run) => {
    const d = draft(), agent = $("newagent").value;
    try {
      const made = await api("/api/tasks", { method: "POST", body: { title: d.title, text: d.text, project: d.project, severity: d.severity, owner: d.owner || undefined, agent: agent || undefined, session: d.session || undefined, parent: parent ?? undefined, files: d.files.map((f) => f.id) } });
      end();
      box.close();
      if (run) onRun(made, agent); else onMade(made);
    } catch (e) { if (e.message !== FILE_GONE || !await prune()) error(e.message); $("newtitle").focus(); }   // a file let go meanwhile: named
  });
  function create(run = false) {
    if ($("newcreate").disabled) return;           // a file on its way, or the task being made
    error(null);
    return send(run);
  }

  $("newcreate").addEventListener("click", () => create());
  $("newrun").addEventListener("click", () => create(true));
  // Enter in the title is Create; what is typed never goes by a slip: Discard asks
  box.querySelector("form").addEventListener("submit", (e) => { e.preventDefault(); create(); });
  onSend($("newtext"), () => create());
  $("newdiscard").addEventListener("click", async () => {
    const typed = $("newtitle").value.trim() || $("newtext").value.trim() || files.length;
    if (typed && !await askConfirm("Discard this task? What you typed and its files go.", "Discard")) return;
    end();
    box.close();
  });
  $("newclose").addEventListener("click", () => { keep(); box.close(); });
  box.addEventListener("cancel", keep);
  box.querySelector("form").addEventListener("input", keep);
  $("newproj").addEventListener("change", () => { if (unchosen && $("newproj").value) { unchosen = false; error(null); } showAgents(); });
  $("newagent").addEventListener("change", () => { chose = $("newagent").value; });
  $("newadd").addEventListener("click", () => $("newpick").click());
  $("newpick").addEventListener("change", (e) => { add([...e.target.files]); e.target.value = ""; });
  onFiles($("newtext"), box, add, (on) => $("newtext").classList.toggle("dropping", on));

  /** The pickers follow the projects and sessions while the window is open; a chosen one gone is said. */
  function refresh() {
    if (!box.open) return;
    const meta = getMeta(), projects = projectOptions(meta, $("newproj").value), sessions = sessionOptions(meta), now = JSON.stringify([projects, sessions]);
    showAgents();
    if (now === offered) return;
    offered = now;
    const project = $("newproj").value, session = $("newsession").value;
    fillSelect($("newproj"), projects, project);
    fillSelect($("newsession"), sessions, session);
    if ($("newproj").value !== project) error("The project you chose is gone: choose another.");
    else if ($("newsession").value !== session) error("The session you chose is gone: choose another.");
    keep();
  }

  return { open, refresh, add };
}
