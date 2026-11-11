// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The focus ring's color: the accent while it shows against the background, else the text color;
// a link's: the theme's while it reads, else lighter on a dark background, darker on a light one.
import assert from "node:assert/strict";
import test from "node:test";
import { contrast, linkColor, ringColor } from "../static/contrast.js";

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

test("a link keeps the theme's color while it reads on the background", () => {
  assert.equal(linkColor("#1e1e1e", "#6ea8fe"), "#6ea8fe");
  assert.equal(linkColor("#ffffff", "#2f6fdf"), "#2f6fdf");
  assert.equal(linkColor("#1e1e1e", "rebeccapurple"), "rebeccapurple");   // not measurable: as the theme says
});

test("a link too dark for a dark theme is lightened, too light for a light one darkened, its hue kept", () => {
  const dark = linkColor("#322925", "#315f78");   // a theme whose link was made for a light background
  assert.ok(contrast("#322925", dark) >= 4.5, dark);
  assert.ok(contrast(dark, "#ffffff") < contrast("#315f78", "#ffffff"), `${dark} is lighter`);
  const light = linkColor("#ffffff", "#a0c8ff");
  assert.ok(contrast("#ffffff", light) >= 4.5, light);
  assert.ok(contrast(light, "#000000") < contrast("#a0c8ff", "#000000"), `${light} is darker`);
  const [r, , b] = [1, 3, 5].map((i) => parseInt(dark.slice(i, i + 2), 16));
  assert.ok(b > r, `${dark} stays blue`);
  assert.ok(contrast("#808080", linkColor("#808080", "#7f7f7f")) >= 4.5);   // a mid grey background: whichever way reads
});
