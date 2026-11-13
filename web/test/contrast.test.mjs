// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The accent's and a link's color: the theme's while it reads on the background, else lighter on
// a dark background, darker on a light one (the accent is also the focus ring, and a primary
// key's fill under words in the background's color).
import assert from "node:assert/strict";
import test from "node:test";
import { contrast, readable } from "../static/contrast.js";

test("contrast is WCAG's ratio, for #rgb and #rrggbb", () => {
  assert.equal(Math.round(contrast("#000", "#ffffff")), 21);
  assert.equal(contrast("#777777", "#777"), 1);
  assert.equal(contrast("rgb(0,0,0)", "#fff"), null);
});

test("an accent near the background (a light cursor on a light theme) reads, and so do words in the background's color on it", () => {
  for (const [bg, cursor] of [["#ffffff", "#ffffff"], ["#ffffff", "#fdfdfd"], ["#1e1e1e", "#2a2a2a"], ["#eff1f5", "#dc8a78"], ["#faf9f5", "#c4625a"]]) {
    const accent = readable(bg, cursor);
    assert.ok(contrast(accent, bg) >= 4.5, `${cursor} on ${bg}: ${accent}`);
  }
  assert.equal(readable("#1e1e1e", "#ffffff"), "#ffffff");   // a cursor that reads stays the theme's
});

test("a link keeps the theme's color while it reads on the background", () => {
  assert.equal(readable("#1e1e1e", "#6ea8fe"), "#6ea8fe");
  assert.equal(readable("#ffffff", "#2f6fdf"), "#2f6fdf");
  assert.equal(readable("#1e1e1e", "rebeccapurple"), "rebeccapurple");   // not measurable: as the theme says
});

test("a link too dark for a dark theme is lightened, too light for a light one darkened, its hue kept", () => {
  const dark = readable("#322925", "#315f78");   // a theme whose link was made for a light background
  assert.ok(contrast("#322925", dark) >= 4.5, dark);
  assert.ok(contrast(dark, "#ffffff") < contrast("#315f78", "#ffffff"), `${dark} is lighter`);
  const light = readable("#ffffff", "#a0c8ff");
  assert.ok(contrast("#ffffff", light) >= 4.5, light);
  assert.ok(contrast(light, "#000000") < contrast("#a0c8ff", "#000000"), `${light} is darker`);
  const [r, , b] = [1, 3, 5].map((i) => parseInt(dark.slice(i, i + 2), 16));
  assert.ok(b > r, `${dark} stays blue`);
  assert.ok(contrast("#808080", readable("#808080", "#7f7f7f")) >= 4.5);   // a mid grey background: whichever way reads
});
