// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { check, until } from "./check.mjs";
import { shot } from "./fake.mjs";

export function configFixture(detector, at) {
  let reverted = false;
  return {
    reverts(suffix, method) {
      if (suffix !== "/config/7/revert" || method !== "POST") return false;
      reverted = true;
      return true;
    },
    screen: () => ({
      config_changes: [{ id: 7, detector, class: "tune", at, applied: true, reverted,
        previous: {}, next: { threshold: { absolute: 4 } }, replay: "Replay: 6 → 5 firings." }],
      sunset: [{ detector: "flow.backlog_growth", removal: true }],
    }),
  };
}

export async function checkConfiguration(page, suffix) {
  const reports = page.locator("#sv-reports");
  const config = reports.locator("#sv-config-changes");
  await config.locator("summary").focus();
  await config.locator("summary").press("Enter");
  const revert = config.getByRole("button", { name: "Revert change" });
  await revert.focus();
  const replay = await config.getByText(/Replay: 6 .*5 firings/).isVisible();
  const sunset = await config.getByText("flow.backlog_growth · Proposed for removal", { exact: true }).isVisible();
  const dimensions = await revert.evaluate((key) => ({
    focus: key === document.activeElement, fits: document.documentElement.scrollWidth <= innerWidth,
    touch: !("ontouchstart" in window) || key.getBoundingClientRect().height >= 40,
  }));
  check("AC-253 AC-257 configuration history, its undo, and sunset fit the Supervisor screen",
    replay && sunset && Object.values(dimensions).every(Boolean), JSON.stringify({ replay, sunset, ...dimensions }));
  await shot(page, `supervisor-configuration-${suffix}`);
  await revert.press("Enter");
  check("AC-253 one owner key reverts the recorded change",
    await until(() => config.getByText(/Reverted · tune/).isVisible()));
  for (const title of ["Decisions waiting (1)", "Weekly review ·", "Exception digest ·"]) {
    const summary = reports.getByText(title, { exact: title === "Decisions waiting (1)" });
    if (!await summary.evaluate((node) => node.parentElement.open)) await summary.click();
  }
}
