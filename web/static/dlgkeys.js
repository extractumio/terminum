// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A window's buttons by the keyboard (terminum.md AC-281). In the window on top (the last modal dialog opened, else
// a card holding the focus, `[data-keyscope]`: + Session, a report), each button of its footer (its own `.actions`,
// a phone's bar, any `[data-keyrow]`) presses with ⌘⇧ and a letter of its label (Ctrl+Shift off a Mac), the letter underlined;
// `data-key` gives a button its own: a letter, or "Enter" (Run: ⌘⇧↩). Cancel stays Esc. Taken before the page's
// other keys (Quick's ⌘⇧D) and the browser's; a disabled button's key, or a busy window's, does nothing. With no such
// window every key goes where it went, the terminal's too. ⌘/ (Ctrl+/): the keys' help (keyhelp.js).
import { FREE, ariaKey, keyName, letters } from "./keyletters.js";
import { openHelp } from "./keyhelp.js";

export const MAC = /Mac|iPhone|iPad/.test(navigator.platform);
const opened = [];                               // the modal dialogs in the order they opened, the last on top
// a window's footer: its own row of keys (a phone's bar at its top holds them, phonewin.js), or a card's marked row
const ROWS = ":scope > form > .actions, :scope > .actions, .dhead > .barkeys, [data-keyrow]";
let marked = new Set(), watched = null, queued = false;

const modal = (d) => { try { return d.matches(":modal"); } catch { return true; } };
/** The window whose keys the keyboard presses now, or null. */
export function topWindow() {
  const d = opened.filter((o) => o.open && modal(o)).at(-1);
  if (d) return d;
  const card = document.activeElement?.closest?.("[data-keyscope]");
  return card && !card.closest("[hidden]") ? card : null;
}

const label = (b) => b.textContent.replace(/\s+/g, " ").trim();
const own = (scope, e) => e.closest("dialog, [data-keyscope]") === scope;
/** `scope`'s footer buttons as {button, key, at, label}: `key` a letter, "Enter" or null; `at` the place of its letter. */
export function keysOf(scope) {
  const rows = [...scope.querySelectorAll(ROWS)].filter((r) => own(scope, r));
  const buttons = rows.flatMap((r) => [...r.querySelectorAll("button")]).filter((b) => own(scope, b)
    && b.getClientRects().length && label(b) && !b.matches(".icon, [value=cancel]") && label(b) !== "Cancel");
  const given = buttons.map((b) => b.dataset.key ?? null);
  const auto = letters(buttons.map((b, i) => (given[i] ? null : b.textContent)), given.filter((k) => k && k !== "Enter"));
  return buttons.map((button, i) => {
    const key = given[i] ?? auto[i]?.letter ?? null;
    const at = given[i] ? (given[i].length === 1 ? button.textContent.toUpperCase().indexOf(given[i]) : -1) : auto[i]?.at ?? -1;
    return { button, key, at, label: label(button) };
  });
}

/** The text node and offset of `b`'s `at`-th character. */
function place(b, at) {
  const walk = document.createTreeWalker(b, NodeFilter.SHOW_TEXT);
  for (let n = walk.nextNode(), seen = 0; n; seen += n.length, n = walk.nextNode()) if (at < seen + n.length) return [n, at - seen];
  return [null, 0];
}
function unmark(b) {
  for (const w of b.querySelectorAll("span.dkeyw")) { const p = w.parentNode; w.replaceWith(w.textContent); p.normalize(); }
  b.removeAttribute("aria-keyshortcuts");
  delete b.dataset.keyhint;
}
/** `b`'s key underlined (a letter) or said after its label (↩), and named for assistive tech; nothing written when so. */
function mark({ button: b, key, at }) {
  const u = b.querySelector("u.dkey");
  const right = key && key !== "Enter" ? u && u.textContent.toUpperCase() === key && underlineAt(b, u) === at : !u;
  if (!right) {
    unmark(b);
    const [n, off] = key && key !== "Enter" && at >= 0 ? place(b, at) : [null, 0];
    if (n) {
      // the word stays one piece of text: a flex button would set its parts apart, on screen and to a screen reader
      const word = document.createElement("span"), u = document.createElement("u");
      word.className = "dkeyw";
      u.className = "dkey";
      u.textContent = n.data[off];
      word.append(n.data.slice(0, off), u, n.data.slice(off + 1));
      n.replaceWith(word);
    }
  }
  const aria = key ? ariaKey(key, MAC) : null;
  if (b.getAttribute("aria-keyshortcuts") !== aria) aria ? b.setAttribute("aria-keyshortcuts", aria) : b.removeAttribute("aria-keyshortcuts");
  const hint = key === "Enter" ? keyName("Enter", MAC) : undefined;
  if (b.dataset.keyhint !== hint) hint ? (b.dataset.keyhint = hint) : delete b.dataset.keyhint;
}
/** Where in `b`'s text its underline stands. */
function underlineAt(b, u) {
  let at = 0;
  const walk = document.createTreeWalker(b, NodeFilter.SHOW_TEXT);
  for (let n = walk.nextNode(); n; n = walk.nextNode()) { if (u.contains(n)) return at; at += n.length; }
  return -1;
}

