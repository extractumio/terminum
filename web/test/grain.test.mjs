// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The grain's tile (grain.svg, grain.css): an SVG the browser can load. A comment holding "--"
// is not XML: the image fails, and the chrome loses its grain without a word.
import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";

const svg = fs.readFileSync(new URL("../static/grain.svg", import.meta.url), "utf8");

test("grain.svg: its comments hold no double hyphen, its filter is there", () => {
  for (const c of svg.matchAll(/<!--([\s\S]*?)-->/g)) assert.ok(!c[1].includes("--"), c[1].slice(0, 80));
  assert.match(svg, /<filter id="specks"/);
  assert.ok(svg.trimStart().startsWith("<svg") && svg.trimEnd().endsWith("</svg>"));
});
