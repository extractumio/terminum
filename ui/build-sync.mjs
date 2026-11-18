// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Makes a folder hold exactly the given files, writing only those whose bytes differ: an unchanged
// file keeps its time, so cargo does not compile terminumd again for a UI build that changed nothing
// (rust-embed embeds ui/dist; cargo judges embedded files by their times). A file only added is
// seen through TM_BUILD, which hashes ui/src and ui/public: a cargo build without it may miss one.
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";

/** `path` relative to `dir`, "/"-separated: the keys of syncDir's files. */
export const relPath = (dir, path) => relative(dir, path).split("\\").join("/");

/** Every file under `dir`, as relPath keys. */
export function listFiles(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { recursive: true, withFileTypes: true })
    .filter((e) => e.isFile())
    .map((e) => relPath(dir, join(e.parentPath, e.name)));
}

/** `files`: Map of relPath → bytes (Uint8Array). Folders a removal leaves empty go too. */
export function syncDir(dir, files) {
  mkdirSync(dir, { recursive: true });
  for (const [rel, bytes] of files) {
    const path = join(dir, rel);
    const st = statSync(path, { throwIfNoEntry: false });
    if (st?.size === bytes.length && Buffer.compare(readFileSync(path), bytes) === 0) continue;
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, bytes);
  }
  const root = resolve(dir);   // the walk up stops here, however `dir` was written
  for (const rel of listFiles(root)) {
    if (files.has(rel)) continue;
    rmSync(join(root, rel));
    for (let p = dirname(join(root, rel)); p !== root && readdirSync(p).length === 0; p = dirname(p)) rmSync(p, { recursive: true });
  }
}
