// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-15: persistence of the private live runner's tmux layout between browsers.
import fs from "node:fs";
import path from "node:path";

/** Whether layout.json holds the panes tmux has now (quick shells apart: never kept). terminumd writes it within
 *  5 s of a change (AC-15), so a browser's last window closed just before the stop would be made again for the next
 *  browser, a shell under that window's name: one named claude is a coding agent's row (titles.rs), first in the list */
export function layoutKept(tmux, app) {
  const quick = new Set(tmux("list-windows", "-a", "-F", "#{window_id} #{@tm-quick}").split("\n").map((l) => l.split(" ")).filter(([, q]) => q).map(([w]) => w));
  const live = tmux("list-panes", "-a", "-F", "#{window_id} #{pane_id}").split("\n").map((l) => l.split(" ")).filter(([w, p]) => p && !quick.has(w)).map(([, p]) => p).sort();
  let kept = [];
  try {
    kept = JSON.parse(fs.readFileSync(path.join(app, "layout.json"), "utf8")).sessions.flatMap((s) => s.windows.flatMap((w) => w.panes.map((p) => p.id))).sort();
  } catch { /* not written yet */ }
  return live.length > 0 && live.join() === kept.join();
}
