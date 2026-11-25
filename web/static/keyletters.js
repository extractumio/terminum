// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Which letter presses which button of a window (terminum.md AC-281): one rule for every window, no list per button.

/** The letters a window's buttons may take with ⌘⇧ (Ctrl+Shift off a Mac). The others are never given: C, E, I, J,
 *  K, M open the browsers' developer tools; N, P, Q, T, W (and R in Firefox) are browser keys a page never gets; U
 *  is Linux's Unicode input; V and Z paste as plain text and redo in a text field, X turns its direction (Firefox). */
export const FREE = "ABDFGHLOSY";

/** Each label's letter as {letter, at} (`at`: its place in the label, to underline), or null: none left, or no
 *  label. The rightmost label chooses first (a window's main keys stand at the right), each taking its words'
 *  first letters before its other letters, the first free one no other took; `taken`: letters already given. */
const isLetter = (ch) => /\p{L}/u.test(ch ?? "");

export function letters(labels, taken = []) {
  const given = new Set(taken), out = labels.map(() => null);
  for (let i = labels.length - 1; i >= 0; i--) {
    const label = labels[i] ?? "", places = [...Array(label.length).keys()];
    const free = (p) => FREE.includes(label[p].toUpperCase()) && !given.has(label[p].toUpperCase());
    const at = places.find((p) => isLetter(label[p]) && !isLetter(label[p - 1]) && free(p)) ?? places.find(free);
    if (at === undefined) continue;
    out[i] = { letter: label[at].toUpperCase(), at };
    given.add(out[i].letter);
  }
  return out;
}

/** A key as the user's keyboard names it: ⌘⇧D on a Mac, Ctrl+Shift+D elsewhere; "Enter" is ↩ (Enter off a Mac). */
export function keyName(key, mac, shift = true) {
  const k = key === "Enter" ? (mac ? "↩" : "Enter") : key;
  return mac ? `⌘${shift ? "⇧" : ""}${k}` : `Ctrl+${shift ? "Shift+" : ""}${k}`;
}

/** The same key for `aria-keyshortcuts`. */
export const ariaKey = (key, mac) => `${mac ? "Meta" : "Control"}+Shift+${key}`;
