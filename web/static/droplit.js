// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// An outline shown while files are dragged over a place (AC-36, AC-106): each dragover keeps it, and
// it goes once they stop for 800 ms, whatever ended the drag (Esc, a drop in the Files panel's
// frame, the pointer leaving through it): a browser repeats dragover at least every 550 ms while a
// drag is over the page (HTML's drag processing model).

const QUIET = 800;

/** `set(on)` shows or hides the outline; the result, called `lit(true)` at each dragover and
 *  `lit(false)` on a leave or a drop, keeps it no longer than the drag. */
export function dropLit(set) {
  let off = 0;
  const lit = (on) => {
    clearTimeout(off);
    set(on);
    if (on) off = setTimeout(() => lit(false), QUIET);
  };
  return lit;
}
