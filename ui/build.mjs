// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Bundles the panel into dist/ (embedded into terminumd by rust-embed, served under /fb/).
// Language grammars are split into lazy chunks, so the first load stays small.
// A build writes only the files of dist/ whose bytes changed (build-sync.mjs): the same sources
// leave dist/ as it was, so make build then make install compile terminumd once.
import * as esbuild from "esbuild";
import { cpSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { listFiles, syncDir } from "./build-sync.mjs";

const watch = process.argv.includes("--watch");
const build = process.env.TM_BUILD ?? "dev";
if (watch) {
  rmSync("dist", { recursive: true, force: true });
  cpSync("public", "dist", { recursive: true });
}
writeFileSync(".build-id", build + "\n"); // the browser test runs its terminumd as this build
const ctx = await esbuild.context({
  write: watch,
  // renderers.js: the web app's output widgets' renderers (AC-56); it shares chunks with the panel
  entryPoints: ["src/main.ts", { in: "src/web-widgets.ts", out: "renderers" }],
  bundle: true,
  splitting: true,
  format: "esm",
  outdir: "dist",
  chunkNames: "chunks/[name]-[hash]",
  minify: !watch,
  sourcemap: watch ? "inline" : false,
  target: ["safari16"],
  loader: { ".svg": "text" },  // icons are inlined as markup (src/icons.ts)
  define: { __BUILD__: JSON.stringify(build) }, // the build id (AC-34)
  logLevel: "info",
});
if (watch) await ctx.watch();
else {
  const { outputFiles } = await ctx.rebuild();
  await ctx.dispose();
  const files = new Map(listFiles("public").map((rel) => [rel, readFileSync(join("public", rel))]));
  for (const f of outputFiles) files.set(relative("dist", f.path).split("\\").join("/"), f.contents);
  syncDir("dist", files);
}
