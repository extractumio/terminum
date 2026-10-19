// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// ⌘Z and ⌘⇧Z on the terminal's input (AC-03): the shown program's own undo and redo keys, as
// measured: Ctrl+_ undoes in Claude Code, zsh, bash and fish; of them only fish redoes by
// default (Alt+/); Codex has neither. Another program gets nothing (Ctrl+_ is nano's Go To
// Line): where there is none the page says so and sends nothing.

import { agentName } from "./nav.js";
import { longPress } from "./rowmenu.js";

const UNDOES = ["Claude Code", "zsh", "bash", "fish"];

/** The program a row runs: Claude Code, Codex, else its process ("zsh"). */
function program(item) {
  if (agentName(item.agent)) return agentName(item.agent);
  return (item.sub ?? []).find((p) => p !== "quick shell" && /^[\w.+-]+$/.test(p)) ?? "This program";
}

/** {bytes} to send for undo (or redo) in `item`'s pane, else {why} there is none. */
export function undoKey(item, redo) {
  if (!item) return { why: "No pane is shown" };
  const name = program(item);
  if (!UNDOES.includes(name)) return { why: `${name} has no ${redo ? "redo" : "undo"} terminum knows` };
  if (!redo) return { bytes: "\x1f" };
  if (name === "fish") return { bytes: "\x1b/" };
  return { why: `${name} has no redo` };
}

/** On a touch screen: a finger resting on the keyboard `button` offers Undo and Redo (`menu`'s
 *  show), and iOS's own undo on the input `field` (three fingers, a shake) does the same. */
export function undoTouch({ button, field, menu, undo }) {
  longPress(button, () => button, () => {
    const r = button.getBoundingClientRect();
    menu.show([["Undo", () => undo(false)], ["Redo", () => undo(true)]], r.right - 210, r.top - 110);
  });
  field.addEventListener("beforeinput", (e) => {
    if (e.inputType !== "historyUndo" && e.inputType !== "historyRedo") return;
    e.preventDefault();
    undo(e.inputType === "historyRedo");
  });
}
