// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Makes a folder hold exactly the given files, writing only those whose bytes differ: an unchanged
// file keeps its time, so cargo does not compile terminumd again for a UI build that changed nothing
// (rust-embed embeds ui/dist; cargo judges embedded files by their times). A file only added is
// seen through TM_BUILD, which hashes ui/src and ui/public: a cargo build without it may miss one.
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";

/** Every file under `dir`, as paths relative to it ("/"-separated). */
export function listFiles(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { recursive: true, withFileTypes: true })
    .filter((e) => e.isFile())
    .map((e) => relative(dir, join(e.parentPath, e.name)).split("\\").join("/"));
}

/** `files`: Map of relative path → bytes. Returns the paths written and removed. */
export function syncDir(dir, files) {
  const written = [];
  const removed = [];
  mkdirSync(dir, { recursive: true });
  for (const [rel, bytes] of files) {
    const path = join(dir, rel);
    if (existsSync(path) && Buffer.compare(readFileSync(path), Buffer.from(bytes)) === 0) continue;
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, bytes);
    written.push(rel);
  }
  for (const rel of listFiles(dir)) {
    if (!files.has(rel)) {
      rmSync(join(dir, rel));
      removed.push(rel);
    }
  }
  const dirs = readdirSync(dir, { recursive: true, withFileTypes: true }).filter((e) => e.isDirectory());
  for (const path of dirs.map((e) => join(e.parentPath, e.name)).sort((a, b) => b.length - a.length)) {
    if (readdirSync(path).length === 0) rmSync(path, { recursive: true }); // deepest first
  }
  return { written, removed };
}
