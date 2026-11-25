// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A task its agent asked on, answered from its window (terminum-dispatcher.md AC-285): Answer, Continue with
// Recommended and Archive, whoever started its session and whether it lives. terminumd decides where an answer goes:
// typed into a live session the human opened, else on as RUN, its window opening only for what is left to choose.
import { askConfirm } from "./dialogs.js";
import { api, button, sayRefused } from "./kanbanutil.js";

const ARCHIVE = (id) => `Archive ${id}? It fails as not wanted and leaves the board; tasks blocked by it ask a human.`;

/** The keys of `t` waiting on the question `t.asked`, for its comment field `n`: Archive and Continue with
 *  Recommended to stand at the left; `bind(run)` makes the window's Run key Answer, `wait(run, waiting)` sets the
 *  keys off while the field is empty or the window busy. `act(n, work)` holds the window while work runs
 *  (kanbanwin.js commentAction); `runs`: RUN's window; the window closes once the answer went or waits. */
export function answerKeys(t, n, { act, runs, tryClose, refresh }) {
  const question = t.asked;
  /** One press: Archive asks first; Answer and Archive send what is written, Recommended leaves it in the field. */
  const press = (choice) => act(n, async (current) => {
    if (choice === "archive" && !await askConfirm(ARCHIVE(t.id), "Archive")) return;
    const sends = choice !== "recommended", files = sends ? n.dropped() : [];
    try {
      const got = await api(`/api/tasks/${t.id}/answer`, { method: "POST", body: { choice, question, text: sends ? n.area.value : "", files } });
      if (got.run && await runs.open([t], { words: got.comment, files, current, prefer: t.runs_on, atOnce: true, answer: { choice, question } }) !== true) return;
    } catch (e) {
      await sayRefused(e, n.prune);
      return refresh();
    }
    if (sends) n.sent();
    if (current()) await tryClose();
  });

  const archive = button(null, "Archive", () => press("archive"));
  archive.title = "Not wanted: it fails and leaves the board, with what is written here as your comment";
  const recommended = button(null, "Continue with Recommended", () => press("recommended"));
  recommended.title = "Go on with the answers the agent recommends; what is written here stays unsent";
  return {
    archive, recommended,
    bind(run) {
      run.textContent = "Answer";
      run.title = "Send your answer to its agent; the window closes";
      run.onclick = () => press("answer");
    },
    wait(run, waiting) {
      run.disabled = waiting || !n.area.value.trim();
      recommended.disabled = waiting;
    },
  };
}
