// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Files dragged from Finder (or another app) onto the tree (AC-95): they upload into the folder
// under the pointer: a folder's own row that folder, a file's row the folder it is in, below the
// rows the root. While the drag lasts that folder's rows are marked and a line over the tree says
// where they go; moving up or down picks another. Each is saved by the tree's host, one after
// another, never over a file there: a name taken asks Keep Both or Skip. Rows dragged out of the
// tree itself (tree-drag.ts) carry no files and are not taken here.
import { ApiError, uploadFile, UPLOAD_MAX } from "./api";
import { ask } from "./dialogs";
import { basename, dirname } from "./paths";

/** The folder a drop on row `p` (`dir`: whether it is a folder; null: below the rows) goes to. */
export const dropTarget = (row: { p: string; dir: boolean } | null, root: string) => (!row ? root : row.dir ? row.p : dirname(row.p));

const plural = (n: number, one: string) => `${n} ${one}${n === 1 ? "" : "s"}`;

export interface DropHost {
  root(): string | null;
  host(): string | null;
  readOnly(dir: string): boolean;
  mark(dir: string | null): void;
  toast(text: string): void;
}

/** Takes files dropped on `el`'s rows (`.row[data-p][data-d]`). */
export function bindDrop(el: HTMLElement, h: DropHost) {
  const note = document.createElement("div");
  note.className = "dropnote";
  note.hidden = true;
  document.body.append(note);
  const files = (e: DragEvent) => !!e.dataTransfer?.types.includes("Files");
  const target = (e: DragEvent) => {
    const row = (e.target as Element).closest?.<HTMLElement>(".row[data-p]");
    return dropTarget(row ? { p: row.dataset.p!, dir: row.dataset.d === "1" } : null, h.root()!);
  };
  const clear = () => { h.mark(null); note.hidden = true; };

  el.addEventListener("dragover", (e) => {
    if (!files(e) || !h.root()) return;
    e.preventDefault();
    const dir = target(e), ro = h.readOnly(dir);
    e.dataTransfer!.dropEffect = ro ? "none" : "copy";
    const n = e.dataTransfer!.items.length;
    note.textContent = ro ? "Read-only: outside writable folders" : `Upload ${n ? plural(n, "file") : "files"} to ${dir === h.root() ? basename(dir) + " (the root)" : basename(dir)}`;
    note.classList.toggle("ro", ro);
    const r = el.getBoundingClientRect();         // over the tree's top, never moving its rows
    Object.assign(note.style, { left: `${r.left + 6}px`, top: `${r.top + 6}px`, maxWidth: `${r.width - 12}px` });
    note.hidden = false;
    h.mark(ro ? null : dir);
  });
  el.addEventListener("dragleave", (e) => { if (!el.contains(e.relatedTarget as Node | null)) clear(); });
  el.addEventListener("drop", (e) => {
    if (!files(e) || !h.root()) return;
    e.preventDefault();
    const dir = target(e);
    clear();
    if (h.readOnly(dir)) return h.toast("Read-only: outside writable folders");
    // a folder dropped is not uploaded (its entry says it is one; a File of it reads nothing)
    const items = [...e.dataTransfer!.items].filter((i) => i.kind === "file");
    const folders = items.filter((i) => i.webkitGetAsEntry?.()?.isDirectory).map((i) => i.getAsFile()?.name ?? "a folder");
    const list = items.filter((i) => !i.webkitGetAsEntry?.()?.isDirectory).map((i) => i.getAsFile()).filter((f): f is File => !!f);
    void send(dir, list, folders);
  });

  async function send(dir: string, list: File[], folders: string[]) {
    const host = h.host(), where = basename(dir);
    let done = 0, skipped = 0, failed = "";
    for (const [i, f] of list.entries()) {
      if (list.length > 1 || f.size > 4 << 20) h.toast(`Uploading ${f.name} (${i + 1} of ${list.length})…`);
      try {
        if (f.size > UPLOAD_MAX) throw new ApiError(413, "too_large", "Larger than the 100 MB an upload may be", null);
        try { await uploadFile(dir, f.name, f, false, host); }
        catch (e) {
          if (!(e instanceof ApiError) || e.code !== "exists") throw e;
          const choice = await ask(`“${f.name}” is in ${where} already`, "Keep Both saves this one under the next free number; nothing is written over.",
            [{ id: "skip", label: "Skip" }, { id: "both", label: "Keep Both", primary: true }]);
          if (choice !== "both") { skipped++; continue; }
          await uploadFile(dir, f.name, f, true, host);
        }
        done++;
      } catch (e: any) { failed = `${f.name}: ${e.message}`; break; }   // the rest are not sent
    }
    const said = [done || !failed ? `Uploaded ${plural(done, "file")} to ${where}` : "",
      skipped ? `${skipped} skipped` : "", folders.length ? `folders are not uploaded (${folders.join(", ")})` : "", failed].filter(Boolean);
    h.toast(said.join("; "));
  }
}
