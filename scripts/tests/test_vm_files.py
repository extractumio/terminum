# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Files a Mac's Linux VM holds under the checkout (spec AC-102): which process is the VM, what counts, when the
Docker VM's caches are dropped. Stand-ins for lsof and docker: the machine's own VM is never touched."""
import contextlib
import importlib.util
import io
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("vm_files", Path(__file__).resolve().parents[1] / "vm_files.py")
vm_files = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(vm_files)

ROOT = Path("/Users/alex/terminum")
VM_PATH = ("/System/Library/Frameworks/Virtualization.framework/Versions/A/XPCServices/"
           "com.apple.Virtualization.VirtualMachine.xpc/Contents/MacOS/com.apple.Virtualization.VirtualMachine")
PS = f"  74738 {VM_PATH}\n  528 /usr/sbin/cfprefsd\n 900 /usr/bin/vim com.apple.Virtualization.VirtualMachine.txt\n"


def said(**kwargs):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        over = vm_files.report(root=ROOT, ps_text=PS, **kwargs)
    return over, out.getvalue()


class Counting(unittest.TestCase):
    def test_only_the_vm_process_is_looked_at(self):
        self.assertEqual(vm_files.vm_pids(PS), [74738])

    def test_files_under_the_checkout_count_a_deleted_one_too(self):
        lsof = "\n".join(["p74738", "fcwd", "n/", "f12", f"n{ROOT}/ui/node_modules/a.js", "f13",
                          f"n{ROOT}/.checks/term-212-linux/x", f"n{ROOT}-other/y", "n/Users/alex/Docker.raw"])
        self.assertEqual(vm_files.under(lsof, ROOT), 2)


class Report(unittest.TestCase):
    def test_under_the_limit_is_said_and_nothing_dropped(self):
        drops = []
        over, out = said(releasing=True, count=lambda pid, root: 24, drop=lambda: drops.append(1) or "")
        self.assertEqual((over, drops), (0, []))
        self.assertIn("the VM 74738 holds 24 files under the checkout", out)

    def test_a_plain_look_over_the_limit_only_says(self):
        drops = []
        over, out = said(releasing=False, count=lambda pid, root: 156_000, drop=lambda: drops.append(1) or "")
        self.assertEqual((over, drops), (1, []))
        self.assertIn("holds 156000 files under the checkout (over 10000): make leftovers END=1 drops its caches", out)

    def test_release_over_the_limit_drops_and_counts_again(self):
        counts = iter([156_000, 24])
        over, out = said(releasing=True, count=lambda pid, root: next(counts), drop=lambda: "")
        self.assertEqual(over, 0)
        self.assertIn("the VM 74738 held 156000 files under the checkout, 24 now", out)

    def test_a_count_lsof_cannot_make_is_released_too(self):
        counts = iter([None, 30])
        over, out = said(releasing=True, count=lambda pid, root: next(counts), drop=lambda: "")
        self.assertEqual(over, 0)
        self.assertIn("the VM 74738 held an unknown number of files under the checkout, 30 now", out)

    def test_a_failed_drop_is_said_and_stays_over(self):
        over, out = said(releasing=True, count=lambda pid, root: 20_000, drop=lambda: "docker: Docker does not answer")
        self.assertEqual(over, 1)
        self.assertIn("could not drop the Docker VM's caches: docker: Docker does not answer", out)
        self.assertIn("20000 now", out)

    def test_no_vm_says_nothing(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(vm_files.report(True, root=ROOT, ps_text="528 /usr/sbin/cfprefsd\n",
                                             count=lambda pid, root: 1 / 0, drop=lambda: 1 / 0), 0)
        self.assertEqual(out.getvalue(), "")


if __name__ == "__main__":
    unittest.main()
