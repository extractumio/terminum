// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The address a fetched file is shown from (shown.js, AC-09): only a picture, a video or a sound keeps its type.
import { test } from "node:test";
import assert from "node:assert/strict";
import { shownAs, shownUrl } from "../static/shown.js";

test("a picture, a video, a sound or a PDF keeps its type; an SVG is its own case; anything else is bytes", () => {
  for (const t of ["image/png", "image/jpeg", "IMAGE/GIF", "image/webp", "image/avif", "video/mp4", "audio/mpeg", "video/quicktime", "application/pdf"]) assert.equal(shownAs(t), t.toLowerCase(), t);
  assert.equal(shownAs("image/png; charset=binary"), "image/png");
  assert.equal(shownAs("image/svg+xml"), "svg");
  assert.equal(shownAs("Image/SVG+XML; charset=utf-8"), "svg");
  for (const t of ["text/html", "application/xhtml+xml", "text/xml", "application/xml", "application/x-pdf", "text/plain", "", null, undefined, "image/svg+xml-compressed", "image/png x"]) assert.equal(shownAs(t), "bytes", String(t));
});

test("an SVG is shown from a data: address, an HTML file from a blob: of bytes, a PNG from its own blob:", async () => {
  const svg = await shownUrl(new Blob(["<svg xmlns='http://www.w3.org/2000/svg'><script>x()</script></svg>"], { type: "image/svg+xml" }));
  assert.match(svg, /^data:image\/svg\+xml;base64,/);
  const html = await shownUrl(new Blob(["<script>x()</script>"], { type: "text/html" }));
  assert.equal((await fetch(html)).headers.get("content-type"), "application/octet-stream");
  const png = await shownUrl(new Blob([new Uint8Array([137, 80])], { type: "image/png" }));
  assert.equal((await fetch(png)).headers.get("content-type"), "image/png");
  URL.revokeObjectURL(html); URL.revokeObjectURL(png);
});
