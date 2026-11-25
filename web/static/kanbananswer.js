// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A task its agent asked on, answered from its window (terminum-dispatcher.md AC-285): Answer, Continue with
// Recommended and Archive, whoever started its session and whether it lives. terminumd decides where an answer goes:
// typed into a live session the human opened, else on as RUN, its window opening only for what is left to choose.
import { askConfirm } from "./dialogs.js";
import { api, button, say, FILE_GONE } from "./kanbanutil.js";

/** The keys of `t` waiting on the question `t.asked`, for its comment field `n`: { archive, recommended, answer }.
 *  `act(n, work)` holds the window while work runs (kanbanwin.js commentAction); `runs`: RUN's window; `tryClose`:
 *  the window closes once the answer went or waits. */
export function answerKeys(t, n, { act, runs, tryClose, refresh }) {
  const question = t.asked;
  /** A refusal said where the press was; a file let go meanwhile is named and the rest kept. */
  const refused = async (e) => { if (e.message !== FILE_GONE || !await n.prune()) say(e.message); };

  const answer = (choice) => act(n, async (current) => {
    const written = choice === "answer";
    const files = written ? n.dropped() : [];
    let got;
    try {
      got = await api(`/api/tasks/${t.id}/answer`, { method: "POST", body: { choice, question, text: written ? n.area.value : "", files } });
    } catch (e) { return refused(e); }
    if (got.run) {
      try {
        const went = await runs.open([t], { words: got.comment, files, current, prefer: t.runs_on, atOnce: true, answer: { choice, question } });
        if (went !== true) return;
      } catch (e) { return refused(e); }
    }
    if (written) n.sent();                         // Recommended leaves what is written in the field
    if (current()) await tryClose();
  });

  const archive = () => act(n, async (current) => {
    if (!await askConfirm(`Archive ${t.id}? It fails as not wanted and leaves the board; tasks blocked by it ask a human.`, "Archive")) return;
    try {
      await api(`/api/tasks/${t.id}/answer`, { method: "POST", body: { choice: "archive", question, text: n.area.value, files: n.dropped() } });
    } catch (e) { await refused(e); return refresh(); }
    n.sent();                                      // what was written went with it, as the human's comment
    if (current()) await tryClose();
  });

  const recommended = button(null, "Continue with Recommended", () => answer("recommended"));
  recommended.title = "Go on with the answers the agent recommends; what is written here stays unsent";
  const shelve = button(null, "Archive", archive);
  shelve.title = "Not wanted: it fails and leaves the board, with what is written here as your comment";
  return { archive: shelve, recommended, answer: () => answer("answer") };
}
