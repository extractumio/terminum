// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Ask about a selection (AC-37): Ask (markpick.js) opens a dialog with the selected text and a
// field for the question; Add to Chat asks terminumd for the pane's next quote number (it
// counts up across devices: `askno`) and pastes the quote, the question and a rule at the
// input's cursor, a blank line after it, without Enter: in a coding agent's box by two Ctrl+J
// (a new line there; the edges of a paste are trimmed or folded into "[Pasted text]"), elsewhere
// (a shell, where Ctrl+J would run the line) inside the paste. Send to Agent (a coding agent's
// pane only) pastes the same and presses Enter.

const $ = (id) => document.getElementById(id);
const ANSWER = 10000;          // ms the dialog waits for the number
const ENTER_AFTER = 200;       // ms: an Enter in the same read as a paste is a new line to Claude Code

/** What Add to Chat pastes: the selection quoted, numbered, the question under it. */
export function askText(n, quote, question) {
  return `Quote #${String(n).padStart(4, "0")}:\n - Quote Start ----\n«${quote.replace(/\n+$/, "")}»\n- Quote End ----\n${question.trim()}\n----`;
}

/** What goes to the pane for `text`, in order: [kind, data], kind "keys" (typed), "paste" or
 *  "enter" (typed a moment after); `now`: sent to the agent (Send to Agent). */
export function askSends(text, agent, now = false) {
  if (now) return [["paste", text], ["enter", "\r"]];
  return agent ? [["paste", text], ["keys", "\n\n"]] : [["paste", `${text}\n\n`]];
}

/** send(msg) to terminumd; toChat(text): pasted into the shown pane's input; keys(data): typed
 *  there; agent(): the shown pane runs a coding agent. {open(text), seen(m)}. */
export function askBox({ send, toChat, keys, agent }) {
  const box = $("askbox"), q = $("askq"), add = box.querySelector(".primary"), now = $("asksend");
  let quote = "", waiting = null;
  const fail = (text) => { $("askerr").textContent = text; $("askerr").hidden = !text; };
  const ready = (on) => { add.disabled = now.disabled = !on; };
  q.addEventListener("input", () => ready(q.value.trim()));
  box.addEventListener("close", () => { waiting?.(null); });
  box.querySelector("form").addEventListener("submit", async (e) => {
    const how = e.submitter?.value;
    if (how !== "add" && how !== "send") return;                    // Cancel closes it
    e.preventDefault();
    if (!q.value.trim() || waiting) return;
    ready(false);
    fail("");
    const m = await new Promise((resolve) => {
      const timer = setTimeout(() => resolve({ error: "The Mac did not answer: is the connection back?" }), ANSWER);
      waiting = (got) => { clearTimeout(timer); waiting = null; resolve(got); };
      send({ t: "askno" });
    });
    ready(q.value.trim());
    if (!m || !box.open) return;                                   // closed meanwhile
    if (m.error || !Number.isInteger(m.n)) return fail(m.error ?? "No number came back.");
    box.close();
    for (const [kind, data] of askSends(askText(m.n, quote, q.value), agent(), how === "send")) {
      if (kind === "enter") setTimeout(() => keys(data), ENTER_AFTER);
      else (kind === "keys" ? keys : toChat)(data);
    }
  });
  return {
    open(text) {
      quote = text;
      $("asktext").textContent = text;
      q.value = "";
      ready(false);
      now.hidden = !agent();                                        // a shell would run the quote
      fail("");
      box.showModal();
      q.focus();
    },
    /** Every message from terminumd (app.js dropped another pane's). */
    seen(m) { if (m.t === "askno") waiting?.(m); },
  };
}
