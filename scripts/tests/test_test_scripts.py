# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-104: real module subprocesses preserve counts, bound concurrency and turn failures red."""
import json
from contextlib import contextmanager
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from testrun import short_tmp

RUNNER = Path(__file__).resolve().parents[1] / "test_scripts.py"


def wait_for(probe, seconds=10):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if result := probe():
            return result
        time.sleep(0.02)
    raise AssertionError("module subprocess did not reach its condition")


class Modules(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix="tm-units-", dir=short_tmp())
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.tests = self.root / "tests"
        self.tests.mkdir()
        self.command = [sys.executable, "-I", str(RUNNER), "--start-directory", str(self.tests),
                        "--log-root", str(self.root / "logs")]

    def module(self, name, body):
        (self.tests / name).write_text(body)

    def run_modules(self, *args):
        return subprocess.run([*self.command, *args], capture_output=True, text=True, timeout=45)

    def test_counts_and_verdict_match_serial_discovery_and_output_is_kept(self):
        self.module("test_one.py", """import unittest
class Cases(unittest.TestCase):
    def test_ok(self): print('module output retained')
    @unittest.skip('fixture skip')
    def test_skip(self): pass
    @unittest.expectedFailure
    def test_expected_failure(self): self.fail('expected')
""")
        self.module("test_two.py", "import unittest\nclass Cases(unittest.TestCase):\n def test_other(self): pass\n")
        serial = subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", str(self.tests)],
                                capture_output=True, text=True, timeout=10)
        parallel = self.run_modules()
        self.assertEqual((serial.returncode, parallel.returncode), (0, 0), parallel.stdout)
        self.assertIn("Ran 4 tests", serial.stderr)
        self.assertIn("scripts: 4 tests, 0 failures, 0 errors, 1 skipped, 1 expected failures", parallel.stdout)
        self.assertIn("slowest modules:", parallel.stdout)
        logs = list((self.root / "logs").glob("tm-scripts-*"))
        self.assertEqual(len(logs), 1)
        self.assertIn("module output retained", (logs[0] / "test_one.py.log").read_text())
        self.assertEqual(json.loads((logs[0] / "test_two.py.json").read_text())["tests"], 1)

    def test_a_failing_import_crash_empty_module_or_unexpected_success_turns_the_step_red(self):
        self.module("test_good.py", "import unittest\nclass Cases(unittest.TestCase):\n def test_good(self): pass\n")
        for body in (
            "import unittest\nclass Cases(unittest.TestCase):\n def test_bad(self): self.fail('fixture failure')\n",
            "raise ImportError('fixture import failure')\n",
            "import os\nos._exit(0)\n",
            "# no tests\n",
            "import unittest\nclass Cases(unittest.TestCase):\n @unittest.expectedFailure\n def test_unexpected(self): pass\n",
        ):
            with self.subTest(body=body):
                self.module("test_bad.py", body)
                result = self.run_modules()
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn("FAIL test_bad.py", result.stdout)
                self.assertIn("ok   test_good.py: 1 tests", result.stdout)
                self.assertIn("scripts: FAIL test_bad.py", result.stdout)

    def test_an_empty_or_entirely_skipped_suite_is_red(self):
        self.assertEqual(self.run_modules().returncode, 1)
        self.module("test_skip.py", "import unittest\n@unittest.skip('fixture')\nclass Cases(unittest.TestCase):\n def test_skip(self): pass\n")
        self.assertEqual(self.run_modules().returncode, 1)

    def test_at_most_the_requested_two_modules_run_and_each_runs_once(self):
        release = self.root / "release"
        for letter in "abcd":
            ready = self.root / letter
            self.module(f"test_{letter}.py", f"""import pathlib, time, unittest
class Cases(unittest.TestCase):
    def test_wait(self):
        ready = pathlib.Path({str(ready)!r})
        self.assertFalse(ready.exists(), 'module ran twice')
        ready.touch()
        deadline = time.monotonic() + 10
        while not pathlib.Path({str(release)!r}).exists():
            self.assertLess(time.monotonic(), deadline, 'release never arrived')
            time.sleep(0.01)
""")
        with subprocess.Popen([*self.command, "--jobs", "2"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True) as child:
            try:
                wait_for(lambda: (self.root / "a").exists() and (self.root / "b").exists())
                self.assertFalse((self.root / "c").exists())
                self.assertFalse((self.root / "d").exists())
            finally:
                release.touch()
                output, _ = child.communicate(timeout=30)
        self.assertEqual(child.returncode, 0, output)
        self.assertIn("scripts: 4 tests", output)
        self.assertTrue(all((self.root / letter).exists() for letter in "abcd"))

    def test_jobs_outside_the_small_bound_are_refused(self):
        for count in ("0", "4", "many"):
            with self.subTest(count=count):
                self.assertEqual(self.run_modules("--jobs", count).returncode, 2)

    def test_noisy_module_output_rotates_without_changing_its_result(self):  # AC-196
        self.module("test_noisy.py", """import sys, unittest
class Cases(unittest.TestCase):
    def test_output(self):
        sys.stdout.write('module line\\n' * 1_000_000)
        print('last module record')
""")
        result = self.run_modules()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("scripts: 1 tests", result.stdout)
        reports, = (self.root / "logs").glob("tm-scripts-*")
        log = reports / "test_noisy.py.log"
        parts = [Path(str(log) + suffix) for suffix in (".2", ".1", "")]
        self.assertTrue(Path(str(log) + ".1").exists())
        self.assertTrue(all(p.stat().st_size <= 10 << 20 for p in parts if p.exists()))
        kept = b"".join(p.read_bytes() for p in parts if p.exists())
        self.assertEqual(kept.count(b"module line\n"), 1_000_000)
        self.assertIn(b"last module record\n", kept)

    def test_a_green_module_with_a_failed_output_sink_cannot_pass(self):  # AC-196
        import test_scripts
        self.module("test_good.py", "import unittest\nclass Cases(unittest.TestCase):\n def test_good(self): pass\n")
        reports = self.root / "reports"
        reports.mkdir()

        @contextmanager
        def failed_sink(*_):
            with open(os.devnull, "wb") as out:
                yield out
            raise RuntimeError("sink failed")

        with mock.patch.object(test_scripts, "bounded_log", failed_sink):
            with self.assertRaisesRegex(RuntimeError, "sink failed"):
                test_scripts.pool(self.tests, reports, 1)
        self.assertTrue(json.loads((reports / "test_good.py.json").read_text())["successful"])

    def test_cancelling_the_direct_runner_ends_its_module_process(self):
        ready = self.root / "ready"
        self.module("test_wait.py", f"""import os, pathlib, time, unittest
class Cases(unittest.TestCase):
    def test_wait(self):
        pathlib.Path({str(ready)!r}).write_text(str(os.getpid()))
        while True: time.sleep(0.01)
""")
        # Exercise the direct make-test path even when this test runs under the outer check guard.
        env = {key: value for key, value in os.environ.items() if key != "TM_TEST_CHILDREN_FILE"}
        child = subprocess.Popen(self.command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env)
        pid = None
        try:
            pid = int(wait_for(lambda: ready.read_text() if ready.exists() else ""))
            child.send_signal(signal.SIGTERM)
            output, _ = child.communicate(timeout=35)
            self.assertEqual(child.returncode, 143, output)
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, 0)
        finally:
            if child.poll() is None:
                child.send_signal(signal.SIGTERM)
                child.communicate(timeout=35)

    def test_make_and_targeted_checks_use_the_same_runner(self):
        import check_changed
        repo = RUNNER.parents[1]
        command = dict(check_changed.plan(["scripts/probe.py"], set(), tests_of_cases=[]))["scripts"]
        self.assertEqual(command, "python3 scripts/test_scripts.py")
        self.assertIn("\t" + command + "\n", (repo / "Makefile").read_text())


if __name__ == "__main__":
    unittest.main()
