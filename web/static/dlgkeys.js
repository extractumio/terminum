// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A window's buttons by the keyboard (terminum.md AC-281). In the window on top (the last modal dialog opened, else
// a card holding the focus, `[data-keyscope]`: + Session, a report), each button of its footer (its own `.actions`,
// a phone's bar, any `[data-keyrow]`) presses with ⌘⇧ and a letter of its label (Ctrl+Shift off a Mac), the letter underlined;
// `data-key` gives a button its own: a letter, or "Enter" (Run: ⌘⇧↩). Cancel stays Esc. Taken before the page's
// other keys (Quick's ⌘⇧D) and the browser's; a disabled button's key, or a busy window's, does nothing. With no such
// window every key goes where it went, the terminal's too. ⌘/ (Ctrl+/): the keys' help (keyhelp.js).
import { FREE, ariaKey, keyName, letters } from "./keyletters.js";
import { openHelp } from "./keyhelp.js";

const MAC = /Mac|iPhone|iPad/.test(navigator.platform);
const opened = [];                               // the modal dialogs in the order they opened, the last on top
// a window's footer: its own row of keys (a phone's bar at its top holds them, phonewin.js), or a card's marked row
const ROWS = ":scope > form > .actions, :scope > .actions, .dhead > .barkeys, [data-keyrow]";
let marked = new Set(), watched = null, queued = false;

/** The window whose keys the keyboard presses now, or null. */
function topWindow() {
  const d = opened.filter((o) => o.open && o.matches(":modal")).at(-1);
  if (d) return d;
  const card = document.activeElement?.closest?.("[data-keyscope]");
  return card && !card.closest("[hidden]") ? card : null;
}

const label = (b) => b.textContent.replace(/\s+/g, " ").trim();
const own = (scope, e) => e.closest("dialog, [data-keyscope]") === scope;
/** `scope`'s footer buttons as {button, key, at, label}: `key` a letter, "Enter" or null; `at` the place of its letter. */
function keysOf(scope) {
  const rows = [...scope.querySelectorAll(ROWS)].filter((r) => own(scope, r));
  const buttons = rows.flatMap((r) => [...r.querySelectorAll("button")]).map((b) => [b, label(b)])
    .filter(([b, words]) => words && words !== "Cancel" && own(scope, b) && !b.matches(".icon, [value=cancel]") && b.getClientRects().length);
  const auto = letters(buttons.map(([b]) => (b.dataset.key ? null : b.textContent)), buttons.map(([b]) => b.dataset.key).filter((k) => k?.length === 1));
  return buttons.map(([button, words], i) => {
    const mine = button.dataset.key;
    const { letter = null, at = -1 } = mine ? { letter: mine, at: button.textContent.toUpperCase().indexOf(mine) } : auto[i] ?? {};
    return { button, key: letter, at, label: words };
  });
}

/** The text node and offset of `b`'s `at`-th character. */
function place(b, at) {
  const walk = document.createTreeWalker(b, NodeFilter.SHOW_TEXT);
  for (let n = walk.nextNode(), seen = 0; n; seen += n.length, n = walk.nextNode()) if (at < seen + n.length) return [n, at - seen];
  return [null, 0];
}
/** Sets `name` to `value`, or takes it off for null; nothing written when it stands so. */
function attr(b, name, value) {
  if (b.getAttribute(name) !== value) value === null ? b.removeAttribute(name) : b.setAttribute(name, value);
}
function unmark(b) {
  for (const w of b.querySelectorAll("span.dkeyw")) { const p = w.parentNode; w.replaceWith(w.textContent); p.normalize(); }
  for (const name of ["aria-keyshortcuts", "data-keyhint", "data-keymark"]) attr(b, name, null);
}
/** `b`'s key underlined (a letter) or said after its label (↩), and named for assistive tech; nothing written when so
 *  (its own writes are its window's changes: once drawn, it stands). */
function mark({ button: b, key, at }) {
  const letter = key && key !== "Enter" && at >= 0, want = key ? `${key}:${at}` : null;
  if (b.dataset.keymark === want && (!letter || b.querySelector("u.dkey"))) return;
  unmark(b);
  const [n, off] = letter ? place(b, at) : [null, 0];
  if (n) {
    // the word stays one piece of text: a flex button would set its parts apart, on screen and to a screen reader
    const word = document.createElement("span"), u = document.createElement("u");
    word.className = "dkeyw";
    u.className = "dkey";
    u.textContent = n.data[off];
    word.append(n.data.slice(0, off), u, n.data.slice(off + 1));
    n.replaceWith(word);
  }
  attr(b, "aria-keyshortcuts", key ? ariaKey(key, MAC) : null);
  attr(b, "data-keyhint", key === "Enter" ? keyName("Enter", MAC) : null);
  attr(b, "data-keymark", want);
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
  watcher.takeRecords();                         // its own marks: drawn already
}
const soon = () => { if (!queued) { queued = true; requestAnimationFrame(paint); } };
/** Whether a change in the window can touch its footer: a row's own, a row coming or going, a part shown or hidden. */
const ANY_ROW = ".actions, .barkeys, [data-keyrow]";      // ROWS without its place, for any node in the window
const footer = (n) => n.nodeType === 1 && (n.closest(ANY_ROW) || n.querySelector(ANY_ROW));
const watcher = new MutationObserver((records) => {
  if (records.some((r) => r.type === "attributes" ? r.attributeName !== "class" || footer(r.target)
    : footer(r.target.parentElement ?? r.target) || [...r.addedNodes, ...r.removedNodes].some(footer))) soon();
});

new MutationObserver((records) => {
  for (const { target: d } of records) {
    if (d.localName !== "dialog") { if (d.matches("[data-keyscope]")) soon(); continue; }   // within the window: its watcher's
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
  if (e.repeat) return;
  const hit = keysOf(scope).find((k) => k.key === key)?.button;
  if (!hit || hit.disabled || scope.dataset.busy) return;
  hit.click();
}, true);
