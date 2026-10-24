// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// "Follow the agent's folder" (AC-51, AC-19): the header's folder and Files show where the pane's
// coding agent works (a worktree it moved to, as its hooks said) rather than the pane's own folder;
// so do the list's rows (subOf).
// This browser's choice in the menu, else settings.json's page.followAgent; on by default.

const KEY = "followAgent";
let chosen = null, byDefault = true;
const listeners = new Set();
const tell = () => { for (const fn of listeners) fn(); };

/** Whether the header and Files follow the agent's folder now. */
export const following = () => chosen ?? byDefault;

/** A row's parts under its title (`item.sub`: program, folder, titles) with the folder its coding
 *  agent works in in place of the pane's while following it (AC-01). */
export function subOf(item, follow = following()) {
  const sub = item.sub ?? [];
  const at = follow && item.agentFolder ? sub.findIndex((p) => /^[~/]/.test(p)) : -1;
  return at < 0 ? sub : sub.map((p, i) => (i === at ? item.agentFolder : p));
}

/** fn() when it was turned on or off. */
export const onFollow = (fn) => listeners.add(fn);

/** The menu's switch `box`, kept in `store` (app.js's). {page(p)}: settings.json's page part. */
export function followSwitch(store, box) {
  chosen = store.get(KEY, null);
  box.checked = following();
  box.addEventListener("change", () => { chosen = box.checked; store.set(KEY, chosen); tell(); });
  return {
    page(p) {
      const was = following();
      byDefault = p.followAgent !== false;
      box.checked = following();
      if (following() !== was) tell();
    },
  };
}
