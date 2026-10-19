// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Sent history (AC-90): what the user sent in the shown pane's project (its host and folder),
// kept by terminumd. On Enter the page tells terminumd what stood at the cursor: a coding
// agent's input box (its hooks, when they speak, give the prompt exactly and this is dropped),
// or a shell's command line after its prompt; nothing in other programs (an editor, less). The
// button under the split opens the last one as Markdown (a command as a shell block), ↑ older,
// ↓ newer, Copy.
import { typedEnd } from "./cursor.js";
import { fabPane } from "./fabpane.js";
import { when } from "./marks.js";
import { agentName } from "./nav.js";
import { markdownInto } from "./widgetui.js";

const $ = (id) => document.getElementById(id);
// where a shell's prompt ends: "$ ", "% ", "# ", "❯ ", "› ", "> ", "» " after a space or the line's start
const PROMPT = /(^|\s)[$%#❯›>»] /;
const BOX_PROMPT = /^[❯>›!]\s?/;     // an agent's input box: its first row's prompt
const WRAPS = 10;                     // the most rows one command line wraps over

/** The text sent by Enter: in an agent's input box {top, bottom} the typed text of its rows
 *  (the prompt and the next rows' two columns dropped); else, at a shell prompt (`shell`), the
 *  cursor's line with the rows it wrapped from, after the prompt; "" otherwise. Pure: rows
 *  {txt, hard (a line ends there), cur (the cursor's row), end (the column after its typed text)}. */
export function sentText(rows, box, shell) {
  if (box) {
    const lines = rows.slice(Math.max(0, box.top + 1), box.bottom).map((r, i) => {
      const txt = r.end == null ? r.txt : [...r.txt].slice(0, r.end).join("");
      return i === 0 && box.top >= -1 ? txt.replace(BOX_PROMPT, "") : txt.replace(/^ {2}/, "");
    });
    return lines.join("\n").trim();
  }
  const cur = rows.findIndex((r) => r.cur);
  if (!shell || cur < 0) return "";
  let first = cur;
  while (first > 0 && cur - first < WRAPS && !rows[first - 1].hard) first--;
  const line = rows.slice(first, cur + 1).map((r) => r.txt).join("");
  const m = PROMPT.exec(line);
  // only its end trimmed: a command after a space is one the shell keeps out of its history, and so does terminumd
  return (m ? line.slice(m.index + m[0].length) : line.trimStart()).trimEnd();
}

/** The sent history's button and pane. view: term.js's view; item(): the shown pane's row. */
export function sentHistory({ view, send, item, copy, toast }) {
  let n = 0, last = null;
  const pane = fabPane({ fab: $("sentfab"), pane: $("sentpane"), close: $("spclose"), onToggle: (on) => { if (on) ask(0); } });

  function rows() {
    return view.screen.filter(Boolean).map((r) => ({ txt: r.txt, hard: !!r.data?.e, cur: !!r.el.querySelector(".cur"), end: typedEnd(r.orig ?? r.data) }));
  }

  async function draw(m) {
    last = m;
    const body = $("spbody"), it = m.item;
    $("sppos").textContent = m.error ? "" : m.total ? `${m.n + 1} of ${m.total}` : "";
    $("spolder").disabled = !!m.error || m.n + 1 >= m.total;
    $("spnewer").disabled = !!m.error || m.n === 0;
    $("spcopy").disabled = !it;
    // cut at its start when long (right to left), the marks keeping its slashes in place
    $("spwhere").textContent = m.path ? `\u200e${m.host && m.host !== "local" ? `${m.host}: ${m.path}` : m.path}\u200e` : "";
    $("spmeta").textContent = it ? [agentName(it.agent) ?? "Shell", it.title, when(it.at)].filter(Boolean).join(" · ") : "";
    if (m.error || !it) { body.className = "quiet"; body.textContent = m.error ?? "Nothing sent in this folder yet."; pane.place(); return; }
    body.className = "markdown";
    const fence = "`".repeat(Math.max(3, ...[...it.text.matchAll(/`+/g)].map((x) => x[0].length + 1)));   // longer than any run in it
    const md = it.agent ? it.text : `${fence}sh\n${it.text}\n${fence}`;
    await markdownInto(body, md);
    pane.place();
  }

  function ask(k) { n = Math.max(0, k); send({ t: "sentget", n }); }

  $("spolder").onclick = () => ask(n + 1);
  $("spnewer").onclick = () => ask(n - 1);
  $("spcopy").onclick = () => { if (last?.item) { copy(last.item.text); toast("Copied"); } };
  $("sentpane").addEventListener("keydown", (e) => {
    if (e.key === "ArrowUp" && !$("spolder").disabled) { e.preventDefault(); ask(n + 1); }
    else if (e.key === "ArrowDown" && !$("spnewer").disabled) { e.preventDefault(); ask(n - 1); }
  });

  return {
    /** Enter goes to the shown pane now: what it sends. */
    enter() {
      const it = item();
      if (!it || !(it.agent || it.shell)) return;                    // an editor, less: nothing is sent
      const text = sentText(rows(), it.agent ? view.box : null, it.shell);
      if (text) send({ t: "sent", text, agent: it.agent ?? "" });
    },
    onMessage(m) {
      // the answer to the last ask for the pane shown; an older one (a quick ↑, another pane) is dropped
      if (m.t === "sentone" && pane.open() && (m.error || (m.n === n && m.sid === item()?.id))) void draw(m);
      else if (m.t === "sentnew" && pane.open() && last && m.host === last.host && m.path === last.path && n === 0) ask(0);
    },
    /** Another pane is shown: its project's history, when open. */
    shown() { if (pane.open()) ask(0); },
    place: pane.place,
  };
}
