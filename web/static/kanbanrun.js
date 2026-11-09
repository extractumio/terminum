// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// RUN's window (terminum-dispatcher.md AC-121, AC-122, AC-138): which of the pool's agents starts the tasks and
// with what message; one at its limit is chosen all the same, and Run waits for its room (AC-157); where is the
// project's (AC-140), said here; the agent dropped on, or the last chosen, already picked: one press. Several tasks
// go by their numbers; what each says shows here first. Dropped on another agent, a conversation goes on there.
import { $, THIS_MAC, api, fillAgents, onSend, showError } from "./kanbanutil.js";
import { taskBoxes } from "./taskboxes.js";

/** `addAgent()` opens the agents' window; `noFolder(project, words)`: terminumd says nothing can start there; `done(waits)`
 *  after a run started, or with what it waits for. */
export function runWindow({ addAgent, noFolder, done }) {
  taskBoxes();
  const box = $("runbox");
  let tasks = [], offer = null, attached = [], fresh = false, dropped = null, answer = () => {};

  const say = (text) => showError($("runerror"), text);
  const chosen = () => offer.agents.find((a) => a.id === $("runagent").value);
  /** At a limit: chosen, it waits for its room (AC-157). */
  const busy = (a) => (a.full ? "at its limit: Run waits" : "");
  let said = "";                                   // what the window says whatever agent is chosen

  /** The chosen agent's, else the window's own words; at its limit with an idle holder, one press closes it and runs
   *  (AC-141), else Run waits for its room (AC-157). */
  function showAgent() {
    const full = chosen()?.full, close = full?.close;
    const words = said || (full ? (close ? full.words : `${full.words} Run waits until it has room, then starts.`) : "");
    $("runnone").textContent = words;
    $("runnone").hidden = !words;
    $("rungo").textContent = close ? `Close ${close.tasks}'s session and run` : "Run";
    $("runserver").textContent = offer.host ?? THIS_MAC;
    $("runfolder").textContent = offer.dir;
    $("runcommand").textContent = chosen()?.line ?? "";
  }

  function draw(scratch) {
    fresh = scratch;
    const first = fresh || offer.first, none = first && !offer.agents.length, moved = !first && offer.moved;
    const names = tasks.map((t) => t.id).join(", "), on = offer.agents.find((a) => a.id === dropped);
    $("runhead").textContent = moved ? `Continue ${names} on ${moved.to}` : !first ? `Run ${names} again` : on ? `Run ${names} on ${on.name}` : `Run ${names}`;
    const words = none ? "No agents yet." : moved ? `Its conversation ${moved.conversation} with ${moved.from} goes on on ${moved.to}, with what you write. Run from scratch starts anew.`
      : first ? "" : "It goes on in its session, with what you write.";
    said = words;
    // the agent it was dropped on is said by the title; what a person checks only sometimes stays folded
    $("runagentrow").hidden = !first || none || !!on;
    $("runhow").hidden = !first || none;
    $("runwhat").hidden = !first;
    $("runwhat").open = offer.by_agent || tasks.length > 1;   // several, or an agent's words: read before they go (AC-121)
    $("runmarked").hidden = !offer.by_agent;
    $("runadd").hidden = !none;
    $("runfresh").hidden = first;
    $("rungo").disabled = none;
    if (first) {
      fillAgents($("runagent"), offer.agents, () => "", [dropped, offer.last]);
      for (const o of $("runagent").options) { const a = offer.agents.find((x) => x.id === o.value); if (a && busy(a)) o.textContent = `${a.name} (${busy(a)})`; }
      $("runprompt").textContent = offer.prompt;
    }
    showAgent();
  }

  const others = () => tasks.slice(1).map((t) => t.id);
  const read = async () => {
    const q = new URLSearchParams({ also: others().join(","), ...(dropped ? { agent: dropped } : {}) });
    offer = await api(`/api/tasks/${tasks[0].id}/run?${q}`);
  };

  /** Opens it for `list`, on `agent` if chosen: true when a run started, else false; a refusal throws terminumd's
   *  words. `words`, `files`: the comment that goes with it; `why`: what a press already found wrong. */
  async function open(list, { words = "", files = [], agent = null, why = "" } = {}) {
    tasks = list;
    dropped = agent;
    attached = files;
    $("runmessage").value = words;
    await read();
    if (offer.no_folder) {
      noFolder(offer.project, offer.no_folder);
      return false;
    }
    draw(false);
    say(why);
    box.returnValue = "";
    box.showModal();
    $("runmessage").focus();
    return new Promise((resolve) => { answer = resolve; });
  }

  async function go() {
    say("");
    $("rungo").disabled = true;
    try {
      const first = fresh || offer.first;
      // going on, it is its session's agent, or the one it was dropped on (AC-138)
      const started = await api(`/api/tasks/${tasks[0].id}/run`, { method: "POST", body: { agent: first ? $("runagent").value : dropped, close: first ? chosen()?.full?.close?.run : undefined, comment: $("runmessage").value, files: attached, fresh, also: others(), prompt_rev: offer.prompt_rev } });
      box.close("run");
      done(started?.waits);
    } catch (e) { say(e.message); }
    $("rungo").disabled = false;
  }

  $("rungo").addEventListener("click", go);
  onSend($("runmessage"), go);
  $("runagent").addEventListener("change", showAgent);
  $("runfresh").addEventListener("click", () => draw(true));
  $("runadd").addEventListener("click", async () => {
    await addAgent();
    await read();
    draw(true);
  });
  box.addEventListener("close", () => answer(box.returnValue === "run"));
  return { open };
}
