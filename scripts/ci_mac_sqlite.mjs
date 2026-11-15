// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-104: the binding used by web/test/boardkit.mjs, before any costly Mac checks.
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";

const store = new DatabaseSync(":memory:");
try {
  const statement = store.prepare("SELECT CASE WHEN ?1 IS NULL THEN 'no task' ELSE ?1 END AS value");
  assert.equal(statement.get("task").value, "task");
  assert.equal(statement.get(null).value, "no task");
  console.log("ci_mac: node:sqlite numbered parameters passed");
} finally { store.close(); }
