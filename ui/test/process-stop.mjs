// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Stop only the child the harness owns, and wait before deleting its state or tmux.
export async function stopChild(child, grace = 5000) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  let timer, forced, ended, failed;
  const exit = new Promise((resolve, reject) => {
    ended = resolve; failed = reject;
    child.once("exit", ended); child.once("error", failed);
    timer = setTimeout(() => {
      child.kill("SIGKILL");
      forced = setTimeout(() => reject(Error("The private test daemon did not exit after SIGKILL")), 2000);
    }, grace);
    child.kill("SIGTERM");
  });
  try { await exit; }
  finally {
    clearTimeout(timer); clearTimeout(forced);
    child.off("exit", ended); child.off("error", failed);
  }
}
