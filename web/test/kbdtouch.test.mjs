// What a finger's lift on the input is (AC-03, kbdtouch.js liftOf): a tap, the menu, a swipe, or nothing.
import { test } from "node:test";
import assert from "node:assert/strict";
import { liftOf } from "../static/kbdtouch.js";

const W = 390;
const at = (x, y, ms) => ({ x, y, at: ms });

test("a short still touch is a tap; held still half a second it is the menu; between, nothing", () => {
  assert.equal(liftOf(at(100, 500, 0), at(102, 501, 120), W), "tap");
  assert.equal(liftOf(at(100, 500, 0), at(101, 500, 600), W), "menu");
  assert.equal(liftOf(at(100, 500, 0), at(100, 500, 420), W), "hold");
});

test("a quick long swipe across is → to the right and Undo to the left", () => {
  assert.equal(liftOf(at(150, 500, 0), at(240, 510, 150), W), "swipe-right");
  assert.equal(liftOf(at(250, 500, 0), at(160, 495, 150), W), "swipe-left");
});

test("a slow, short, steep or edge swipe is no swipe", () => {
  assert.equal(liftOf({ ...at(150, 500, 0), scroll: 500 }, at(240, 510, 400), W), "");      // slow: a scroll
  assert.equal(liftOf({ ...at(150, 500, 0), scroll: 500 }, at(190, 500, 120), W), "");      // 40 px
  assert.equal(liftOf({ ...at(150, 500, 0), scroll: 500 }, at(210, 560, 150), W), "");      // as far down as across
  assert.equal(liftOf({ ...at(10, 500, 0), scroll: 500 }, at(110, 500, 150), W), "");       // from the screen's edge
  assert.equal(liftOf({ ...at(380, 500, 0), scroll: 500 }, at(280, 500, 150), W), "");
});

test("a slide is done already: its lift is nothing", () => {
  assert.equal(liftOf({ ...at(150, 500, 0), slide: true }, at(240, 500, 150), W), "");
});
