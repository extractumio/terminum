// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A hot keys layout of the user's (AC-18, ~/.terminum/hotkeys.json, checked by terminumd): rows
// of keys by tmux's names ("F5", "C-r", "S-Enter", "text:git status", "Paste") become the
// panel's [label, action, hint] keys (input.js), the action being the bytes a terminal sends.
import { encodeKey } from "./keys.js";

const ACTIONS = { Copy: "copy", Paste: "paste", Upload: "upload", Fn: "fn", Ctrl: "mod:ctrl", Alt: "mod:alt" };
// tmux's key names -> KeyboardEvent.key
const NAMES = { Enter: "Enter", Tab: "Tab", BTab: "Tab", Escape: "Escape", Esc: "Escape", BSpace: "Backspace",
  Up: "ArrowUp", Down: "ArrowDown", Left: "ArrowLeft", Right: "ArrowRight", Home: "Home", End: "End",
  PPage: "PageUp", PageUp: "PageUp", PgUp: "PageUp", NPage: "PageDown", PageDown: "PageDown", PgDn: "PageDown",
  IC: "Insert", Insert: "Insert", DC: "Delete", Delete: "Delete", Space: " " };
const ARROWS = { ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→" };
const SHOWN = { Escape: "Esc", Backspace: "⌫", PageUp: "PgUp", PageDown: "PgDn", Insert: "Ins", Delete: "Del", " ": "Space", Enter: "Enter", Tab: "Tab" };

/** One key of a layout (a name, or {key, label}) as [label, action, hint], or null when the
 *  page cannot type it. */
export function hotKey(spec) {
  const name = typeof spec === "string" ? spec : spec?.key;
  if (typeof name !== "string" || !name) return null;
  const label = typeof spec === "object" && spec.label ? String(spec.label) : null;
  if (name.startsWith("text:")) return name.length > 5 ? [label ?? name.slice(5), name.slice(5), `Types "${name.slice(5)}"`] : null;
  if (ACTIONS[name]) return [label ?? name, ACTIONS[name]];
  const m = /^((?:[CMS]-)*)(.+)$/.exec(name);
  const mods = { ctrlKey: m[1].includes("C-"), altKey: m[1].includes("M-"), shiftKey: m[1].includes("S-") || m[2] === "BTab" };
  const key = NAMES[m[2]] ?? (/^F([1-9]|1[0-2])$/.test(m[2]) || m[2].length === 1 ? m[2] : null);
  if (!key) return null;
  const code = key.length === 1 && /[a-z]/i.test(key) ? `Key${key.toUpperCase()}` : key.length === 1 && /\d/.test(key) ? `Digit${key}` : "";
  const bytes = key.length === 1 && !mods.ctrlKey && !mods.altKey ? (mods.shiftKey ? key.toUpperCase() : key)
    : encodeKey({ key, code, metaKey: false, ...mods }, false);
  if (!bytes) return null;
  // arrows without a modifier follow the pane's cursor key mode (input.js seqOf)
  const action = ARROWS[key] && !m[1] ? `arrow:${{ ArrowUp: "A", ArrowDown: "B", ArrowRight: "C", ArrowLeft: "D" }[key]}` : bytes;
  const shown = (mods.ctrlKey && key.length === 1 ? "^" + key.toUpperCase()
    : (mods.ctrlKey ? "⌃" : "") + (mods.altKey ? "⌥" : "") + (mods.shiftKey && m[2] !== "BTab" ? "⇧" : "")
      + (m[2] === "BTab" ? "⇧Tab" : key === "Enter" && m[1] ? "↩" : ARROWS[key] ?? SHOWN[key] ?? key));
  return [label ?? shown, action, name];
}

/** A layout's rows as the panel's: {rows} or {error} naming the first key the page cannot type. */
export function hotRows(rows) {
  if (!Array.isArray(rows) || !rows.length) return { error: "hotkeys.json: no rows" };
  const out = [];
  for (const [i, row] of rows.entries()) {
    if (!Array.isArray(row)) return { error: `hotkeys.json: row ${i + 1} is not a list` };
    const keys = [];
    for (const spec of row) {
      const k = hotKey(spec);
      if (!k) return { error: `hotkeys.json: row ${i + 1}: the page cannot type ${JSON.stringify(spec)}` };
      keys.push(k);
    }
    out.push(keys);
  }
  return { rows: out };
}
