# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-102, AC-104: cancel a guarded step without interrupting its owned-child cleanup."""
import os
import shutil
import signal
import subprocess
import tempfile
import time

import owned_children

CANCEL_WAIT_S = 60  # leave another minute for recovery before land.py's 120 s escalation
SIGNALS = (signal.SIGTERM, signal.SIGHUP, signal.SIGINT)


class Cancellation:
    """Record the first signal; repeated signals never raise through cleanup."""
    def __init__(self):
        self.number = None
        self.previous = {}

    def __enter__(self):
        for number in SIGNALS:
            self.previous[number] = signal.signal(number, self.receive)
        return self

    def receive(self, number, _frame):
        if self.number is None:
            self.number = number

    def __exit__(self, kind, *_):
        for number, handler in self.previous.items():
            signal.signal(number, handler)
        if self.number and kind is None:
            raise SystemExit(128 + self.number)


def signal_guard(child, started, number):
    """Only our Popen child with its recorded start identity may receive a signal."""
    if started is not None and owned_children.identity(child.pid) == started:
        try:
            os.kill(child.pid, number)
        except ProcessLookupError:
            pass


def recover(child, started, folder):
    """End children before reaping the guard: its unreaped PID pins our private group against reuse."""
    import check_step

    signal_guard(child, started, signal.SIGSTOP)
    try:
        current = owned_children.identity(child.pid)
        group = child.pid if current is None or current == started else None
        pids = [pid for pid, _ in check_step.own_processes(folder, group) if pid != child.pid]
        expected = owned_children.cleanup_identities(pids, owned_children.path(folder))
        check_step.end_pids(list(expected), expected)
        signal_guard(child, started, signal.SIGKILL)
        remaining = [pid for pid, _ in check_step.own_processes(folder, group) if pid != child.pid]
        child.wait(timeout=check_step.KILL_AFTER_S)
        if remaining:
            raise RuntimeError(f"guard cleanup left owned processes; private root retained: {folder}")
    except BaseException:
        signal_guard(child, started, signal.SIGCONT)
        raise


def run_guard(guard, command, *, cwd, stdout):
    """Run one private guard, forward cancellation once, wait for cleanup, and never record a pass."""
    with Cancellation() as cancelled:
        folder = os.path.realpath(tempfile.mkdtemp(prefix="tmc.", dir="/tmp"))
        child = None
        cleaned = False
        try:
            child = subprocess.Popen([*guard, "--owned-root", folder, *command], cwd=cwd,
                                     stdout=stdout, stderr=subprocess.STDOUT, start_new_session=True)
            started = owned_children.identity(child.pid)
            deadline = None
            while True:
                if cancelled.number and deadline is None:
                    signal_guard(child, started, cancelled.number)
                    deadline = time.monotonic() + CANCEL_WAIT_S
                # WNOWAIT preserves the guard's PID even after exit, until owned-child recovery ends.
                ended = os.waitid(os.P_PID, child.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                if ended:
                    code = ended.si_status if ended.si_code == os.CLD_EXITED else -ended.si_status
                    break
                if deadline is not None and time.monotonic() >= deadline:
                    code = 128 + cancelled.number
                    break
                time.sleep(0.1)
            if code:
                recover(child, started, folder)
            else:
                child.wait()
            cleaned = True
            if cancelled.number:
                raise SystemExit(128 + cancelled.number)
            if code < 0:
                raise SystemExit(128 - code)
            return code
        finally:
            if cleaned or child is None:
                shutil.rmtree(folder)
            else:
                print(f"guard cleanup incomplete; private root retained: {folder}", flush=True)
