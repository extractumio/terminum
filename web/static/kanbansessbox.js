// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// + Session on an agent's card, with a first prompt (terminum-dispatcher.md AC-204): a dock under the pool, not a
// dialog, so the board's cards stay draggable onto it. Its box is a plain text field (copy, paste, undo are the
// browser's); a pasted image or a dropped or added file is kept by terminumd, a chip each, its path sent with the
// prompt (this Mac's sessions only); a task card dropped on it puts its ID at the caret, for the agent to read by
// get_task. Start opens the session on that agent and terminumd types the prompt once; with no text nothing is
// typed (AC-142). Close and Esc keep the draft (its text, in this browser); Discard clears it. Start folds the dock at once
// and the agent's key says "Starting…" while terminumd starts it, every + Session key inert meanwhile (no other draft can
// be typed then); a failure opens the dock again with the draft, its files and terminumd's words. A start that went in
// keeps the board: a line under the pool says so, with a link to the session.
import { askConfirm } from "./dialogs.js";
import { el, read, write } from "./listparts.js";
import { sessionUrl } from "./tasklink.js";
import { $, agentColor, api, fileChips, onFiles, onSend, showError, upload } from "./kanbanutil.js";

const DRAFT = "tm.sessiondraft";

/** `getMeta()`: projects and agents, `most` bytes a file may have among them; `dragging()`: the tasks of a drag;
 *  `started()`: a session started, for the pool to read its slots again. */
