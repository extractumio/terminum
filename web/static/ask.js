// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Ask about a selection (AC-37): Ask (markpick.js) opens a dialog with the selected text and a
// field for the question; Add to Chat asks terminumd for the pane's next quote number (it
// counts up across devices: `askno`) and pastes the quote, the question and a rule at the
// input's cursor, a blank line after it, without Enter: in a coding agent's box by two Ctrl+J
// (a new line there; the edges of a paste are trimmed or folded into "[Pasted text]"), elsewhere
// (a shell, where Ctrl+J would run the line) inside the paste.

const $ = (id) => document.getElementById(id);
const ANSWER = 10000;          // ms the dialog waits for the number

/** What Add to Chat pastes: the selection quoted, numbered, the question under it. */
export function askText(n, quote, question) {
  return `Quote #${String(n).padStart(4, "0")}:\n - Quote Start ----\n«${quote.replace(/\n+$/, "")}»\n- Quote End ----\n${question.trim()}\n----`;
}

/** What goes to the pane for `text`, in order: [kind, data], kind "keys" (typed) or "paste". */
export function askSends(text, agent) {
  return agent ? [["paste", text], ["keys", "\n\n"]] : [["paste", `${text}\n\n`]];
}

/** send(msg) to terminumd; toChat(text): pasted into the shown pane's input; keys(data): typed
 *  there; agent(): the shown pane runs a coding agent. {open(text), seen(m)}. */
export function askBox({ send, toChat, keys, agent }) {
  const box = $("askbox"), q = $("askq"), add = box.querySelector(".primary");
  let quote = "", waiting = null;
  const fail = (text) => { $("askerr").textContent = text; $("askerr").hidden = !text; };
  q.addEventListener("input", () => { add.disabled = !q.value.trim(); });
  box.addEventListener("close", () => { waiting?.(null); });
  box.querySelector("form").addEventListener("submit", async (e) => {
    if (e.submitter?.value !== "add") return;                       // Cancel closes it
    e.preventDefault();
    if (!q.value.trim() || waiting) return;
    add.disabled = true;
    fail("");
    const m = await new Promise((resolve) => {
      const timer = setTimeout(() => resolve({ error: "The Mac did not answer: is the connection back?" }), ANSWER);
      waiting = (got) => { clearTimeout(timer); waiting = null; resolve(got); };
      send({ t: "askno" });
    });
    add.disabled = !q.value.trim();
    if (!m || !box.open) return;                                   // closed meanwhile
    if (m.error || !Number.isInteger(m.n)) return fail(m.error ?? "No number came back.");
    box.close();
    for (const [kind, data] of askSends(askText(m.n, quote, q.value), agent())) (kind === "keys" ? keys : toChat)(data);
  });
  return {
    open(text) {
      quote = text;
      $("asktext").textContent = text;
      q.value = "";
      add.disabled = true;
      fail("");
      box.showModal();
      q.focus();
    },
    /** Every message from terminumd (app.js dropped another pane's). */
    seen(m) { if (m.t === "askno") waiting?.(m); },
  };
}
