#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The full run's one retry, kept honest (spec AC-201): once every step had its first run, a red step of real programs
(RETRIED) runs its red checks once more, unchanged: the red scripts of e2e, the red checks of page by name, the whole
suite of panel, live and docker. A check gets no retry when its code changed since the newest full pass (check_changed's
plan for those files names it, or its red lines name a case they touched), nor when it was retried in 3 of the last
7 recorded runs; a leftover never retries; red twice is red. A crash of a program the checks drive (a crash report, tmux's
server gone) is said as such. What passed only on its retry is said in red last, kept in the step's pass so a release
reusing it names it in its notes, and printed as the text of a task the run's agent makes or comments on."""
from pathlib import Path
import re
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_changed import changed_files, plan, red, touched_cases  # noqa: E402
from check_logs import tail  # noqa: E402
import gate_record  # noqa: E402

RETRIED = ("panel", "live", "e2e", "docker", "page")
CRASH_REPORTS = Path.home() / "Library" / "Logs" / "DiagnosticReports"
CRASHERS = ("tmux", "node", "terminumd", "WebKit", "com.apple.WebKit", "Chromium", "chrome", "Google Chrome")
SERVER_GONE = re.compile(r"server exited unexpectedly|lost server")
RED_SCRIPT = re.compile(r"^FAIL (scripts/e2e_\w+\.py)$")
AC = re.compile(r"\bAC-(\d{2,3})\b")


def e2e_command(scripts):
    """The e2e step over `scripts`: each runs, a red one said as "FAIL <script>", which its retry reads."""
    loop = " ".join(f"'{s}'" for s in scripts)
    return f'rc=0; for f in {loop}; do python3 "$f" || {{ echo "FAIL $f"; rc=1; }}; done; exit $rc'


def red_checks(step, log):
    """The checks a red step names: e2e's scripts, page's checks (e2e_all.mjs's "red checks:" line); else its suite."""
    lines = tail(log, 400)
    if step == "e2e":
        found = [m.group(1) for line in lines if (m := RED_SCRIPT.match(line.strip()))]
    elif step == "page":
        found = next((line.split(":", 1)[1].split() for line in reversed(lines) if line.startswith("red checks:")), [])
    else:
        found = []
    return found or [step]


def again(step, command, checks):
    """The retry's command: only the red checks where the step can name them."""
    if checks == [step]:
        return command
    if step == "e2e":
        return e2e_command(checks)
    return "node web/test/e2e_all.mjs " + " ".join(checks)


def crash(began, log, reports=CRASH_REPORTS):
    """What crashed while the step ran, by file names only, or None."""
    names = []
    try:
        names = sorted(p.name for p in reports.iterdir() if p.name.startswith(CRASHERS) and p.stat().st_mtime >= began)
    except OSError:
        pass
    if any(SERVER_GONE.search(line) for line in tail(log, 400)):
        names.insert(0, "tmux's server gone")
    return ", ".join(names) or None


def git(root, *args):
    return subprocess.run(["git", *args], cwd=root, capture_output=True, text=True).stdout.strip()


class Changes:
    """What changed since the newest full pass (else the newest v* tag): its files, the cases they touch, and the
    targeted plan's step names and page checks for them (check_changed.plan, AC-104)."""
    def __init__(self, root):
        found = gate_record.newest_full_pass(root, "HEAD")
        self.base = found[0] if found else git(root, "describe", "--tags", "--abbrev=0", "--match", "v*")
        self.files, self.cases, self.names, self.pages = [], set(), set(), set()
        if not self.base:
            return
        self.files, merge_base = changed_files(self.base, cwd=root)
        self.cases = touched_cases(self.files, merge_base, cwd=root)
        steps = plan(self.files, self.cases, root)
        self.names = {name for name, _ in steps}
        self.pages = {w for name, command in steps if name == "page" for w in command.split()[2:]}

    def of(self, step, check, log):
        """Why `check` is a check of what changed, or None."""
        if not self.base:
            return None
        since = f"since {self.base[:9]}'s full pass"
        named = (step == "e2e" and Path(check).stem in self.names or step == "page" and check in self.pages
                 or step == "panel" and "panel" in self.names
                 or step == "live" and any(f.startswith("web/test/e2e_live") for f in self.files)
                 or step == "docker" and any(f.startswith(("scripts/e2e_remote_sshd", "scripts/check_docker")) for f in self.files))
        if named:
            return f"its code changed {since}"
        cases = {c for line in tail(log, 400) if line.startswith("FAIL") for c in AC.findall(line)} & self.cases
        return f"AC-{min(cases)} changed {since}" if cases else None


class Retry:
    def __init__(self, root, ident):
        self.root, self.ident = root, ident
        self.changes = None
        self.passed, self.said = [], []

    def why_not(self, step, check, log):
        if gate_record.retried_lately(self.root, f"{step} {check}"):
            return f"retried in {gate_record.LOST_AFTER} of the last {gate_record.OF_LAST} runs"
        if self.changes is None:
            self.changes = Changes(self.root)
        return self.changes.of(step, check, log)

    def decide(self, step, command, log, began):
        """(the retry's command or None, the checks, the step's record entry), said as it goes."""
        entry = {}
        if broke := crash(began, log):
            entry["crash"] = broke
            print(red(f"crash: {broke}, while {step} ran: an external program, said as such"), flush=True)
        if step not in RETRIED:
            entry["cause"] = "failed"
            return None, [], entry
        checks = red_checks(step, log)
        refused = {check: why for check in checks if (why := self.why_not(step, check, log))}
        if refused:
            entry["cause"] = "failed, no retry: " + "; ".join(f"{c}: {w}" for c, w in refused.items())
            print(f"note {step}: no retry: " + "; ".join(f"{c}: {w}" for c, w in refused.items()), flush=True)
            return None, checks, entry
        print(f"note {step} failed ({' '.join(checks)}), run once more unchanged; the first run's last lines:", flush=True)
        print("".join(f"     {line}\n" for line in tail(log)), end="", flush=True)
        entry["cause"] = "failed twice"
        return again(step, command, checks), checks, entry

    def passed_on_retry(self, step, checks, first):
        lines = "".join(f"    {line}\n" for line in tail(first))
        for check in checks:
            title = f"Bug: flaky in the full run: {step} {check}"
            self.passed.append(f"{step} {check}")
            self.said.append(f"TASK for this run's agent: create_task, or add_comment on the open task titled "
                             f"\"{title}\" (P2), with the first run's last lines:\n{lines}")

    def say_last(self):
        """The run's last lines: each retried check, red, with the text of its task."""
        for words in self.said:
            print(words, end="")
        if self.passed:
            print(red(f"PASSED ONLY ON RETRY, the first run red: {', '.join(self.passed)}: name each with its open task, "
                      "here and in the release notes"))


def whole_notes(root, ident, names):
    """The whole run's pass notes, from its steps': what was retried, a budget over, Docker's skip."""
    notes = {"retried": [], "timing": []}
    for name in names:
        step = gate_record.passed(root, ident, name) or {}
        notes["retried"] += step.get("retried", [])
        notes["timing"] += step.get("timing", [])
        if step.get("skip"):
            notes["skip"] = step["skip"]
    return {k: v for k, v in notes.items() if v}
