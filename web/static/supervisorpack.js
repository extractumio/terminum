// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-220 and AC-222: portable preferences and explicit approval of a single, fully displayed command.
import { el } from "./listparts.js";
import { button, downloadBlob, showError } from "./kanbanutil.js";
import { actions, check, grid, hint, literal, row, section, stack, text, tick } from "./supervisorfields.js";

const named = (id) => encodeURIComponent(id);

function feedback() {
  const node = el("p");
  node.setAttribute("role", "alert");
  node.hidden = true;
  return node;
}

export function probeCommands({ request, run }) {
  const id = literal(text("")), command = literal(text("", "area")), rows = el("div", "sv-stack");
  const addError = feedback();
  const add = button("textkey", "Review new probe command", () => run(addError, async () => {
    const probe = id.value.trim();
    if (!probe) throw new Error("Enter a probe ID.");
    if (probe === "." || probe === "..") throw new Error("A probe ID cannot be a dot or two dots.");
    const saved = await request(`/probe-candidates/${named(probe)}`, "POST", { command: command.value });
    draw(saved);
    id.value = command.value = "";
    return [...rows.querySelectorAll("button[data-probe]")].find((key) => key.dataset.probe === probe);
  }));

  function pending(probe) {
    const error = feedback(), preview = el("div", "sv-stack");
    const route = `/probe-candidates/${named(probe)}`;
    const review = button("textkey", "View command", () => run(error, async () => {
      const candidate = await request(`${route}/preview`, "POST", {});
      const full = el("pre", "field sv-command", candidate.command);
      const confirm = button("textkey", "Approve command", () => run(error, async () => {
        draw(await request(`${route}/confirm`, "POST", { approval: candidate.approval }));
        return add;
      }));
      const cancel = button("textkey", "Close preview", () => { preview.replaceChildren(); review.focus(); });
      preview.replaceChildren(full, actions(cancel, confirm));
      return confirm;
    }));
    review.dataset.probe = probe;
    const discard = button("textkey", "Discard", () => run(error, async () => {
      draw(await request(route, "DELETE"));
      return add;
    }));
    return stack(el("h3", null, `${probe} (unset)`), actions(review, discard), preview, error);
  }

  function draw(view) {
    const confirmed = view.confirmed_probes.map((probe) => {
      const error = feedback();
      const remove = button("textkey", "Remove command", () => run(error, async () => {
        draw(await request(`/probe-commands/${named(probe)}`, "DELETE"));
        return add;
      }));
      return stack(el("h3", null, `${probe} (approved)`), actions(remove), error);
    });
    rows.replaceChildren(...view.unset_probes.map(pending), ...confirmed);
  }

  const node = section("Probe commands", hint("Approved commands will provide input for the named probes. Review each command before approving it."),
    grid(row("Probe ID", id), row("Command", command)), actions(add), addError, rows);
  return { node, draw };
}

export function portableSettings({ request, run, revision, imported }) {
  const commands = check(false), secrets = check(false), file = text("", "file");
  file.accept = ".json,.txt,application/json,text/plain";
  const exportError = feedback(), importError = feedback();
  const output = button("textkey", "Export settings", () => run(exportError, async () => {
    const pack = await request("/export", "POST", { include_commands: commands.checked, include_secrets: secrets.checked }, true);
    downloadBlob(new Blob([pack], { type: "text/plain;charset=utf-8" }), "supervisor-settings.json");
  }));
  const input = button("textkey", "Import settings", () => run(importError, async () => {
    const selected = file.files[0];
    if (!selected) throw new Error("Choose a Supervisor settings file.");
    if (selected.size > 768 * 1024) throw new Error("The Supervisor settings file is larger than 768 KiB.");
    imported(await request("/import", "POST", { revision: revision(), file: await selected.text() }));
  }));
  file.addEventListener("change", () => showError(importError, ""));
  return section("Import and export",
    hint("Export uses saved preferences. Include commands or recognized secrets only when you want them in the file."),
    actions(tick("Include commands", commands), tick("Include secrets", secrets), output), exportError,
    hint("Import replaces the preferences and the draft in this tab. The project switch, tier agents and approved probe commands stay as they are."),
    row("Settings file", file), actions(input), importError);
}
