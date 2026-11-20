// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The windows terminumd sends when Claude Code or Codex starts where terminum serves it less well
// (terminum-agents.md AC-208): a Codex under a shared app-server, an agent in its full-screen TUI. The
// page asks for the shown pane's (`notice {id}`) when it shows a coding agent and when its screen or
// TUI changes; terminumd alone decides whether one is due, once a run. A window waits behind one open;
// focus starts on Close (the user is about to type, and a Space must never tick the box). Its "Do not
// show again" turns that kind off for every browser (`notices {kind, on: false}`).
import { el } from "./listparts.js";
import { icon } from "./icons.js";

const $ = (id) => document.getElementById(id);

/** view: the TermView (its screen: alt); sid(): the shown pane; agentOf(id): its coding agent, as the list
 *  names it (the view's may still be the pane shown before); copy(text): the page's Copy. */
export function agentNotices({ view, send, sid, agentOf, copy }) {
  let asked = "";
  let turn = Promise.resolve();

  /** The shown pane's agent, screen or own TUI (`tui`: the header's !) may have changed. */
  function sync(tui) {
    const id = sid(), agent = id && agentOf(id);
    const now = agent ? `${id}|${agent}|${!!view.alt}|${!!tui}` : "";
    if (now === asked) return;
    asked = now;
    if (now) send({ t: "notice", id });
  }

  /** A part of terminumd's words: a paragraph, or a command on its own line with its Copy. */
  function part(p) {
    if (typeof p?.cmd !== "string") return el("p", null, String(p?.text ?? ""));
    const row = el("div", "noticecmd"), b = el("button", "icon");
    b.type = "button";
    b.title = b.ariaLabel = `Copy ${p.cmd}`;
    b.append(icon("copy", 16));
    b.addEventListener("click", () => copy(p.cmd));
    row.append(el("code", "field", p.cmd), b);
    return row;
  }

  function open(m) {
    const box = $("noticebox");
    $("noticetitle").textContent = String(m.title ?? "");
    $("noticeparts").replaceChildren(...(Array.isArray(m.parts) ? m.parts : []).map(part));
    $("noticeoff").checked = false;
    box.returnValue = "";
    box.showModal();
    $("noticeclose").focus();
    return new Promise((resolve) => box.addEventListener("close", () => {
      if ($("noticeoff").checked) send({ t: "notices", kind: m.kind, on: false });
      resolve();
    }, { once: true }));
  }

  /** terminumd's `notice`: shown after the one open, if any. */
  function show(m) {
    turn = turn.then(() => open(m)).catch(() => {});
  }
  return { sync, show };
}
