#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The full run's one retry, kept honest (spec AC-201): once every step had its first run, a red step of real programs
(an e2e script, panel, live, docker, page) runs once more, unchanged: page its red checks alone, by name, the others
whole. A check gets no retry when its code changed since the newest full pass (check_changed's plan for those files
names it, its suite's files changed, or its red lines name a case they touched), nor when it was retried in 3 of the
last 7 recorded runs that ran it; a leftover never retries; red twice is red. A crash of a program the checks drive
(a crash report, tmux's server gone) is said as such. What passed only on its retry is said in red last, kept in the
step's pass so a release reusing it names it in its notes, and printed as the text of a task the run's agent makes or
comments on."""
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_changed import changed_files, plan, red, tail, touched_cases  # noqa: E402
import gate_record  # noqa: E402
from worktree import git  # noqa: E402

# the suites of real programs, and the files that are their code beside what check_changed's plan names
SUITES = {"panel": ("ui/test/e2e_panel",), "live": ("web/test/e2e_live",), "page": (),
          "docker": ("scripts/e2e_remote_sshd", "scripts/check_docker")}
CRASH_REPORTS = Path.home() / "Library" / "Logs" / "DiagnosticReports"
CRASHERS = ("tmux", "node", "terminumd", "WebKit", "com.apple.WebKit", "Chromium", "chrome", "Google Chrome")
SERVER_GONE = re.compile(r"server exited unexpectedly|lost server")
AC = re.compile(r"\bAC-(\d{2,3})\b")


def retried(step):
    return step in SUITES or step.startswith("e2e_")


def lines(log):
    """Every line a step's bounded log kept, oldest first: what a step says early counts too."""
    found = []
    for part in (Path(f"{log}.2"), Path(f"{log}.1"), Path(log)):
        try:
            found += part.read_text(errors="replace").splitlines()
        except OSError:
            pass
    return found


def red_checks(step, said):
    """The checks a red step names: page's (e2e_all.mjs's "red checks:" line); else the step itself."""
    if step == "page":
        named = next((line.split(":", 1)[1].split() for line in reversed(said) if line.startswith("red checks:")), [])
        if named:
            return named
    return [step]


def named(step, check):
    """A retried check's name in the record, the notes and its task: the step, and the check when it is one of the step's."""
    return step if check == step else f"{step} {check}"


def crash(began, said, reports=CRASH_REPORTS):
    """What crashed while the step ran, by file names only, or None."""
    names = []
    try:
        names = sorted(p.name for p in reports.iterdir() if p.name.startswith(CRASHERS) and p.stat().st_mtime >= began)
    except OSError:
        pass
    if any(SERVER_GONE.search(line) for line in said):
        names.insert(0, "tmux's server gone")
    return ", ".join(names) or None


class Changes:
    """What changed since the newest full pass (else the newest v* tag): its files, the cases they touch, and the
    targeted plan's step names and page checks for them (check_changed.plan, AC-104)."""
    def __init__(self, root):
        found = gate_record.newest_full_pass(root, "HEAD")
        self.base = found[0] if found else git("describe", "--tags", "--abbrev=0", "--match", "v*", cwd=root, check=False).stdout.strip()
        self.files, self.cases, self.names, self.pages = [], set(), set(), set()
        if not self.base:
            return
        self.files, merge_base = changed_files(self.base, cwd=root)
        self.cases = touched_cases(self.files, merge_base, cwd=root)
        steps = plan(self.files, self.cases, root)
        self.names = {name for name, _ in steps}
        self.pages = {w for name, command in steps if name == "page" for w in command.split()[2:]}

    def of(self, step, check, said):
        """Why `check` is a check of what changed, or None."""
        if not self.base:
            return None
        since = f"since {self.base[:9]}'s full pass"
        suite = SUITES.get(step) or ()
        if (step in self.names if step != "page" else check in self.pages) or suite and any(f.startswith(suite) for f in self.files):
            return f"its code changed {since}"
        cases = {c for line in said if line.startswith("FAIL") for c in AC.findall(line)} & self.cases
        return f"AC-{min(cases)} changed {since}" if cases else None


class Retry:
    def __init__(self, root):
        self.root, self.changes, self.runs = root, None, None
        self.passed = []   # (check, the task's text)

    def why_not(self, step, check, said):
        if self.runs is None:
            self.runs = gate_record.read(self.root, "gate")
        if gate_record.retried_lately(self.runs, named(step, check)):
            return f"retried in {gate_record.LOST_AFTER} of the last {gate_record.OF_LAST} runs"
        if self.changes is None:
            self.changes = Changes(self.root)
        return self.changes.of(step, check, said)

    def decide(self, step, command, log, began):
        """(the retry's command or None, the checks, the step's record entry), said as it goes."""
        said, entry = lines(log), {}
        if broke := crash(began, said):
            entry["crash"] = broke
            print(red(f"crash: {broke}, while {step} ran: an external program, said as such"), flush=True)
        if not retried(step):
            entry["cause"] = "failed"
            return None, [], entry
        checks = red_checks(step, said)
        if refused := "; ".join(f"{check}: {why}" for check in checks if (why := self.why_not(step, check, said))):
            entry["cause"] = f"failed, no retry: {refused}"
            print(f"note {step}: no retry: {refused}", flush=True)
            return None, checks, entry
        print(f"note {step} failed ({' '.join(checks)}), run once more unchanged; the first run's last lines:", flush=True)
        print(tail(log), end="", flush=True)
        entry["cause"] = "failed twice"
        return command if checks == [step] else "node web/test/e2e_all.mjs " + " ".join(checks), checks, entry

    def passed_on_retry(self, step, checks, first):
        for name in (named(step, check) for check in checks):
            self.passed.append((name, f"TASK for this run's agent: create_task, or add_comment on the open "
                                f"task titled \"Bug: flaky in the full run: {name}\" (P2), with the first run's "
                                f"last lines:\n{tail(first)}"))

    def say_last(self):
        """The run's last lines: each retried check, red, with the text of its task."""
        for _, task in self.passed:
            print(task, end="")
        if self.passed:
            print(red(f"PASSED ONLY ON RETRY, the first run red: {', '.join(c for c, _ in self.passed)}: name each with "
                      "its open task, here and in the release notes"))
