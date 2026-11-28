// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// What the session list shares with the board, with nothing of the list behind it.
import { icon } from "./icons.js";
import { textForm } from "./render.js";

/** A JSON value kept on this browser; in private mode, this page's only. */
export function read(key) { try { return JSON.parse(localStorage.getItem(key) ?? "null"); } catch { return null; } }
export function write(key, v) { try { if (v) localStorage.setItem(key, JSON.stringify(v)); else localStorage.removeItem(key); } catch { /* private mode */ } }

/** "1 task", "2 tasks". */
export const plural = (n, one) => `${n} ${one}${n === 1 ? "" : "s"}`;

export function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = textForm(String(text));
  return e;
}

export function part(tag, cls, ...nodes) {
  const node = el(tag, cls);
  node.append(...nodes);
  return node;
}

/** A pointer's press leaves the key unfocused: no later key (Esc in Chrome) or closing window lights its ring, and a
 *  menu it opens gives the focus back to what had it (the terminal's keys); Tab still reaches it, ringed
 *  (docs/UI-GUIDELINES.md "Focus"). */
const noFocus = (e) => e.preventDefault();
export const pointerKey = (b) => { b.addEventListener("mousedown", noFocus); return b; };
/** The board's text keys only (the terminal's keys would lose it): pointerKey's, and a field in focus gives the focus up
 *  first, so its change comes before the key's click (WebKit dropped it when the click drew the field anew). */
const blurField = (e) => {
  const f = document.activeElement;
  if (f instanceof HTMLInputElement || f instanceof HTMLTextAreaElement) f.blur();
  e.preventDefault();
};
export const textKey = (b) => { b.addEventListener("mousedown", blurField); return b; };

/** A small icon button: its class, icon (and its size), label for a screen reader, tooltip; a pointer's press leaves it
 *  unfocused (pointerKey). Inside a draggable it starts no drag. */
export function iconButton(cls, name, label, title, onClick, size = 14) {
  const b = pointerKey(el("button", cls));
  b.type = "button";
  b.title = title;
  b.setAttribute("aria-label", label);
  b.append(icon(name, size));
  b.addEventListener("click", (e) => { e.stopPropagation(); onClick(b); });
  return b;
}

// Projects' colors (AC-92): ten hues far apart, in pastel (tokens.css), so two projects never read as one
export const PROJECT_HUES = [25, 145, 265, 85, 205, 325, 55, 175, 295, 115];
export const hueColor = (h) => `oklch(var(--proj-l) var(--proj-c) ${h})`;
export const projectHue = (p) => PROJECT_HUES[(p.color ?? 0) % PROJECT_HUES.length];
export const projectColor = (p) => hueColor(projectHue(p));
/** Default has no color of its own; its sessions keep their folders'. */
export const colored = (p) => p?.color >= 0;
/** A project's color for words and dots, Default's the muted. */
export const projectInk = (p) => (colored(p) ? projectColor(p) : "var(--muted)");

/** A name cut to `most` characters (code points, so no letter is split), then "…" (terminum-agents.md AC-39). */
export function shortName(name, most = 32) {
  const cs = [...name];
  return cs.length <= most ? name : `${cs.slice(0, most).join("").trimEnd()}…`;
}

/** A waiting line in its project's stripe and name (terminum-agents.md AC-39, terminum-kanban.md AC-117); `p` none (no
 *  project, Default) takes both off. Written only when changed: the lines are a live region. Answers the whole name
 *  when it was cut, for the key's title. */
export function tagProject(line, p) {
  let tag = line.querySelector(".proj");
  line.classList.toggle("inproject", !!p);
  if (!p) { line.style.removeProperty("--proj"); tag?.remove(); return ""; }
  const cut = shortName(p.name), ink = projectInk(p), words = textForm(`${cut}:`);
  if (line.style.getPropertyValue("--proj") !== ink) line.style.setProperty("--proj", ink);
  if (!tag) { tag = el("span", "proj"); line.querySelector(".what").before(tag); }
  if (tag.textContent !== words) tag.textContent = words;
  return cut === p.name ? "" : p.name;
}

/** A reminder not resolved (AC-91). */
export const openReminder = (i) => !!i.remind && !i.resolved;
/** pane id -> {mark, remind}: the list's marks of a session with bookmarks or open reminders. */
export function markedSessions(items) {
  const m = new Map();
  for (const b of items) {
    const mark = !b.remind, remind = openReminder(b);
    if (b.orphaned || !(mark || remind)) continue;
    const v = m.get(b.sid) ?? { mark: false, remind: false };
    v.mark ||= mark; v.remind ||= remind;
    m.set(b.sid, v);
  }
  return m;
}

export const when = (s) => new Date(s * 1000).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });

/** A frame to `src` with no step of the page's Back: a frame put in anew leaves no history, a moved one a step
 *  Back would undo first (terminum-kanban.md AC-134). */
export function frameTo(frame, src) {
  const home = frame.parentNode, next = frame.nextSibling;
  frame.remove();
  frame.src = src;
  home.insertBefore(frame, next);
}
