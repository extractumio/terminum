// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-213: the project's P0 waiting hours in the Supervisor tab, checked from e2e_supervisor.mjs.
import { check } from "./check.mjs";
import { shot } from "./fake.mjs";

const puts = (calls) => calls.filter(({ method, suffix }) => method === "PUT" && suffix === "/settings").length;

/** Shows the saved hours, refuses 0 inline before a write, then leaves 1 and 4 in the draft. */
export async function waitingHours(page, panel, calls, suffix) {
  const group = panel.locator('details[data-section="Detectors and mutes"]');
  await group.locator(":scope > summary").click();
  const second = group.getByLabel("P0 waiting to Card level, hours"), third = group.getByLabel("Then to Incident level, further hours");
  check("AC-213 the tab shows the project's P0 waiting hours", await second.inputValue() === "24" && await third.inputValue() === "24");
  const before = puts(calls);
  await second.fill("0");
  await page.click("#supervisorok");
  const error = group.locator(".sv-field-error");
  await error.waitFor();
  check("AC-213 a P0 waiting hour out of range is refused inline before a write",
    (await error.textContent()).startsWith("P0 waiting to Card level, hours: ") && await second.getAttribute("aria-invalid") === "true"
    && await second.evaluate((node) => node === document.activeElement) && puts(calls) === before);
  await shot(page, `supervisor-p0-hours-${suffix}`);
  await second.fill("1");
  await third.fill("4");
  check("AC-213 a corrected hour clears its error", await error.count() === 0 && await second.getAttribute("aria-invalid") === null);
  await group.locator(":scope > summary").click();
}

/** The saved pack carries the edited hours. */
export function checkWaitingSave(settings) {
  check("AC-213 Save keeps the P0 waiting hours",
    JSON.stringify(settings.p0_waiting) === JSON.stringify({ level2_hours: 1, level3_hours: 4 }));
}
