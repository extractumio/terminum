# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Files Docker Desktop's VM holds (spec AC-102): which process is Docker's VM, what counts, when its caches are
dropped. Stand-ins for lsof and docker: the machine's own VM is never touched."""
import contextlib
import importlib.util
import io
from pathlib import Path
import subprocess
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location("vm_files", Path(__file__).resolve().parents[1] / "vm_files.py")
vm_files = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(vm_files)

ROOT = Path("/Users/alex/terminum")
VM_PATH = ("/System/Library/Frameworks/Virtualization.framework/Versions/A/XPCServices/"
           "com.apple.Virtualization.VirtualMachine.xpc/Contents/MacOS/com.apple.Virtualization.VirtualMachine")
PS = (f"  74738 1 2-03:04:05 {VM_PATH}\n  528 1 47-03:00:06 /usr/sbin/cfprefsd\n"
      "  900 600 01:02 /usr/bin/vim com.apple.Virtualization.VirtualMachine.txt\n"
      "  901 600 00:01 /usr/bin/grep -r /com.apple.Virtualization.VirtualMachine\n")
DISK = "n/Users/alex/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw"


def docker_vm(under, elsewhere=0):
    return "\n".join(["p74738", "fcwd", "n/", "f6", DISK] + [f"n{ROOT}/ui/node_modules/m{i}.js" for i in range(under)]
                     + [f"n/private/tmp/tm-x/f{i}" for i in range(elsewhere)]) + "\n"


def run(listings, ps=PS, why="", **kwargs):
    """report() with lsof answering `listings` in turn; (its return, what it printed, the drops)."""
    answers, drops, out = iter(listings), [], io.StringIO()

    def list_files(pid, root):
        text = next(answers)
        return None if text is None else vm_files.counts(text, root)
    with contextlib.redirect_stdout(out):
        code = vm_files.report(root=ROOT, ps_text=ps, list_files=list_files,
                               drop=lambda pid: drops.append(pid) or why, **kwargs)
    return code, out.getvalue(), drops


class Counting(unittest.TestCase):
    def test_only_the_vm_process_is_looked_at(self):
        self.assertEqual(vm_files.vm_pids(PS), [74738])

    def test_every_file_counts_and_those_under_the_checkout_are_said(self):
        text = docker_vm(2, elsewhere=3) + f"n{ROOT}-other/y\n"
        self.assertEqual(vm_files.counts(text, ROOT), (True, 8, 2))
        self.assertEqual(vm_files.counts(text, None), (True, 8, None))

    def test_a_listing_lsof_stops_short_is_unknown(self):
        cut = subprocess.CompletedProcess([], 1, stdout=docker_vm(10)[:40], stderr="lsof: Too many open files in system")
        with mock.patch.object(vm_files.subprocess, "run", return_value=cut):
            self.assertIsNone(vm_files.listing(74738, ROOT))


class Report(unittest.TestCase):
    def test_under_the_limit_is_said_and_nothing_dropped(self):
        code, out, drops = run([docker_vm(24)], releasing=True)
        self.assertEqual((code, drops), (0, []))
        self.assertIn("Docker's VM 74738 holds 26 files, 24 under the checkout\n", out)

    def test_a_plain_look_over_the_limit_only_says(self):
        code, out, drops = run([docker_vm(156_000)], releasing=False)
        self.assertEqual((code, drops), (1, []))
        self.assertIn("holds 156002 files, 156000 under the checkout (over 10000): make leftovers END=1 drops its caches", out)

    def test_files_outside_the_checkout_count_toward_the_limit(self):
        code, _, drops = run([docker_vm(0, elsewhere=12_000), docker_vm(0)], releasing=True)
        self.assertEqual((code, drops), (0, [74738]))

    def test_release_over_the_limit_drops_once_and_counts_again(self):
        code, out, drops = run([docker_vm(156_000), docker_vm(24)], releasing=True)
        self.assertEqual((code, drops), (0, [74738]))
        self.assertIn("Docker's VM 74738 held 156002 files, 156000 under the checkout; 26 files, 24 under the checkout now", out)

    def test_a_sessions_own_release_drops_under_the_limit_too(self):
        code, _, drops = run([docker_vm(5_000), docker_vm(24)], releasing=True, always=True)
        self.assertEqual((code, drops), (0, [74738]))

    def test_an_uncountable_vm_is_released(self):
        code, out, drops = run([None, docker_vm(30)], releasing=True)
        self.assertEqual((code, drops), (0, [74738]))
        self.assertIn("held an unknown number of files (lsof could not list them); 32 files, 30 under the checkout now", out)

    def test_a_failed_drop_is_said_and_stays_over(self):
        code, out, _ = run([docker_vm(20_000), docker_vm(20_000)], releasing=True, why="docker run: Cannot connect")
        self.assertEqual(code, 1)
        self.assertIn("could not drop the Docker VM's caches: docker run: Cannot connect", out)

    def test_another_vm_is_never_counted_or_dropped(self):
        other = "\n".join(["p75000", "n/Users/alex/.colima/default/disk"] + [f"n{ROOT}/f{i}" for i in range(20_000)])
        ps = PS + f"75000 1 00:10 {VM_PATH}\n"
        code, out, drops = run([docker_vm(24), other], ps=ps, releasing=True)
        self.assertEqual((code, drops), (0, []))
        self.assertIn("Docker's VM 74738 holds 26 files", out)
        code, out, drops = run([other], ps=f"75000 1 00:10 {VM_PATH}\n", releasing=True)
        self.assertEqual((code, out, drops), (0, "", []))

    def test_no_vm_says_nothing(self):
        self.assertEqual(run([], ps="528 1 00:10 /usr/sbin/cfprefsd\n", releasing=True), (0, "", []))

    def test_the_drop_needs_no_host_namespace(self):
        done = subprocess.CompletedProcess([], 0, "", "")
        with mock.patch.object(vm_files.subprocess, "run", return_value=done) as called:
            self.assertEqual(vm_files.release(74738), "")
        command = called.call_args.args[0]
        self.assertEqual(command[:8], ["docker", "--context", "desktop-linux", "run", "--rm", "--privileged", "--name", command[7]])
        self.assertNotIn("--pid=host", command)
        self.assertEqual(command[-3:], ["sh", "-c", "sync; echo 2 > /proc/sys/vm/drop_caches"])

    def test_a_drop_that_hangs_removes_its_container(self):
        calls = []

        def fake(command, **kwargs):
            calls.append(command)
            if "run" in command:
                raise subprocess.TimeoutExpired(command, 180)
            return subprocess.CompletedProcess(command, 0, "", "")
        with mock.patch.object(vm_files.subprocess, "run", side_effect=fake):
            self.assertEqual(vm_files.release(74738), "docker run did not end in 180 s")
        self.assertEqual(calls[1][3:5], ["rm", "-f"])
        self.assertEqual(calls[1][5], calls[0][calls[0].index("--name") + 1])


if __name__ == "__main__":
    unittest.main()
