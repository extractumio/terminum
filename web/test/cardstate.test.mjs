// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// What a card says of its session's agent (terminum-kanban.md AC-136).
import { test } from "node:test";
import assert from "node:assert/strict";
import { cardState } from "../static/cardstate.js";

const task = (session, more = {}) => ({ status: "IN PROGRESS", session: { key: "k", closed: false, agent: "claude", ...session }, ...more });

test("AC-136 a stopped agent with its task open says so", () => {
  assert.deepEqual(cardState(task({ state: "done" })), { cls: "stopped", words: "stopped, task open", why: undefined });
  assert.equal(cardState(task({ state: "done", agent: null })).words, "no agent");
});

test("AC-136 its turn over while its background work runs is no stop", () => {
  const st = cardState(task({ state: "done", background: true }));
  assert.equal(st.cls, "background");
  assert.equal(st.words, "background work runs");
  assert.match(st.why, /goes on when that ends/);
  // with no agent left, nothing of it can go on
  assert.equal(cardState(task({ state: "done", background: true, agent: null })).words, "no agent");
});

test("AC-136 working, a finished task, a wait and a failure", () => {
  assert.equal(cardState(task({ state: "working" })), null);
  assert.equal(cardState(task({ state: "done", background: true }, { status: "DONE" })), null);
  assert.equal(cardState(task({ state: "waiting", why: "a permission" })).words, "waits for you");
  assert.equal(cardState(task({ state: "failed", why: "rate_limit" })).words, "failed: rate limit");
  assert.equal(cardState({ status: "IN PROGRESS", session: { key: "k", closed: true } }).words, "session closed");
  assert.equal(cardState({ status: "TODO" }), null);
});
