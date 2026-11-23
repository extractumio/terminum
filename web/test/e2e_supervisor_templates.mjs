// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-263: template-filled fields stay in the editable draft until Save.
import { check } from "./check.mjs";
import { shot } from "./fake.mjs";

export const TEMPLATES = [{
  id: "prototype", name: "Prototype", never: ["Use real customer data in experiments."],
  priorities: ["Test the riskiest assumption first."],
  targets: { stability: "Keep the demo usable during trials." },
  surfaces: { t2: ["experiments/interfaces/**"], t3: ["fixtures/shared/**"] },
  probes: { git: true, gate_record: false },
}];

export const SCORE = { name: "Release cadence", verdict: "not measured yet", guard: "Stability", target: "Ship at each checkpoint.", metrics: [
  { name: "Releases per month", verdict: "not measured yet", missing: "release tags are unavailable" },
  { name: "Fixture measured series", verdict: "better", missing: null, before: 3, current: 5, noise: 0.5 },
] };

export async function templateDraft(page, panel, calls, suffix) {
  await panel.locator('[data-section="Project template"] summary').click();
  const choice = panel.getByLabel("Project type");
  const apply = panel.getByRole("button", { name: "Fill settings draft" });
  check("AC-263 filling a draft requires a project type", await apply.isDisabled());
  await choice.selectOption("prototype");
  await apply.click();
  const target = panel.getByLabel("Stability target", { exact: true });
  check("AC-263 filling a draft shows the candidate settings and keeps unrelated edits",
    await panel.getByLabel("Never list, one rule per line").inputValue() === TEMPLATES[0].never[0]
    && await target.isVisible() && await target.inputValue() === TEMPLATES[0].targets.stability
    && await panel.getByLabel("T3 breaking surfaces").inputValue() === "fixtures/shared/**"
    && await panel.getByLabel("Small tier").inputValue() === "mine"
    && !await panel.getByLabel("Gate records", { exact: true }).isChecked()
    && !await page.isChecked("#supervisoron")
    && !calls.some(({ method, suffix }) => method === "PUT" && suffix === "/settings"));
  await shot(page, `supervisor-template-${suffix}`);
  await target.fill("Keep the demo ready for user trials.");
  await panel.getByLabel("T3 breaking surfaces").fill("src/storage/**");
}
