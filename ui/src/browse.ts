// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Any folder in the tree (AC-275): "Open folder…" takes a typed path and the tree shows that
// folder in the pane's place, kept in the pane's workspace (`browse`) through a cd, a pane
// switch or a reload, until Back shows the pane's folder again.

import { api, isPage, type Page } from "./api";
import { PIN_HOME } from "./embed";
import { typedFolder } from "./paths";

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

export interface BrowseHost {
  /** the folder the tree shows now, and the pane's own */
  shown(): string;
  pane(): string;
  /** show `path` (null: the pane's folder again) */
  go(path: string | null): Promise<void>;
  focusTree(): void;
}

/** The header's Open folder button, its path field and the Back button. */
export class Browse {
  private field = $<HTMLInputElement>("goto");
  private why = $("goto-why");
  private asked = 0;   // the newest Enter: an older check's answer is dropped

  constructor(private host: BrowseHost) {
    $("browse").onclick = () => this.setOpen(!$("browse").classList.contains("on"));
    $("back").onclick = () => void host.go(null);
    this.field.addEventListener("input", () => this.say(""));
    this.field.addEventListener("keydown", (e) => {
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); this.setOpen(false); host.focusTree(); }
      if (e.key === "Enter") { e.preventDefault(); void this.submit(); }
    });
  }

  /** Back is offered while a chosen folder is shown, and names the pane's folder it goes to. */
  render(browsing: boolean) {
    $("back").hidden = !browsing;
    $("back").title = `Back to the session's folder: ${this.host.pane()}`;
  }

  /** The path field opens holding the shown folder; closing it drops a pending check. */
  private setOpen(on: boolean) {
    this.asked++;
    this.field.hidden = !on;
    $("browse").classList.toggle("on", on);
    this.say("");
    if (!on) return;
    const shown = this.host.shown();
    this.field.value = shown === "/" ? "/" : shown + "/";
    this.field.focus();
    this.field.setSelectionRange(this.field.value.length, this.field.value.length);
  }

  private say(text: string) {
    this.why.textContent = text;
    this.field.toggleAttribute("aria-invalid", !!text);
  }

  /** The typed folder is read once first: what cannot be listed says why here, the field kept. */
  private async submit() {
    const run = ++this.asked;
    const typed = typedFolder(this.field.value, this.host.shown(), PIN_HOME);
    if ("error" in typed) return this.say(typed.error);
    let why: string | null;
    try {
      const r = await api<Page>("GET", "/api/ls", { query: { path: typed.path, offset: 0, limit: 1 }, is: isPage });
      why = r.status === "error" ? r.error ?? "not readable" : null;
    } catch (e) { why = (e as Error).message; }
    if (run !== this.asked) return;
    if (why) return this.say(`${typed.path}: ${why}`);
    this.setOpen(false);
    await this.host.go(typed.path);
    this.host.focusTree();
  }
}
