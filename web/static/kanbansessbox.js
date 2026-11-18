// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// + Session on an agent's card, with a first prompt (terminum-dispatcher.md AC-204): a dock under the pool, not a
// dialog, so the board's cards stay draggable onto it. Its box is a plain text field (copy, paste, undo are the
// browser's); a pasted image or a dropped or added file is kept by terminumd, a chip each, its path sent with the
// prompt (this Mac's sessions only); a task card dropped on it puts its ID at the caret, for the agent to read by
// get_task. Start opens the session on that agent and terminumd types the prompt once; with no text nothing is
// typed (AC-142). Close and Esc keep the draft (its text, in this browser); Discard clears it.
import { askConfirm } from "./dialogs.js";
import { el, iconButton, read, write } from "./listparts.js";
import { sessionUrl } from "./tasklink.js";
import { $, agentColor, api, onFiles, onSend, showError, upload } from "./kanbanutil.js";

const DRAFT = "tm.sessiondraft";

/** `getMeta()`: projects and agents, `most` bytes a file may have among them; `dragging()`: the tasks of a drag. */
export function sessionComposer({ getMeta, dragging }) {
  const dock = $("sesscompose"), area = $("sesstext"), list = $("sessfiles"), error = $("sesserror"), start = $("sessstart"), discard = $("sessdiscard");
  let agent = null, key = null, files = [], uploading = 0, busy = false;
  area.value = read(DRAFT)?.text ?? "";

  const say = (text) => showError(error, text);
  const keep = () => write(DRAFT, area.value.trim() ? { text: area.value } : null);
  /** Start waits for files on their way and for its own answer; Discard shows while there is something to clear. */
  function state() {
    start.disabled = busy || uploading > 0 || !agent;
    dock.setAttribute("aria-busy", String(busy || uploading > 0));
    discard.hidden = !area.value.trim() && !files.length;
  }
  function showFiles() {
    list.replaceChildren(...files.map((f) => {
      const li = el("li", null, f.name);
      li.append(iconButton("icon", "x", `Remove ${f.name}`, "Remove this file", () => { files = files.filter((o) => o !== f); showFiles(); }));
      return li;
    }));
    state();
  }

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

  function open(a, from) {
    if (key && key !== from) key.setAttribute("aria-expanded", "false");
    agent = a;
    key = from;
    key?.setAttribute("aria-expanded", "true");
    $("sessagent").replaceChildren(el("span", "sessdot"), el("span", null, a.name));
    dock.style.setProperty("--ag", agentColor(a));
    dock.hidden = false;
    say("");
    state();
    area.focus();
  }
  function close() {
    dock.hidden = true;
    key?.setAttribute("aria-expanded", "false");
    key?.focus({ preventScroll: true });
    key = null;
  }

  async function go() {
    if (start.disabled) return;
    busy = true;
    state();
    say("");
    try {
      const text = area.value.trim();
      const r = await api(`/api/task-projects/${encodeURIComponent(agent.project)}/session`, { method: "POST", body: { agent: agent.id, ...(text ? { prompt: text } : {}), files: files.map((f) => f.id) } });
      if (r.typed === false) {                     // the session stands, the human's: the draft stays to paste there
        say(`The session started, but its prompt was not typed: ${r.why}. Paste it there yourself.`);
        error.append(" ", Object.assign(el("a", null, "Open the session"), { href: sessionUrl(r.key) }));
        return;
      }
      area.value = "";
      files = [];
      keep();
      location.href = sessionUrl(r.key);
    } catch (e) { say(e.message); } finally { busy = false; state(); }
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
  $("sessadd").addEventListener("click", () => $("sessfile").click());
  $("sessfile").addEventListener("change", (e) => { const chosen = [...e.target.files]; e.target.value = ""; if (chosen.length) add(chosen); });
  discard.addEventListener("click", async () => {
    if (area.value.trim() && !await askConfirm("Discard what you typed?", "Discard")) return;
    area.value = "";
    files = [];
    keep();
    showFiles();
    area.focus();
  });

  return {
    open,
    /** The pool drew anew: the key of the agent it is open for is lit again, an agent gone says so. */
    drawn(keys) {
      if (dock.hidden || !agent) return;
      key = keys.get(agent.id) ?? null;
      key?.setAttribute("aria-expanded", "true");
      if (!getMeta().agents.some((a) => a.id === agent.id)) { agent = null; say("Its agent is gone: choose another's + Session."); state(); }
    },
  };
}
