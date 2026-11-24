#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Files a Mac's Linux VM holds open under the checkout (spec AC-102). Docker Desktop's VM shares the Mac's
folders over VirtioFS and keeps every file a container read through a bind mount open until the VM's kernel
forgets it: the container's end and the folder's deletion free nothing, and a VM up for days can fill the
Mac's file table (ENFILE). Dropping the VM's dentry and inode caches frees them; the containers keep running.

  vm_files.py            say what each VM holds under the checkout
  vm_files.py --release  and drop the Docker VM's caches when that is over MAX (or cannot be counted)"""
from pathlib import Path
import subprocess
import sys

MAX = 10_000
VM = "/com.apple.Virtualization.VirtualMachine"   # the end of the path of Virtualization.framework's VM process
IMAGE = "alpine:3.20"                               # the image the Docker checks already use
DROP = ["sync; echo 2 > /proc/sys/vm/drop_caches"]


def checkout():
    """The primary checkout, which holds every worktree, its toolchain and its checks' folders."""
    common = subprocess.run(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"], capture_output=True,
                            text=True, check=True, cwd=Path(__file__).resolve().parent).stdout.strip()
    return Path(common).parent


def vm_pids(ps_text):
    """Pids of the VM processes in `ps -axo pid=,comm=`."""
    pids = []
    for line in ps_text.splitlines():
        pid, _, command = line.strip().partition(" ")
        if pid.isdigit() and command.strip().endswith(VM):
            pids.append(int(pid))
    return pids


def under(lsof_text, root):
    """How many of the files in `lsof -Fn`'s output are under `root` (a deleted one keeps its name)."""
    prefix = f"n{root}/"
    return sum(line.startswith(prefix) for line in lsof_text.splitlines())


def held(pid, root):
    """Files `pid` holds under `root`; None if lsof cannot say (it dies when the file table is full)."""
    try:
        listed = subprocess.run(["lsof", "-n", "-p", str(pid), "-Fn"], capture_output=True, text=True,
                                errors="replace", timeout=120)
    except (OSError, subprocess.SubprocessError):
        return None
    return under(listed.stdout, root) if listed.returncode in (0, 1) and listed.stdout else None


def release():
    """Drop the Docker VM's caches; '' when done, else why not."""
    try:
        run = subprocess.run(["docker", "run", "--rm", "--privileged", "--pid=host", IMAGE,
                              "nsenter", "-t", "1", "-m", "--", "sh", "-c", *DROP],
                             capture_output=True, text=True, timeout=180)
    except (OSError, subprocess.SubprocessError) as e:
        return f"docker: {e}"
    return "" if run.returncode == 0 else f"docker run: {(run.stderr.strip().splitlines() or ['exit %d' % run.returncode])[-1]}"


def report(releasing, root=None, ps_text=None, count=held, drop=release):
    """Say each VM's count; with `releasing`, drop the caches of one over MAX. Returns how many stay over."""
    root = root or checkout()
    if ps_text is None:
        ps_text = subprocess.run(["ps", "-axo", "pid=,comm="], capture_output=True, text=True).stdout
    over = 0
    for pid in vm_pids(ps_text):
        files = count(pid, root)
        said = "an unknown number of" if files is None else files   # lsof dies when the file table is full
        if files is not None and files <= MAX:
            print(f"the VM {pid} holds {files} files under the checkout")
            continue
        if not releasing:
            print(f"the VM {pid} holds {said} files under the checkout (over {MAX}): make leftovers END=1 drops its caches")
            over += 1
            continue
        why = drop()
        after = count(pid, root)
        if why:
            print(f"could not drop the Docker VM's caches: {why}")
        print(f"the VM {pid} held {said} files under the checkout, {'an unknown number' if after is None else after} now")
        over += after is None or after > MAX
    return over


def main(argv):
    if argv not in ([], ["--release"]):
        sys.exit(__doc__.split("\n\n")[1])
    return 1 if report(bool(argv)) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
