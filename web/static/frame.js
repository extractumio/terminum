// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// How a full-screen program in a pane wider than the view is drawn (AC-04, AC-05), as
// iterm-enhancer's web app does: a frame (mc, htop, an agent with its side panel open) as Fit,
// or as Grid when Fit would make it unreadable; rows that end at their text (Codex, a coding
// agent's full-screen mode, vim, a plain tmux) re-flow in Wrap at the chosen size.

/** Fit's text size: the pane's columns in the view's width, at most a little over the chosen size. */
export const fitPx = (view) => Math.min(Math.floor(((view.inner().w - 1) / view.cols / view.charRatio()) * 10) / 10, view.chosenSize() + 4);

/** A frame wider than the view: on the alternate screen, at least half of the rows with text on
 *  them reach past the columns that fit at the chosen size (borders and backgrounds to the pane's
 *  edge; re-flowed, the frame breaks), and a third to stay one (view.frame: the last verdict), so
 *  an agent printing a few long lines does not flip the view between Wrap and Fit. A row's text
 *  ends at its last character: the blanks a program pads its rows with are the padding Wrap hides
 *  (render.js markEdge). */
export function framed(view) {
  if (!view.alt || !view.cols || !view.tooWide()) return false;
  const fit = Math.floor(view.inner().w / (view.charRatio() * view.chosenSize()));
  let drawn = 0, wide = 0;
  for (const r of view.screen) {
    if (!r || !r.txt.trim()) continue;
    drawn++;
    if (r.txt.length > fit) wide++;
  }
  return drawn > 0 && wide * (view.frame ? 3 : 2) >= drawn;
}

/** How a frame is shown: Fit while the text stays at least `min` px, else Grid (the chosen size,
 *  the frame's left part in sight, the rest by scrolling sideways); null when there is no frame. */
export const frameMode = (view, min) => (framed(view) ? (fitPx(view) >= min ? "fit" : "grid") : null);
