// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The keys' help (terminum.md AC-281): the window on top's keys, every window's, then the page's own, each named for
// this keyboard (⌘ on a Mac, Ctrl elsewhere; the page's own keys take ⌘ on a Mac and Meta elsewhere, as they are
// read). Opened by ⌘/ (Ctrl+/), the session page's Settings menu or the board's key; Esc or Close closes it.
import { el } from "./listparts.js";
import { keyName } from "./keyletters.js";

const MARKUP = `<form method="dialog">
  <div class="dhead"><h2 id="keyhelptitle">Keyboard shortcuts</h2><button class="icon x" type="button" title="Close (Esc)" aria-label="Close"><svg class="lu" viewBox="0 0 24 24" aria-hidden="true" style="--s:16px"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg></button></div>
  <div id="keyhelpbody" data-grow></div>
  <div class="actions"><button value="cancel" type="submit" class="primary">Close</button></div>
</form>`;

/** The page's own keys, as [keys, what], by the page shown. */
function pageKeys(mac) {
  const m = mac ? "⌘" : "Meta+", ms = mac ? "⌘⇧" : "Meta+Shift+";
  if (document.getElementById("term")) {
    return [[`${m}[ ${m}]`, "The session shown before, after"], [`${m}F`, "Find in the terminal's history"], [`${m}B`, "Files open or closed"],
      [`${m}D`, "Quick terminal at the right"], [`${ms}D`, "Quick terminal at the bottom"],
      [`${m}Z ${ms}Z`, "Undo, redo in the terminal's input"], ["/", "Find a session"]];
  }
  if (location.pathname.startsWith("/kanban")) {
    return [["Enter", "Open the card in focus"], ["Esc", "Let go of the cards picked, else fold the cards opened"],
      ["↑ ↓", "Through the Inbox's reports"], ["U", "A report read or unread"], ["⌫", "Delete the report"]];
  }
  return [];
}

function table(caption, rows) {
  const t = el("table", "keyhelp");
  t.append(el("caption", null, caption));
  for (const [keys, what] of rows) {
    const tr = el("tr");
    const k = el("td", "keys");
    k.append(keys ? el("kbd", null, keys) : el("span", "nokey", "no key"));
    tr.append(k, el("td", null, what));
    t.append(tr);
  }
  return t;
}

/** Shows the help: `title` and `keys` ([key named, label]) of the window it opened over, if any. */
export function openHelp({ title, keys, mac }) {
  let box = document.getElementById("keyhelp");
  if (!box) {
    box = el("dialog", "box dlg");
    box.id = "keyhelp";
    box.dataset.size = "list";
    box.setAttribute("aria-labelledby", "keyhelptitle");
    box.innerHTML = MARKUP;
    document.body.append(box);
  }
  const every = [["Esc", "Close the window, or Cancel"], [keyName("Enter", mac, false), "In a text field: its Save or Send"],
    [keyName("Enter", mac), "Run (the comment written goes with it)"], [keyName("letter", mac), "The button whose letter is underlined"],
    [keyName("/", mac, false), "This list"]];
  const parts = [];
  if (title) parts.push(table(title, keys.length ? keys : [[null, "No button with a key"]]));
  parts.push(table("In every window", every));
  const own = pageKeys(mac);
  if (own.length) parts.push(table("On this page", own));
  document.getElementById("keyhelpbody").replaceChildren(...parts);
  box.showModal();
}
