#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-104: run the scripts' unittest modules in at most three processes; retain each module's output.

  python3 scripts/test_scripts.py [--jobs 1|2|3]
Each module uses unittest discovery, in a fresh interpreter. Logs and structured results stay in
.checks/tm-scripts-<run>/; a missing result, zero tests or a failing module makes the step fail.
The existing check_step guard owns the whole pool, including cancellation and detached children.
"""
import argparse
from contextlib import ExitStack
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS))
from check_cancel import run_guard  # noqa: E402
from check_logs import bounded_log, tail  # noqa: E402

COUNTS = ("tests", "failures", "errors", "skipped", "expected_failures", "unexpected_successes")
MAX_JOBS = 3


def worker(start, name, reports):
    suite = unittest.defaultTestLoader.discover(str(start), pattern=name)
    result = unittest.TextTestRunner(verbosity=1).run(suite)
    summary = dict(zip(COUNTS, (result.testsRun, len(result.failures), len(result.errors),
                               len(result.skipped), len(result.expectedFailures), len(result.unexpectedSuccesses))))
    summary["successful"] = result.wasSuccessful() and result.testsRun > 0
    (reports / f"{name}.json").write_text(json.dumps(summary) + "\n")
    return 0 if summary["successful"] else 1


def pool(start, reports, jobs):
    with ExitStack() as outputs:
        pending = iter(sorted(p.name for p in start.glob("test*.py") if p.is_file()))
        active = {}
        completed = []
        totals = dict.fromkeys(COUNTS, 0)
        failed = []
        began = time.monotonic()
        while True:
            while len(active) < jobs:
                name = next(pending, None)
                if name is None:
                    break
                output = outputs.enter_context(ExitStack())
                log = output.enter_context(bounded_log(SCRIPTS.parent, reports / f"{name}.log"))
                child = subprocess.Popen([sys.executable, "-I", str(Path(__file__).resolve()),
                                          "--start-directory", str(start), "--reports", str(reports),
                                          "--module", name], stdout=log, stderr=subprocess.STDOUT)
                active[name] = (child, time.monotonic(), output)
            if not active:
                break
            for name, (child, began_module, output) in list(active.items()):
                code = child.poll()
                if code is None:
                    continue
                output.close()  # drain and reap this module's sink before reading its report
                elapsed = time.monotonic() - began_module
                completed.append((elapsed, name))
                del active[name]
                try:
                    summary = json.loads((reports / f"{name}.json").read_text())
                    counts = {key: summary[key] for key in COUNTS}
                    valid = all(type(value) is int and value >= 0 for value in counts.values())
                    passed = valid and counts["tests"] > 0 and summary["successful"] is True and code == 0
                    if valid:
                        for key, value in counts.items():
                            totals[key] += value
                except (OSError, ValueError, KeyError, TypeError):
                    passed = False
                    counts = {"tests": 0}
                print(f"{'ok  ' if passed else 'FAIL'} {name}: {counts['tests']} tests ({elapsed:.2f} s)", flush=True)
                if not passed:
                    failed.append(name)
                    print("\n".join(tail(reports / f"{name}.log")), flush=True)
            if active:
                time.sleep(0.05)
        if not completed or totals["tests"] == totals["skipped"]:
            failed.append("no tests ran")
        print(f"scripts: {totals['tests']} tests, {totals['failures']} failures, {totals['errors']} errors, "
              f"{totals['skipped']} skipped, {totals['expected_failures']} expected failures, "
              f"{totals['unexpected_successes']} unexpected successes ({time.monotonic() - began:.2f} s)", flush=True)
        print("slowest modules: " + ", ".join(f"{name} {elapsed:.2f} s" for elapsed, name in sorted(completed, reverse=True)[:5]), flush=True)
        print(f"module output: {reports}", flush=True)
        print("scripts: FAIL " + ", ".join(failed) if failed else "scripts: OK", flush=True)
        return 1 if failed else 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jobs", type=int, choices=range(1, MAX_JOBS + 1), default=min(MAX_JOBS, os.cpu_count() or 1))
    parser.add_argument("--start-directory", type=Path, default=SCRIPTS / "tests")
    parser.add_argument("--log-root", type=Path, default=SCRIPTS.parent / ".checks")
    parser.add_argument("--pool", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--module", help=argparse.SUPPRESS)
    parser.add_argument("--reports", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    start = args.start_directory.resolve()
    if args.module:
        return worker(start, args.module, args.reports)
    if args.pool:
        return pool(start, args.reports, args.jobs)
    args.log_root.mkdir(parents=True, exist_ok=True)
    reports = Path(tempfile.mkdtemp(prefix="tm-scripts-", dir=args.log_root)).resolve()
    if os.environ.get("TM_TEST_CHILDREN_FILE"):
        return pool(start, reports, args.jobs)
    return run_guard([sys.executable, "-I", str(SCRIPTS / "check_step.py")],
                     [sys.executable, "-I", str(Path(__file__).resolve()), "--pool", "--jobs", str(args.jobs),
                      "--start-directory", str(start), "--reports", str(reports)], cwd=Path.cwd(), stdout=None)


if __name__ == "__main__":
    sys.exit(main())
