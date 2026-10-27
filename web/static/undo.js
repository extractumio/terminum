// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// ⌘Z and ⌘⇧Z on the terminal's input (AC-03): the shown program's own undo and redo keys, as
// measured: Ctrl+_ undoes in Claude Code, zsh, bash and fish; of them only fish redoes by
// default (Alt+/); Codex has neither. Another program gets nothing (Ctrl+_ is nano's Go To
// Line): where there is none the page says so and sends nothing.

import { agentName } from "./nav.js";
import { longPress } from "./rowmenu.js";
import { selectedText } from "./seltext.js";

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

/** Whether the clipboard holds something: false only when the browser lets the page read it
 *  unasked (a permission given) and it is empty; Safari never does, so there it may. */
async function mayPaste() {
  try {
    if ((await navigator.permissions.query({ name: "clipboard-read" })).state !== "granted") return true;
    for (const item of await navigator.clipboard.read()) {
      if (item.types.some((t) => t.startsWith("image/"))) return true;
      if (item.types.includes("text/plain") && (await (await item.getType("text/plain")).text())) return true;
    }
    return false;
  } catch { return true; }
}

/** The menu's choices now (undoTouch's can): Cut and Clear for a selection in an agent's input
 *  box (cuttable(range): whether it has keys; cut(range): the keys Backspace over it sends, then
 *  send(keys)), Copy for any in `term`, Undo where the program of item() has it, Redo after the
 *  page's Undo (undone()) where it has one. Cut and Clear delete only the text chosen: when the
 *  box was drawn anew meanwhile, nothing goes and say(text) tells why. */
export function editChoices({ term, item, undone, cuttable, cut, copy, send, say }) {
  return () => {
    const sel = getSelection(), range = sel?.rangeCount && !sel.isCollapsed ? sel.getRangeAt(0).cloneRange() : null;
    const text = range ? selectedText(term) : null, chosen = range ? range.toString() : "";
    const del = (copied) => () => {
      const same = range.startContainer.isConnected && range.endContainer.isConnected && range.toString() === chosen;
      const keys = same ? cut(range) : null;
      if (!keys) return say("The input box changed: select the text again");
      if (copied) copy(text);
      getSelection().removeAllRanges();
      send(keys);
    };
    const box = !!text && cuttable(range), it = item();
    return { cut: box ? del(true) : null, clear: box ? del(false) : null, copy: text ? () => copy(text) : null,
      undo: !!undoKey(it, false).bytes, redo: undone() && !!undoKey(it, true).bytes };
  };
}

/** On a touch screen: a finger resting on the keyboard `button`, or held still on the input and
 *  lifted (Input's holdMenu, the returned function), opens the page's menu (`menu`'s show) with what
 *  applies, in the owner's order: Cut and Clear (a selection in an agent's input box), Paste,
 *  Copy (a selection), Undo, Redo (AC-03). can(): {cut, clear, copy: () => void or null, undo,
 *  redo: booleans}, taken when the menu opens (a tap on it may clear the selection). iOS's own undo
 *  on the input `field` (three fingers, a shake) undoes too. */
export function undoTouch({ button, field, menu, undo, paste, can }) {
  const editMenu = async (x, y) => {
    const c = can(), items = [];
    if (c.cut) items.push(["Cut", c.cut]);
    if (c.clear) items.push(["Clear", c.clear]);
    if (await mayPaste()) items.push(["Paste", paste]);
    if (c.copy) items.push(["Copy", c.copy]);
    if (c.undo) items.push(["Undo", () => undo(false)]);
    if (c.redo) items.push(["Redo", () => undo(true)]);
    if (items.length) menu.show(items, x - 105, y - 60 - items.length * 44, true, document.activeElement === field);   // the keyboard stays up
  };
  /** ⌘X: the menu's Cut, when it applies (idekeys.js); whether it did. */
  editMenu.cut = () => { const c = can(); return !!c.cut && (c.cut(), true); };
  longPress(button, () => button, () => { const r = button.getBoundingClientRect(); editMenu(r.right - 105, r.top); });
  field.addEventListener("beforeinput", (e) => {
    if (e.inputType !== "historyUndo" && e.inputType !== "historyRedo") return;
    e.preventDefault();
    undo(e.inputType === "historyRedo");
  });
  return editMenu;
}