export function sessionComposer({ getMeta, dragging, started }) {
  const dock = $("sesscompose"), area = $("sesstext"), list = $("sessfiles"), error = $("sesserror"), start = $("sessstart"), discard = $("sessdiscard"), done = $("sessdone");
  let agent = null, files = [], uploading = 0, busy = false, left = null;   // left: the key close() gave the focus to
  let missed = null;                               // the text of a prompt whose session started without it: Start again would open another
  area.value = read(DRAFT)?.text ?? "";

  const say = (text) => showError(error, text);
  const keep = () => write(DRAFT, area.value.trim() ? { text: area.value } : null);
  /** Start waits for files on their way and for its own answer; Discard shows while there is something to clear. */
  const gone = () => !getMeta().agents.some((a) => a.id === agent?.id);
  function state() {
    start.disabled = busy || uploading > 0 || gone() || area.value === missed;
    dock.setAttribute("aria-busy", String(busy || uploading > 0));
    discard.hidden = !area.value.trim() && !files.length;
  }
  function showFiles() {
    fileChips(list, files, (f) => { files = files.filter((o) => o !== f); showFiles(); });
    state();
  }
  /** The agents' keys say which one it is open for and which one is starting (the pool draws each by `mark`). */
  const keys = () => document.querySelectorAll("#agentbar .agsess");
  function mark(k) {
    const starting = busy && k.dataset.agent === agent?.id;
    k.setAttribute("aria-expanded", String(!dock.hidden && k.dataset.agent === agent?.id));
    if (busy) k.setAttribute("aria-disabled", "true"); else k.removeAttribute("aria-disabled");
    if (starting) k.setAttribute("aria-busy", "true"); else k.removeAttribute("aria-busy");
    k.textContent = starting ? "Starting…" : "+ Session";
  }
  const lit = () => { for (const k of keys()) mark(k); };

  async function add(chosen) {
    const p = getMeta().projects.find((x) => x.id === agent?.project);
    if (p?.host) return say("Files go only to a session on this Mac: this project's agents run on another host. Paste their text instead.");
    say("");
    for (const file of chosen) {
      uploading++;
      state();
      try { files.push(await upload(file, getMeta().most_bytes ?? Infinity)); showFiles(); } catch (e) { say(e.message); } finally { uploading--; state(); }
    }
  }

  /** Task IDs at the caret, a space before when it follows a word, one after (the box's end when it has no focus). */
  function insertIds(ids) {
    const v = area.value, at = document.activeElement === area ? area.selectionStart : v.length, end = document.activeElement === area ? area.selectionEnd : v.length;
    const words = `${at > 0 && !/\s/.test(v[at - 1]) ? " " : ""}${ids.join(" ")} `;
    area.setRangeText(words, at, end, "end");
    area.focus();
    keep();
    state();
  }

  /** `focus`: the box takes it (a failure coming back leaves it where the user moved it meanwhile). */
  function open(a, focus = true) {
    if (busy) return;                              // a start on its way: its draft comes back here if it fails
    agent = a;
    $("sessagent").replaceChildren(el("span", "sessdot"), el("span", null, a.name));
    dock.style.setProperty("--ag", agentColor(a));
    dock.hidden = false;
    done.hidden = true;
    lit();
    say("");
    state();
    if (focus) area.focus();
  }
  function close() {
    dock.hidden = true;
    lit();
    left = [...keys()].find((k) => k.dataset.agent === agent?.id) ?? null;
    left?.focus({ preventScroll: true });
  }

  /** A start that did not go as asked: the dock again, its draft and files as they were, terminumd's words in it. */
  function back(text) {
    const at = document.activeElement;
    busy = false;
    open(agent, at === document.body || at === left);   // the pool redrawn meanwhile gives the body the focus
    say(text);
  }
  async function go() {
    if (start.disabled) return gone() && say("Its agent is gone: choose another's + Session.");
    busy = true;
    state();
    say("");
    close();
    try {
      const text = area.value.trim();
      const r = await api(`/api/task-projects/${encodeURIComponent(agent.project)}/session`, { method: "POST", body: { agent: agent.id, ...(text ? { prompt: text } : {}), files: files.map((f) => f.id) } });
      if (r.typed === false) {                     // the session stands, the human's: the draft stays to paste there
        missed = area.value;                       // Start waits for a change: a second press would open a second session
        back(`The session started, but its prompt was not typed: ${r.why}. Paste it there yourself.`);
        error.append(" ", Object.assign(el("a", null, "Open the session"), { href: sessionUrl(r.key) }));
        return;
      }
      area.value = "";
      files = [];
      keep();
      showFiles();
      busy = false;
      lit();
      state();
      const link = Object.assign(el("a", null, "Open the session"), { href: sessionUrl(r.key) }), at = document.activeElement;
      done.firstElementChild.replaceChildren(`Session started on ${agent.name} · `, link);
      done.hidden = false;
      if (at === document.body || at === left) link.focus({ preventScroll: true });   // the focus left where the user moved it meanwhile
      started();
    } catch (e) { back(e.message); }
  }

  onFiles(area, area, add, (on) => area.classList.toggle("dropping", on));
  onSend(area, go);
  area.addEventListener("input", () => { keep(); state(); });
  // a task card's drag: its IDs, never the text the browser would drop by itself; it runs nothing, moves nothing
  area.addEventListener("dragover", (e) => { if (!dragging()) return; e.preventDefault(); e.dataTransfer.dropEffect = "move"; area.classList.add("dropping"); });
  area.addEventListener("dragleave", () => { if (dragging()) area.classList.remove("dropping"); });
  area.addEventListener("drop", (e) => {
    const tasks = dragging();
    if (!tasks) return;                            // files: onFiles's
    e.preventDefault();
    e.stopPropagation();
    area.classList.remove("dropping");
    insertIds(tasks.map((t) => t.id));
  });
  dock.addEventListener("submit", (e) => { e.preventDefault(); go(); });
  dock.addEventListener("keydown", (e) => { if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); } });
  $("sessclose").addEventListener("click", close);
  done.querySelector(".x").addEventListener("click", () => { done.hidden = true; });
  $("sessadd").addEventListener("click", () => $("sessfile").click());
  $("sessfile").addEventListener("change", (e) => { const chosen = [...e.target.files]; e.target.value = ""; if (chosen.length) add(chosen); });
  discard.addEventListener("click", async () => {
    if (area.value.trim() && !await askConfirm("Discard what you typed?", "Discard")) return;
    area.value = "";
    files = [];
    missed = null;
    keep();
    showFiles();
    area.focus();
  });

  return {
    open,
    /** An agent's + Session key as it should stand now: expanded, starting, inert. */
    mark,
  };
}