/** The window on top's keys drawn, every other button's taken off; its changes watched while it is on top. */
function paint() {
  queued = false;
  const scope = topWindow();
  if (scope !== watched) {
    watcher.disconnect();
    watched = scope;
    if (scope) watcher.observe(scope, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ["hidden", "disabled", "class", "data-key"] });
  }
  const keys = scope ? keysOf(scope) : [], now = new Set(keys.map((k) => k.button));
  for (const b of marked) if (!now.has(b)) unmark(b);
  for (const k of keys) mark(k);
  marked = now;
}
const soon = () => { if (!queued) { queued = true; requestAnimationFrame(paint); } };
const watcher = new MutationObserver(soon);

new MutationObserver((records) => {
  for (const { target: d } of records) {
    if (d.localName !== "dialog") { soon(); continue; }
    const i = opened.indexOf(d);
    if (i >= 0) opened.splice(i, 1);
    if (d.open) opened.push(d);
    soon();
  }
}).observe(document.documentElement, { attributes: true, attributeFilter: ["open", "hidden"], subtree: true });
opened.push(...document.querySelectorAll("dialog[open]"));
document.addEventListener("focusin", soon);
soon();

/** The keys' help, over the window on top, its keys listed; pressed again, it closes. */
function help() {
  const scope = topWindow();
  if (scope?.id === "keyhelp") return scope.close();
  const title = scope ? (scope.querySelector(".dhead h2, h2, h3")?.textContent.trim() || "This window") : null;
  openHelp({ title, keys: scope ? keysOf(scope).map(({ key, label }) => [key && keyName(key, MAC), label]) : [], mac: MAC });
}
document.addEventListener("keyhelp", help);    // the Settings menu's row (menu.js)
document.getElementById("keyhelpbtn")?.addEventListener("click", help);   // the board's

/** The letter of a key press: its Latin letter (AZERTY's A is A), else where it lies on the keyboard (a Russian layout). */
function letterOf(e) {
  if (/^[a-z]$/i.test(e.key)) return e.key.toUpperCase();
  return /^Key([A-Z])$/.exec(e.code)?.[1] ?? null;
}

addEventListener("keydown", (e) => {
  if (e.isComposing || e.altKey) return;
  const cmd = MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  if (!cmd) return;
  if (!e.shiftKey && (e.key === "/" || e.code === "Slash")) {
    // Ctrl+/ is the terminal's (^_, undo in a shell's line): its input keeps it
    if (!MAC && (e.target.id === "kbd" || e.target.closest?.("#term"))) return;
    e.preventDefault();
    e.stopPropagation();
    if (!e.repeat) help();
    return;
  }
  if (!e.shiftKey) return;
  const scope = topWindow();
  if (!scope) return;
  const key = e.key === "Enter" || e.code === "NumpadEnter" ? "Enter" : letterOf(e);
  if (!key || (key !== "Enter" && !FREE.includes(key))) return;   // ⌘⇧Z stays redo in a field, ⌘⇧T the browser's
  // a free letter or ⌘⇧↩ in a window is that window's: nothing behind it acts on it, even with no button for it
  e.preventDefault();
  e.stopPropagation();
  const hit = keysOf(scope).find((k) => k.key === key)?.button;
  if (!hit || e.repeat || hit.disabled || scope.dataset.busy) return;
  hit.click();
}, true);
