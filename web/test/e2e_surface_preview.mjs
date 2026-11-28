// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { check } from "./check.mjs";
import { shot } from "./fake.mjs";

export async function surfacePreview(page, panel, calls, suffix, phone) {
  const preview = panel.getByRole("button", { name: "Preview last month" });
  await preview.click();
  await panel.getByText("This map would require no design reviews.").waitFor();
  const t3 = panel.getByLabel("T3 breaking surfaces");
  await t3.fill("unavailable/**");
  await preview.click();
  await panel.getByText("The history could not be read.").waitFor();
  await t3.fill("src/storage/**");
  await preview.click();
  await panel.getByText("T2: 3 · T3: 1 · 12 tasks sampled · 2 without a recorded diff").waitFor();
  check("AC-250 the surface preview sizes unsaved rules by tier and reports missing diffs",
    calls.filter(({ suffix }) => suffix === "/surfaces/preview").at(-1).body.t3[0] === "src/storage/**"
    && !calls.some(({ method, suffix }) => method === "PUT" && suffix === "/settings"));
  check("AC-250 the surface preview is reachable and fits on touch",
    await preview.evaluate((key, touch) => {
      const box = key.getBoundingClientRect();
      return box.left >= 0 && box.right <= innerWidth && (!touch || (box.width >= 40 && box.height >= 40));
    }, phone));
  await shot(page, `supervisor-surface-map-${suffix}`);
}
