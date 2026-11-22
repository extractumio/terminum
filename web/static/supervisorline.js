// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-227–229. This revision and save are shared across projects.
import { button, showError } from "./kanbanutil.js";
import { el } from "./listparts.js";
import { actions, check, fieldError, grid, hint, literal, number, row, section, select, stack, text, tick } from "./supervisorfields.js";

const pad = (n) => String(n).padStart(2, "0");
const clock = (n) => `${pad(Math.floor(n / 60))}:${pad(n % 60)}`;
const minutes = (value) => {
  const [hours, mins] = value.split(":").map(Number);
  return hours * 60 + mins;
};

export function criticalLine(request, run) {
  const body = stack();
  const error = el("p", "merr");
  error.setAttribute("role", "alert");
  const result = el("p", "msay");
  result.setAttribute("role", "status");
  const node = section("Critical line", hint("Delivery settings apply to every project."), body, error, result);
  showError(error, "");

  function draw(saved) {
    const settings = saved.settings;
    const channels = literal(text(settings.channels.join("\n"), "area"));
    const link = literal(text(settings.link_base, "url"));
    link.placeholder = "https://terminum.example";
    const quiet = check(Boolean(settings.quiet));
    const [start, end] = (settings.quiet ?? [1380, 480]).map((n) => text(clock(n), "time"));
    const repeat = settings.ladder.map((n) => {
      const input = number(n, { min: 1, max: 1440, step: 1 });
      input.required = true;
      return input;
    });
    const enableTimes = () => { start.disabled = end.disabled = !quiet.checked; };
    enableTimes();
    quiet.addEventListener("change", enableTimes);
    const selected = select(settings.channels.map((_, i) => [String(i), `Channel ${i + 1}`]), "0");
    const labels = ["First repeat, minutes", "Second repeat, minutes", "Later repeats, every N minutes"];

    const save = button("textkey", "Save line settings", () => {
      try {
        for (const [label, input] of [["Incident link origin", link], ...labels.map((label, i) => [label, repeat[i]])]) {
          if (!input.validity.valid) fieldError(input, label, input.validationMessage);
        }
        if (quiet.checked && (!start.value || !end.value || start.value === end.value)) {
          fieldError(!start.value ? start : end, "Quiet hours", "Choose two different times.");
        }
      } catch { return; }
      run(error, async () => {
        result.textContent = "";
        const updated = await request("/line", "PUT", {
          revision: saved.revision,
          settings: {
            channels: channels.value.split("\n").map((line) => line.trim()).filter(Boolean),
            link_base: link.value.trim() || null,
            quiet: quiet.checked ? [minutes(start.value), minutes(end.value)] : null,
            ladder: repeat.map((input) => Number(input.value)),
          },
        });
        const focus = draw(updated);
        result.textContent = "Critical line settings saved.";
        return focus;
      });
    });
    const test = button("textkey", "Send a test alert", () => run(error, async () => {
      result.textContent = "";
      await request("/line/test", "POST", { channel: Number(selected.value), revision: saved.revision });
      result.textContent = "The command accepted the test alert. Check that it arrived.";
    }));
    test.disabled = selected.disabled = !settings.channels.length;
    body.replaceChildren(
      row("Channel executables, one absolute path per line, in order", channels),
      hint("Saving authorizes these commands to send fixed incident notices. Each executable reads the notice from standard input."),
      row("Incident link origin", link),
      tick("Quiet hours on the server’s clock", quiet),
      grid(row("Quiet hours start", start), row("Quiet hours end", end)),
      grid(...repeat.map((input, i) => row(labels[i], input))),
      actions(save),
      row("Saved channel to test", selected),
      actions(test),
    );
    return save;
  }

  async function load() {
    showError(error, "");
    try { draw(await request("/line")); }
    catch (e) {
      showError(error, e.message);
      body.replaceChildren(button("textkey", "Reload line settings", load));
    }
  }
  body.append(hint("Loading critical line settings…"));
  load();
  return node;
}
