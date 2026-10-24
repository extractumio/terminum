// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-03 against a real private terminumd (e2e_live.mjs): a long run of keys (a coding agent's
// input moved by lines, held) reaches a busy program whole: no read of it ends inside a key's
// bytes. As one run tmux handed it on in parts of 1022 bytes, ESC in one, "[D" in the next,
// and Claude Code typed "[D" into its input.
import fs from "node:fs";
import path from "node:path";
import { check } from "./check.mjs";

const KEYS = 1000;

export async function liveKeys(page, { tmux, work }) {
  const shown = tmux("list-panes", "-a", "-F", "#{pane_id} #{pane_current_path}").split("\n").map((l) => l.split(" ")).find((p) => p[1] === work)?.[0];
  const reader = path.join(work, "reader.py"), log = path.join(work, "reads.txt");
  // a program that reads raw keys and draws after each read, slowly (as an agent redraws its box)
  fs.writeFileSync(reader, [
    "import os, sys, termios, tty, time",
    "tty.setraw(0); total = cut = 0",
    `while total < ${KEYS * 3}:`,
    "    b = os.read(0, 4096); total += len(b); cut += b.endswith(b'\\x1b') or b.endswith(b'\\x1b[')",
    "    os.write(1, b'.'); time.sleep(0.02)",
    `    open(sys.argv[1], 'w').write(f'{total} {cut}')`,
  ].join("\n") + "\n");
  const pane = tmux("new-window", "-d", "-P", "-F", "#{pane_id}", "-n", "keys", "-c", work, `python3 -I ${reader} ${log}`);
  await page.waitForSelector(`#sessions .item[data-id$=":${pane}"]`, { timeout: 5000 }).catch(() => {});
  const id = await page.evaluate((pane) => document.querySelector(`#sessions .item[data-id$=":${pane}"]`)?.dataset.id, pane);
  // the run as the page sends it (boxnav.js stepKeys), on a socket of its own
  await page.evaluate(([id, data]) => new Promise((done) => {
    const ws = new WebSocket(`ws://${location.host}/ws`, ["tm", localStorage.getItem("tm.session")]);
    ws.onopen = () => { ws.send(JSON.stringify({ t: "sub", id })); setTimeout(() => { ws.send(JSON.stringify({ t: "in", data })); setTimeout(() => { ws.close(); done(); }, 300); }, 500); };
    ws.onerror = () => done();
  }), [id, "\x1b[D".repeat(KEYS)]);
  let got = "";
  for (let i = 0; i < 150 && !(got = fs.existsSync(log) ? fs.readFileSync(log, "utf8") : "").startsWith(`${KEYS * 3} `); i++) await new Promise((r) => setTimeout(r, 100));
  const [total, cut] = got.split(" ").map(Number);
  check("AC-03 a long run of keys reaches a busy program whole: no read ends inside a key (\"[D\" typed)", !!id && total === KEYS * 3 && cut === 0, JSON.stringify({ id, total, cut }));
  tmux("kill-window", "-t", pane);
  fs.rmSync(reader, { force: true }); fs.rmSync(log, { force: true });
  if (shown) await page.click(`#sessions .item[data-id$=":${shown}"]`).catch(() => {});
}
