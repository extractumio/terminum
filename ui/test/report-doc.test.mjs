// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum-reports.md AC-268: a report's body is shown as the Files viewer shows a document, in the format it is
// written in; its only pictures are its own files, any other local path is never read.
import assert from "node:assert/strict";
import { test } from "node:test";
import { load } from "./bundle.mjs";

const { formatOf } = await load("src/report-doc.ts");
const { ownFile, renderMd } = await load("src/markdown-core.ts");

test("AC-268 a body that starts as an HTML document is HTML; anything else Markdown", () => {
  for (const html of ["<!doctype html><p>x", "  \n<!DOCTYPE HTML>\n<html>", "<html lang=en><body>x", "<html>"]) assert.equal(formatOf(html), "html", html);
  for (const md of ["## Findings", "Some <html> in a line", "<htmlish>", "<div>x</div>", "", "```html\n<html></html>\n```"]) assert.equal(formatOf(md), "markdown", md);
});

test("AC-268 a report's pictures: its own files only, never a path it names", () => {
  assert.equal(ownFile("/api/task-files/0123456789abcdef0123456789abcdef"), "/api/task-files/0123456789abcdef0123456789abcdef");
  for (const p of ["/.ssh/id.png", "/Users/alex/a.png", "/api/raw", "/api/task-filesX/1"]) assert.equal(ownFile(p), "", p);
  const html = renderMd("![key](../../.ssh/id.png) ![home](/Users/alex/a.png) ![own](/api/task-files/0123456789abcdef0123456789abcdef)", { dir: "/", imagePath: ownFile });
  assert.equal((html.match(/data-raw=/g) ?? []).length, 1, html);
  assert.ok(html.includes('data-raw="/api/task-files/0123456789abcdef0123456789abcdef"'), html);
});
