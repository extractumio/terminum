// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { test } from "node:test";
import { stopChild } from "./process-stop.mjs";

async function child(onTerm) {
  const process = spawn(globalThis.process.execPath, ["-e", `process.on('SIGTERM',()=>{${onTerm}});console.log('ready');setInterval(()=>{},1000)`], { stdio: ["ignore", "pipe", "pipe"] });
  await once(process.stdout, "data");
  return process;
}

test("shutdown waits for graceful exit before cleanup can continue", async () => {
  const process = await child("setTimeout(()=>process.exit(0),100)");
  try {
    const at = performance.now();
    await stopChild(process);
    assert.equal(process.exitCode, 0);
    assert.ok(performance.now() - at >= 80);
  } finally { await stopChild(process, 10); }
});

test("shutdown bounds a child ignoring SIGTERM and accepts an already signaled exit", async () => {
  const process = await child("");
  try {
    await stopChild(process, 50);
    assert.equal(process.signalCode, "SIGKILL");
    await stopChild(process, 10);
  } finally { await stopChild(process, 10); }
});
