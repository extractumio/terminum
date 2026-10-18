// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The focus ring's color: the accent while it shows against the background, else the text color.
import assert from "node:assert/strict";
import test from "node:test";
import { contrast, ringColor } from "../static/contrast.js";

test("contrast is WCAG's ratio, for #rgb and #rrggbb", () => {
  assert.equal(Math.round(contrast("#000", "#ffffff")), 21);
  assert.equal(contrast("#777777", "#777"), 1);
  assert.equal(contrast("rgb(0,0,0)", "#fff"), null);
});

test("the ring is the accent when it shows, the text color when it hides", () => {
  assert.equal(ringColor("#1e1e1e", "#ffffff", "#d4d4d4"), "#ffffff");
  assert.equal(ringColor("#1e1e1e", "#2a2a2a", "#d4d4d4"), "#d4d4d4");   // a cursor near the background
  assert.equal(ringColor("#ffffff", "#fdfdfd", "#1f2328"), "#1f2328");
  assert.equal(ringColor("#1e1e1e", "rebeccapurple", "#d4d4d4"), "rebeccapurple");   // not measurable: as the theme says
});
