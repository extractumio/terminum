#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-251 and AC-252: real Verify, a marked review pane, then a same-commit retry."""
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile

from e2e_integration import HERE, World, tmux
from e2e_ws import check, summary, wait_for
from testrun import short_tmp


def main():
    with tempfile.TemporaryDirectory(prefix="tm-design-", dir=short_tmp()) as work:
        w = World(work)

        def call(path, method="GET", body=None):
            status, result = w.call(path, method, body)
            assert status == 200, (path, status, result)
            return result

        try:
            w.start()
            w.setup()
            path = f"/api/task-projects/{w.project}"
            supervisor = f"{path}/supervisor"
            call(f"{path}/integration", "PUT", {
                "on": True, "verify": shlex.join([w.cli, "integration", "verify-review"]),
            })
            log = Path(work, "review.jsonl")
            reviewer = call("/api/task-agents", "POST", {
                "project": w.project, "name": "Architect fixture", "kind": "claude", "at_once": 1,
                "command": shlex.join(["python3", "-I", str(Path(HERE, "fake_agent.py")), w.cli,
                                       "--kind", "claude", "--round-log", str(log)]),
            })["id"]
            settings = call(f"{supervisor}/settings")
            pack = settings["settings"]
            pack["tiers"]["strong"] = reviewer
            pack["charter"]["surfaces"]["t2"] = ["contract.txt"]
            call(f"{supervisor}/settings", "PUT", {"revision": settings["revision"], "settings": pack})
            call(supervisor, "PUT", {"enabled": True})
            task = w.task("contract")
            sha = w.queue(task, "contract")
            red = wait_for(lambda: w.history(task), lambda history: "returned: red check" in history, 45, last=True)
            check("AC-251 the CLI returns the unreviewed contract and leaves a review comment",
                  "T2 Verify needs an architecture review" in red and "status 73" in red, red)
            review = wait_for(lambda: w.history(task), lambda history: "Design review (architecture): approved." in history, 40, last=True)
            check("AC-252 the assigned critic leaves an attributed verdict on the affected task",
                  "Design review (architecture): approved." in review and "Clients still use it." in review, review)
            records = [json.loads(line) for line in log.read_text().splitlines()]
            approved = [entry for entry in records if entry.get("tool") == "record_design_review" and not entry.get("isError")]
            check("AC-252 one marked critic round writes the review", len(approved) == 1, approved)
            current = w.get(task)
            if not current.get("session"):
                call(f"/api/tasks/{task}/run", "POST", {"agent": w.agent})
            key = wait_for(lambda: (w.get(task).get("session") or {}).get("key"), bool, 30, last=True)
            pane = key.rsplit(":", 1)[1]
            env = {**w.t.env, "TMUX_PANE": pane, "TMUX": tmux("display", "-p", "#{socket_path},#{pid},0").strip()}
            retry = subprocess.run([w.cli, "integration", "queue", task, "--commit", sha,
                                    "--result-file", str(Path(work, "result-contract.md"))],
                                   env=env, capture_output=True, text=True, timeout=20)
            check("AC-251 the identical commit can be requeued after review", retry.returncode == 0, retry.stderr)
            if retry.returncode != 0:
                return summary()
            landed = wait_for(lambda: w.status(task), lambda status: status == "DONE", 80, last=True)
            check("AC-251 the reviewed commit passes the real Verify and lands", landed == "DONE", w.history(task))
            preview = call(f"{supervisor}/surfaces/preview", "POST", pack["charter"]["surfaces"])
            check("AC-250 the signed-in endpoint sizes the landed task from its own merge",
                  preview["sampled"] == 1 and preview["without_diff"] == 0 and preview["t2"] == 1, preview)
        finally:
            w.t.cleanup()
    return summary()


if __name__ == "__main__":
    raise SystemExit(main())
