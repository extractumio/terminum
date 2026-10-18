// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A folder's color (AC-01): one hue per folder, apart for unrelated folders, near for nested ones.
import assert from "node:assert/strict";
import test from "node:test";
import { folderHues, folderKey } from "../static/dircolors.js";

const key = (dir, host = null) => folderKey({ sub: ["zsh", dir], host });
const gap = (a, b) => { const d = Math.abs(a - b) % 360; return Math.min(d, 360 - d); };

test("rows in one folder share its hue; the folder is keyed with its host", () => {
  assert.equal(key("~/p/"), key("~/p"));
  assert.notEqual(key("~/p"), key("~/p", "devbox"));
  assert.equal(folderKey({ sub: ["zsh"], host: null }), null);
  const h = folderHues([key("~/a"), key("~/a"), key("~/b")]);
  assert.equal(h.size, 2);
});

test("folders not inside one another share the circle evenly", () => {
  const h = folderHues([key("~/a"), key("~/b"), key("~/c"), key("/srv/d")]);
  const all = [...h.values()].sort((x, y) => x - y);
  for (let i = 0; i < all.length; i++) assert.ok(Math.abs(gap(all[i], all[(i + 1) % all.length]) - 90) < 1e-9, String(all));
});

test("~ and / are nobody's parent", () => {
  const h = folderHues([key("~"), key("~/a"), key("/"), key("/srv")]);
  const all = [...h.values()].sort((x, y) => x - y);
  for (let i = 0; i < all.length; i++) assert.ok(Math.abs(gap(all[i], all[(i + 1) % all.length]) - 90) < 1e-9, String(all));
});

test("a folder inside another is near its hue, nearer the deeper it lies, and still its own", () => {
  const [a, b, kid, deep, kid2] = [key("~/a"), key("~/b"), key("~/a/x"), key("~/a/y/z/w"), key("~/a/v")];
  const h = folderHues([a, b, kid, deep, kid2]);
  assert.equal(new Set(h.values()).size, 5);
  assert.ok(gap(h.get(a), h.get(kid)) < gap(h.get(a), h.get(b)) / 2);
  assert.ok(gap(h.get(a), h.get(deep)) < gap(h.get(a), h.get(kid)));
  assert.ok(gap(h.get(kid), h.get(kid2)) > 0);
});

test("a host's folders are apart from this Mac's same paths", () => {
  const h = folderHues([key("~/a"), key("~/a/x"), key("~/a", "devbox")]);
  assert.ok(gap(h.get(key("~/a")), h.get(key("~/a", "devbox"))) >= 179);
});
