// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A command line keeps what was typed: Smart Dashes and Smart Quotes (macOS, iOS) turn `--` into `—` and `"` into
// `“`, which a program reads as other words. `autocorrect="off"` asks the browser not to; this puts back what it did
// anyway (terminum-dispatcher.md AC-137): `—` was `--`, `–` a `-` between spaces; one typed on purpose goes too. A
// paste, a drop or an input method's composing stays as it came.

const TYPED = { "—": "--", "–": "-", "“": "\"", "”": "\"", "„": "\"", "‘": "'", "’": "'" };
const SMART = new RegExp(`[${Object.keys(TYPED).join("")}]`, "g");

/** `now` after an edit of `was`, with the marks in what the edit put in typed back; the caret `at` moved with them. */
export function typedBack(was, now, at) {
  let start = 0, end = 0;
  while (start < was.length && start < now.length && was[start] === now[start]) start++;
  while (end < was.length - start && end < now.length - start && was[was.length - 1 - end] === now[now.length - 1 - end]) end++;
  const put = now.slice(start, now.length - end), back = put.replace(SMART, (c) => TYPED[c]);
  if (back === put) return null;
  const value = now.slice(0, start) + back + now.slice(now.length - end);
  return { value, at: at + (at >= now.length - end ? back.length - put.length : 0) };
}

/** `field` keeps what its user types. */
export function asTyped(field) {
  let was = field.value;
  field.addEventListener("focus", () => { was = field.value; });
  field.addEventListener("input", (e) => {
    const typed = !e.isComposing && !/^insert(From(Paste|Drop)|CompositionText)/.test(e.inputType);
    const back = typed && typedBack(was, field.value, field.selectionEnd);
    if (back) {
      field.value = back.value;
      field.setSelectionRange(back.at, back.at);
    }
    was = field.value;
  });
}
