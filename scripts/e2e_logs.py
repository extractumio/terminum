#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-196: real private daemon and host-agent logs rotate while both processes stay alive.
Uses production limits and the maintenance minute; no installed state, ssh or model calls.
"""
import os
from pathlib import Path
import select
import subprocess
import time

from e2e_ws import BIN, Terminumd, check, summary, wait_for
from testrun import free_port, own


def fill(path, size):
    with path.open("ab") as out:
        out.write(b"rotation-test\n" * (size // 14 + 1))


def main():
    t = Terminumd(free_port(), own("tm-test-logs"))
    agent = follower = None
    try:
        path = Path(t.log_path)
        path.parent.mkdir(parents=True)
        path.write_text("previous start\n")
        fill(path, 10 << 20)
        t.start()
        wait_for(lambda: path.read_text(), lambda s: 'event="start"' in s or 'event=start' in s)
        check("AC-196 daemon owns a fresh log after startup rotation", Path(str(path) + ".1").exists() and path.stat().st_size < 100_000)
        initial = subprocess.run([BIN, "logs", "-n", "3"], env=t.env, capture_output=True, text=True, check=True).stdout
        check("AC-196 terminum logs includes archived and active lines", "rotation-test" in initial and "start" in initial)

        root = Path(t.root)
        agent_log = root / "agent.log"
        env = {**t.env, "HOME": str(root / "agent-home")}
        (root / "agent-home").mkdir()
        agent = subprocess.Popen([BIN, "--agent", "--socket", str(root / "remote/sock"), "--log", str(agent_log)],
                                 env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        agent.stdin.write("rotation-test-authentication\n")
        agent.stdin.flush()
        ready, _, _ = select.select([agent.stdout], [], [], 10)
        assert ready and "ready" in agent.stdout.readline(), "private agent did not become ready"
        wait_for(agent_log.read_text, lambda s: "start" in s)

        # Make both active logs exceed their production limit without restarting the writers.
        fill(path, 10 << 20)
        fill(agent_log, 1 << 20)
        integration = Path(t.home) / "integration/test-project"
        integration.mkdir(parents=True)
        expired = integration / "batch-1.1.check.log"
        expired.write_text("expired\n")
        old = time.time() - 31 * 86_400
        os.utime(expired, (old, old))
        wait_for(lambda: (path.stat().st_size, agent_log.stat().st_size, expired.exists()),
                 lambda v: v[0] < 100_000 and v[1] < 100_000 and not v[2], seconds=75)
        check("AC-196 daemon and agent rotate while alive, idle integration expires",
              t.proc.poll() is None and agent.poll() is None and Path(str(agent_log) + ".1").exists())

        # Follow the actual command, then replace the inode by another rotation on restart.
        followed = root / "followed.txt"
        with followed.open("w") as out:
            follower = subprocess.Popen([BIN, "logs", "-n", "0", "-f"], env=t.env, stdout=out, stderr=subprocess.PIPE)
        time.sleep(0.3)
        t.stop()
        wait_for(followed.read_text, lambda s: "stop" in s)
        fill(path, 10 << 20)
        t.start()
        wait_for(followed.read_text, lambda s: "start" in s)
        check("AC-196 logs -f follows replacement and graceful stop flushes", "stop" in followed.read_text() and "start" in followed.read_text())
        agent.stdin.close()
        agent.wait(10)
        check("AC-196 live agent continues in its new file and flushes on exit", "stop" in agent_log.read_text())
        blocked = root / "blocked-log"
        blocked.mkdir()
        failed_dir = root / "failed-agent"
        failed = subprocess.run([BIN, "--agent", "--socket", str(failed_dir / "sock"), "--log", str(blocked)],
                                env=env, input="rotation-test-authentication\n", capture_output=True, text=True, timeout=10)
        check("AC-196 refused agent log leaves no private socket folder", failed.returncode != 0 and not failed_dir.exists() and "cannot open diagnostic log" in failed.stderr)
    finally:
        if follower is not None:
            if follower.poll() is None:
                follower.terminate()
            follower.communicate(timeout=10)
        if agent is not None:
            if agent.poll() is None:
                agent.terminate()
            agent.wait(10)
            agent.stdout.close()
            agent.stderr.close()
        t.cleanup()
    summary()


if __name__ == "__main__":
    main()
