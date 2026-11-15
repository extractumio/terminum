// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-31: browser time controls the stimulus, not how long the bridge or scheduler takes.
// Each step lets a real layout/paint turn run: runFor alone can coalesce ResizeObservers.
import { until } from "./check.mjs";

export const paint = (page) => page.evaluate(() => new Promise((resolve) => {
  const observer = new ResizeObserver(() => { observer.disconnect(); resolve(); });
  observer.observe(document.body);
}));
export async function advance(page, ms) {
  for (let left = ms; left > 0; left -= 16) {
    await page.clock.runFor(Math.min(left, 16));
    await paint(page);
  }
}
/** Wait for the outcome with the usual load slack, advancing controlled time where installed. */
export const outcome = (page, ok, clock = false) => until(async () => {
  if (clock) await advance(page, 32);
  return ok();
});
