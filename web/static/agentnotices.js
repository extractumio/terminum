// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The windows terminumd sends when Claude Code or Codex starts where terminum serves it less well
// (terminum-agents.md AC-208): a Codex under a shared app-server, an agent in its full-screen TUI.
// terminumd decides when, once a run; the page shows them, one at a time (AC-165). Focus starts on Close:
// the user is about to type, and a Space must never tick the box. Its "Do not show again" turns that
// kind off for every browser (`notices {kind, on: false}`).
import { el, iconButton } from "./listparts.js";
import { inTurn } from "./dialogs.js";

const $ = (id) => document.getElementById(id);

/** copy(text): the page's Copy. */
export function agentNotices({ send, copy }) {
  /** A part of terminumd's words: a paragraph, or a command on its own line with its Copy. */
  function part(p) {
    if (typeof p?.cmd !== "string") return el("p", null, String(p?.text ?? ""));
    const row = el("div", "noticecmd");
    row.append(el("code", "field", p.cmd), iconButton("icon", "copy", `Copy ${p.cmd}`, `Copy ${p.cmd}`, () => copy(p.cmd), 16));
    return row;
  }

  function open(m) {
    const box = $("noticebox");
    $("noticetitle").textContent = String(m.title ?? "");
    $("noticeparts").replaceChildren(...(Array.isArray(m.parts) ? m.parts : []).map(part));
    $("noticeoff").checked = false;
    box.showModal();
    $("noticeclose").focus();
    return new Promise((resolve) => box.addEventListener("close", () => {
      if ($("noticeoff").checked) send({ t: "notices", kind: m.kind, on: false });
      resolve();
    }, { once: true }));
  }

  /** terminumd's `notice`: shown after the one open, if any. */
  return { show: (m) => inTurn("noticebox", () => open(m)) };
}
