// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A phone's windows (terminum.md AC-173, "a phone's windows cover the screen"; phone.css): on a screen 600 px
// wide or narrower a window with fields, a text, a list or a document covers what is in sight, its ways on in
// the bar at its top, beside its title. The keys themselves move there as it opens (never copies: the page's
// code holds them), and back when the screen is wide again; what is seldom pressed stays in its row under the
// fields. The bar stands at the top of what is in sight: --app-top follows the visible part of the page, which
// iOS moves when it brings a focused field over the keyboard.

/** Where a window covers the screen: the page's one query of it (phone.css's own is the same). */
export const PHONE = "(max-width: 600px)";
const WINDOWS = 'dialog.dlg[data-size]:not([data-size="ask"]), form.footform';
const CANCEL = '[value="cancel"], [id$="cancel"]';   // ×'s on a phone
const phone = matchMedia(PHONE);

/** A row's ways on: from `.fromright`, else after Cancel, else from its primary; never Cancel. */
function waysOn(row) {
  const keys = [...row.children];
  const from = [".fromright", CANCEL, ".primary"].map((mark) => keys.findIndex((k) => k.matches(mark))).find((i) => i >= 0);
  return from === undefined ? [] : keys.slice(from).filter((k) => !k.matches(CANCEL));
}

/** A row filled anew by its window (a task's keys, by what the task allows) gives the bar its ways on again. */
const refilled = new MutationObserver((records) => {
  const rows = new Set(records.filter((r) => r.addedNodes.length).map((r) => r.target));
  for (const row of rows) arrange(row.closest(WINDOWS), true);
});

/** `refill`: its row was filled anew, with the whole set of its keys: the bar holds what that set says, nothing older. */
function arrange(win, refill = false) {
  const head = win?.querySelector(":scope > form > .dhead, :scope > .dhead");
  const row = win?.querySelector(":scope > form > .actions, :scope > .actions");
  if (!head || !row) return;
  let bar = head.querySelector(":scope > .barkeys");
  if (!phone.matches) {
    if (bar) { row.append(...bar.children); bar.remove(); }
    return;
  }
  if (!bar) {
    bar = document.createElement("div");
    bar.className = "actions barkeys";
    head.append(bar);
    refilled.observe(row, { childList: true });
  }
  const keys = waysOn(row), focused = document.activeElement;
  if (refill) bar.replaceChildren(...keys); else bar.append(...keys);
  if (keys.includes(focused)) focused.focus();     // a key moved keeps the keys
}

const all = () => { for (const win of document.querySelectorAll(WINDOWS)) arrange(win); };
new MutationObserver((records) => {
  for (const { target } of records) if (target.matches?.(WINDOWS) && (target.open || (target.matches("form") && !target.hidden))) arrange(target);
}).observe(document.documentElement, { attributes: true, attributeFilter: ["open", "hidden"], subtree: true });
phone.addEventListener("change", all);
all();

const seen = window.visualViewport;
const top = () => document.documentElement.style.setProperty("--app-top", `${Math.round(seen.offsetTop)}px`);
if (seen) { top(); seen.addEventListener("resize", top); seen.addEventListener("scroll", top); }
