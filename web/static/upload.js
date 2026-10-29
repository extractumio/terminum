// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Files to the shown pane (AC-52): a pasted image, files from "Upload" on the hot keys or Attach
// (a phone's row under the input), files dropped on the page. Each is saved where the pane's shell runs (this Mac or its host), keeping
// a file's name, and its path is pasted as text without Enter: programs such as Claude Code take
// a file by its path. Dropped files go one at a time (the bridge takes two at once) and their
// paths are pasted together, one per line in a coding agent's box; so do picked ones. A file over
// the limit is not sent: the page says to give its path instead (a browser never gives a page a
// file's path).
import { shellWord } from "./input.js";

// what terminumd takes as a pasted image; another type the browser can draw goes as a PNG
const TAKEN = ["image/png", "image/jpeg", "image/gif", "image/webp"];
const MAX_BYTES = 100 << 20;            // terminumd's upload limit (page/upload.rs MAX_BYTES)

/** Why a file over the upload limit was not sent. */
export const tooLarge = (name) => `${name} is larger than the 100 MB an upload may be: type or paste the path to the file instead.`;

/** The image as terminumd takes it: as it is, else drawn into a PNG (TIFF from Preview, HEIC). */
export async function asTaken(blob) {
  if (TAKEN.includes(blob.type)) return blob;
  const bitmap = await createImageBitmap(blob).catch(() => null);
  if (!bitmap) return null;
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  canvas.getContext("2d").drawImage(bitmap, 0, 0);
  bitmap.close();
  return new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
}

// Rows dragged from the Files panel (ui/src/tree-drag.ts): {host, paths}, put as paths, nothing uploaded.
const PATHS = "application/x-terminum-paths";

/** The paths a drag from Files carries, or null: absolute, no control characters (each would be a key). */
export function draggedPaths(raw) {
  let m;
  try { m = JSON.parse(raw); } catch { return null; }
  const paths = Array.isArray(m?.paths) ? m.paths.filter((p) => typeof p === "string" && p.startsWith("/") && !/[\x00-\x1f\x7f]/.test(p)) : [];
  return paths.length ? { host: typeof m.host === "string" ? m.host : null, paths } : null;
}

/** Paths as pasted: one per line (`lines`), else separated by spaces. */
export const pathList = (paths, lines) => paths.map(shellWord).join(lines ? "\n" : " ");

/** sid(): the shown pane; host(): its host (null: this Mac); paste(text) at its input; lines():
 *  paths one per line: a coding agent's box with a bracketed paste (in a shell the lines would
 *  run as one command each at Enter). */
export function uploads({ sid, host = () => null, paste, lines = () => false, toast, hideToast }) {
  /** Saves one file: {path} or {error}. */
  async function save(blob, what, url, from) {
    const r = await fetch(url + encodeURIComponent(from), { method: "POST", headers: { "Content-Type": blob.type || "application/octet-stream" }, body: blob })
      .catch(() => null);
    const text = r ? await r.text().catch(() => "") : "";
    let m = {};
    try { m = JSON.parse(text); } catch { m = { error: text.trim() || undefined }; }   // the server's own refusals are plain text
    if (r?.status === 413) return { error: tooLarge(blob.name ?? `The ${what}`) };   // never the server's own words
    if (!r?.ok || !m.path) return { error: m.error ?? (r ? `The ${what} was not uploaded (${r.status}).` : "The Mac does not answer.") };
    return { path: m.path };
  }

  // The paths saved are pasted; a failure stops the rest and stays on screen.
  async function send(list, what) {
    const from = sid();
    if (!from) return toast("Open a session first.");
    const paths = [];
    let error = null;
    for (const [i, [blob, url]] of list.entries()) {
      if (blob.size > MAX_BYTES) { error = tooLarge(blob.name ?? `The ${what}`); break; }   // not sent at all
      toast(list.length > 1 ? `Uploading ${i + 1} of ${list.length}…` : `Uploading the ${what}…`, 60000);
      ({ path: paths[i], error } = await save(blob, what, url, from));
      if (error) { paths.pop(); break; }
    }
    if (paths.length && sid() !== from) return toast(`The ${what} was saved, but another session is shown now: nothing was pasted.`, 6000);
    if (paths.length) paste(pathList(paths, lines()));
    if (!error) return hideToast();
    toast(list.length > 1 ? `${paths.length} of ${list.length} uploaded (their paths pasted); ${list[paths.length][0].name}: ${error} The rest were not sent.` : error, 8000);
  }

  function putPaths(got) {
    if (!got) return;
    if (!sid()) return toast("Open a session first.");
    const on = host() ?? null;
    if (got.host !== on) return toast(`These files are on ${got.host ?? "this Mac"}; the session runs on ${on ?? "this Mac"}: nothing was put.`, 6000);
    paste(got.paths.map(shellWord).join(" "));
  }

  const fileUrl = (f) => `/upload?name=${encodeURIComponent(f.name)}&sid=`;
  return {
    async pasteImage(blob) {
      const image = await asTaken(blob);
      if (!image) return toast(`This browser cannot read a ${blob.type.replace("image/", "").toUpperCase()} image: nothing was pasted.`, 6000);
      return send([[image, "/paste?sid="]], "image");
    },
    uploadFiles: (list) => send(list.map((f) => [f, fileUrl(f)]), "file"),
    /** Files dropped anywhere on the page (a drop the page did not take would open the file in
     *  the browser and leave the page, with any unsaved edits in View); rows dragged from Files
     *  put their paths (AC-36). */
    bindDrop(target) {
      const kind = (e) => { const t = [...(e.dataTransfer?.types ?? [])]; return t.includes(PATHS) ? "paths" : t.includes("Files") ? "files" : null; };
      document.addEventListener("dragover", (e) => {
        if (!kind(e)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "copy";
        target.classList.add("dropping");
      });
      document.addEventListener("dragleave", (e) => { if (!e.relatedTarget) target.classList.remove("dropping"); });
      document.addEventListener("drop", (e) => {
        const k = kind(e);
        if (!k) return;
        e.preventDefault();
        target.classList.remove("dropping");
        if (k === "paths") return putPaths(draggedPaths(e.dataTransfer.getData(PATHS)));
        const items = [...e.dataTransfer.items].filter((it) => it.kind === "file");
        if (items.some((it) => it.webkitGetAsEntry?.()?.isDirectory)) return toast("A folder cannot be uploaded: drop its files.", 6000);
        const files = items.map((it) => it.getAsFile()).filter(Boolean);
        if (files.length) void send(files.map((f) => [f, fileUrl(f)]), "file");
      });
    },
  };
}
