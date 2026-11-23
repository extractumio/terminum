// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Any folder in the tree (AC-275): "Open folder…" takes a typed path and the tree shows that
// folder in the pane's place, kept in the pane's workspace (`browse`) through a cd, a pane
// switch or a reload, until Back shows the pane's folder again.

import { api, isPage, type Page, type Pane } from "./api";
import { typedFolder } from "./paths";

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

/** The folder a pane's tree shows: the one the user chose, else the pane's. */
export const shownOf = (p: Pick<Pane, "root" | "browse">) => p.browse || p.root;

export interface BrowseHost {
  /** the folder the tree shows now, and the pane's own */
  shown(): string;
  pane(): string;
  /** the pane's home, for `~` (the page names it; null: unknown) */
  home: string | null;
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
    $("browse").onclick = () => (this.field.classList.contains("on") ? this.close() : this.openField());
    $("back").onclick = () => void host.go(null);
    this.field.addEventListener("input", () => this.say(""));
    this.field.addEventListener("keydown", (e) => {
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); this.close(); host.focusTree(); }
      if (e.key === "Enter") { e.preventDefault(); void this.submit(); }
    });
  }

  /** Back is offered while a chosen folder is shown, and names the pane's folder it goes to. */
  render(browsing: boolean) {
    $("back").hidden = !browsing;
    $("back").title = `Back to the session's folder: ${this.host.pane()}`;
    $("browse").classList.toggle("on", this.field.classList.contains("on"));
  }

  private openField() {
    const shown = this.host.shown();
    this.field.value = shown === "/" ? "/" : shown + "/";
    this.field.classList.add("on");
    this.say("");
    this.render(!$("back").hidden);
    this.field.focus();
    this.field.setSelectionRange(this.field.value.length, this.field.value.length);
  }

  private close() {
    this.asked++;
    this.field.classList.remove("on");
    this.say("");
    this.render(!$("back").hidden);
  }

  private say(text: string) {
    this.why.textContent = text;
    this.field.toggleAttribute("aria-invalid", !!text);
  }

  /** The typed folder is read once first: what cannot be listed says why here, the field kept. */
  private async submit() {
    const run = ++this.asked;
    const typed = typedFolder(this.field.value, this.host.shown(), this.host.home);
    if ("error" in typed) return this.say(typed.error);
    const r = await api<Page>("GET", "/api/ls", { query: { path: typed.path, offset: 0, limit: 1 }, is: isPage })
      .catch((e: Error) => ({ status: "error", error: e.message }) as Page);
    if (run !== this.asked) return;
    if (r.status === "error") return this.say(`${typed.path}: ${r.error ?? "not readable"}`);
    this.close();
    await this.host.go(typed.path === this.host.pane() ? null : typed.path);
    this.host.focusTree();
  }
}
