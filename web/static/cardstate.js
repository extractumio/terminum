// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// What a card's foot says of its session's agent (terminum-kanban.md AC-136): its class, words and title;
// null: nothing to say (it works, or the task is done). Also the one reading of a session's row as at work, for
// the row's bar, the card and the board's dot (terminum-agents.md AC-39).

/** A card's session from its live row in the session list: the fields terminumd's card gives it (tasks/api.rs
 *  `with_session`), so a live update drops none of them. */
export const liveSession = (it) => ({ key: it.id, state: it.state, agent: it.agent, why: it.why, background: !!it.background, closed: false });

/** Whether a session's row reads as at work (terminum-agents.md AC-39): its agent works, or its turn ended while
 *  work it started runs in the background, however long (the row's bar and the board's dot pulse by it). */
export const atWork = (it) => it?.state === "working" || (it?.state === "done" && !!it.background);

/** The card and task window share the queue's words (AC-169), ahead of any old session state. */
export function queuedWords(t) {
  const n = t.wait_position;
  const suffix = n % 100 >= 11 && n % 100 <= 13 ? "th" : ({ 1: "st", 2: "nd", 3: "rd" }[n % 10] ?? "th");
  return `queued for ${t.waits}${n ? `, ${n}${suffix}` : ""}`;
}

/** The words of a task that waits for other tasks (terminum-dispatcher.md AC-210): their IDs, no queue place. */
export const waitsForWords = (t) => `waits for ${t.waits_for.join(", ")}`;

/** What a session in an agent's slot does now (terminum-dispatcher.md AC-138), by terminumd's slot `s` and its live
 *  row: quiet "idle" or "integration", otherwise "starting", "waits" or "working". */
export function slotState(s, row) {
  if (s.integration && row?.state !== "waiting" && !atWork(row)) return "integration";
  if (s.idle && !atWork(row)) return "idle";
  return ["starting", "typed"].includes(s.state) ? "starting" : s.state === "asking" || row?.state === "waiting" ? "waits" : "working";
}

/** The card's state line for task `t`, as {cls, words, why}, or null. */
export function cardState(t) {
  const s = t.session;
  const line = (cls, words, why) => ({ cls, words, why });
  if (t.waits_for?.length) return line("waiting", waitsForWords(t), "Dispatcher goes on with it in its conversation once they are DONE");
  if (t.waits) return line("waiting", queuedWords(t), "Run waits for that agent's room, then starts");
  if (!s && !t.run) return null;
  const open = t.status === "IN PROGRESS";
  if (t.run === "no_result") return line("stopped", "DONE, no result");
  if (t.run) return line("waiting", t.run === "asking" ? "answer in its window" : "starting");
  if (s.closed) return line("closed", "session closed");
  if (s.state === "failed") return line("failed", `failed: ${(s.why || "an error").replaceAll("_", " ")}`);
  if (s.state === "waiting") return line("waiting", "waits for you", s.why);
  if (s.state === "working" || !open) return null;
  // its turn ended while a shell or monitor it started still runs (its checks, a landing): the agent goes on
  // when that ends; one that never ends (a dev server) leaves it so
  if (s.agent && s.background) return line("background", "background work runs",
    "Its turn ended while work it started runs in the background; the agent goes on when that ends");
  return line("stopped", s.agent ? "stopped, task open" : "no agent");
}
