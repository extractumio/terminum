// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// What a card says of its session's agent (terminum-kanban.md AC-136); a row read as at work (terminum-agents.md AC-39).
import { test } from "node:test";
import assert from "node:assert/strict";
import { atWork, cardState, liveSession, slotState } from "../static/cardstate.js";

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

test("AC-136 a live update of the session list keeps its background mark", () => {
  const row = { id: "tmux:local:terminum:%2", state: "done", agent: "claude", why: null, background: true, title: "t" };
  assert.deepEqual(liveSession(row), { key: row.id, state: "done", agent: "claude", why: null, background: true, closed: false });
  assert.equal(cardState({ status: "IN PROGRESS", session: liveSession(row) }).words, "background work runs");
  assert.equal(liveSession({ ...row, background: undefined }).background, false);
});

test("AC-39 a row is at work while its agent works or work it started runs past its turn, however long", () => {
  assert.equal(atWork({ state: "working" }), true);
  assert.equal(atWork({ state: "done", background: true }), true, "past the 10 minutes terminumd says done with the mark");
  assert.equal(atWork({ state: "done" }), false, "the work ended, the agent stopped");
  assert.equal(atWork({ state: "waiting", background: true }), false);
  assert.equal(atWork({ state: "failed" }), false);
  assert.equal(atWork(null), false, "no row");
});

test("AC-39 the board's dot reads the row as the bar and the card do: idle only with nothing running", () => {
  const slot = { idle: true, state: "working" };
  assert.equal(slotState(slot, { state: "done", background: true }), "working", "its turn over, its check running: it blinks");
  assert.equal(slotState(slot, { state: "working" }), "working");
  assert.equal(slotState(slot, { state: "done" }), "idle", "the work ended and the agent stopped: still");
  assert.equal(slotState(slot, null), "idle", "no row");
  assert.equal(slotState({ idle: false, state: "working" }, { state: "waiting" }), "waits");
  assert.equal(slotState({ idle: false, state: "typed" }, null), "starting");
  // the card says the same at each step
  const card = (row) => cardState({ status: "IN PROGRESS", session: liveSession({ id: "k", agent: "claude", ...row }) });
  assert.equal(card({ state: "working" }), null);
  assert.equal(card({ state: "done", background: true }).words, "background work runs");
  assert.equal(card({ state: "done" }).words, "stopped, task open");
});
