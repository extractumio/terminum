# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Run the actual benchmark with synthetic page messages and a deterministic clock."""
import contextlib
import importlib.util
import io
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
SPEC = importlib.util.spec_from_file_location("bench_echo", SCRIPTS / "bench_echo.py")
bench = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bench)


class Page:
    def __init__(self, missed):
        self.calls, self.missed, self.typed = 0, missed, ""

    def send(self, message):
        if message.get("k"):
            self.typed += message["data"]

    def until(self, predicate, timeout):
        self.calls += 1
        if self.calls == 1:
            return {"t": "layout", "groups": [{"items": [{"id": "synthetic"}]}]}
        if self.calls == 2:
            return {"t": "screen"}
        return None if self.calls - 2 in self.missed else {"t": "screen", "ch": [[0, {"r": [[self.typed]]}]]}

    def recv(self, timeout):
        raise TimeoutError


class EchoVerdict(unittest.TestCase):
    def run_benchmark(self, misses=(), durations=None):
        page, stopped = Page(set(misses)), []
        daemon = type("Daemon", (), {"start": lambda _: None, "page": lambda _: page, "cleanup": lambda _: stopped.append(True)})()
        durations = durations or [0.001] * 10
        times, now = [], 0
        for duration in durations:
            times.extend([now, now + duration]); now += duration + 0.1
        with patch.object(bench, "Terminumd", return_value=daemon), patch.object(bench.time, "sleep"), \
             patch.object(bench.time, "perf_counter", side_effect=times), patch.object(sys, "argv", ["bench", "--keys", "10"]), \
             contextlib.redirect_stdout(io.StringIO()) as output, self.assertRaises(SystemExit) as result:
            bench.main()
        self.assertEqual(stopped, [True])
        return result.exception.code, output.getvalue()

    def test_missing_echo_fails_even_when_measured_echoes_are_fast(self):
        code, output = self.run_benchmark(misses=[10])
        self.assertEqual(code, 1, output)
        self.assertIn("1 of 10 keys had no echo", output)

    def test_p95_of_ten_samples_includes_the_last_slow_sample(self):
        code, output = self.run_benchmark(durations=[0.001] * 9 + [0.1])
        self.assertEqual(code, 1, output)
        self.assertIn("p95 100.0", output)

    def test_all_fast_echoes_pass(self):
        self.assertEqual(self.run_benchmark()[0], 0)
