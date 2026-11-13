// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Bundles one UI module for node (no DOM) and imports it, for the unit tests.
import * as esbuild from "esbuild";

export async function load(entry, options = {}) {
  const out = await esbuild.build({
    entryPoints: [entry], bundle: true, format: "esm", platform: "neutral", write: false,
    mainFields: ["module", "main"], logLevel: "silent", loader: { ".svg": "text" }, ...options,
  });
  return import("data:text/javascript;base64," + Buffer.from(out.outputFiles[0].text).toString("base64"));
}

/** An esbuild plugin that replaces imports by name ("./api") with the given module sources. */
export const stubs = (modules) => ({
  name: "stubs",
  setup(b) {
    const names = Object.keys(modules).map((n) => n.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&")).join("|");
    b.onResolve({ filter: new RegExp(`^(${names})$`) }, (a) => ({ path: a.path, namespace: "stub" }));
    b.onLoad({ filter: /.*/, namespace: "stub" }, (a) => ({ contents: modules[a.path], loader: "js" }));
  },
});

/** An `api` whose calls wait in globalThis.__calls ({args, resolve, reject}) until a test settles them. */
export const DEFERRED_API = `export const api = (...args) => new Promise((resolve, reject) => globalThis.__calls.push({ args, resolve, reject }));`;

/** One turn of the event loop: promise chains settled by a test have run. */
export const tick = () => new Promise((r) => setTimeout(r, 0));
