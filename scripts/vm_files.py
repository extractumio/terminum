#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Files Docker Desktop's VM holds open on a Mac (spec AC-102). The VM shares the Mac's folders over VirtioFS and
keeps every file a container read through a bind mount open until the VM's kernel forgets it: the container's
end and the folder's deletion free nothing, and a VM up for days can fill the Mac's file table (ENFILE).
Dropping the VM's dentry and inode caches frees them; its containers keep running.

  vm_files.py            say what Docker's VM holds
  vm_files.py --release  and drop its caches (after a bind mount of one's own)"""
import os
from pathlib import Path
import subprocess
import sys

MAX = 10_000
VM = "/com.apple.Virtualization.VirtualMachine"   # the end of the path of Virtualization.framework's VM process
DISK = "/com.docker.docker/Data/vms/"              # the disk only Docker Desktop's VM holds open
IMAGE = "alpine:3.20"                               # the image the Docker checks already use
DROP = "sync; echo 2 > /proc/sys/vm/drop_caches"   # a privileged container may write /proc/sys


def checkout():
    """The primary checkout, which holds every worktree, its toolchain and its checks' folders; None outside one."""
    try:
        common = subprocess.run(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"], capture_output=True,
                                text=True, check=True, cwd=Path(__file__).resolve().parent).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None
    return Path(common).parent


def vm_pids(ps_text):
    """Pids of Virtualization.framework's VM processes in `ps -axo pid=,comm=`."""
    pids = []
    for line in ps_text.splitlines():
        pid, _, command = line.strip().partition(" ")
        if pid.isdigit() and command.strip().endswith(VM):
            pids.append(int(pid))
    return pids


def listing(pid):
    """`lsof -Fn` of `pid`; None unless lsof lists it all (it dies, or stops short, when the file table is full)."""
    try:
        run = subprocess.run(["lsof", "-n", "-p", str(pid), "-Fn"], capture_output=True, text=True,
                             errors="replace", timeout=120)
    except (OSError, subprocess.SubprocessError):
        return None
    return run.stdout if run.returncode == 0 and run.stdout else None


def counts(text, root):
    """(files, of them under `root`) in a listing; a deleted file keeps its name."""
    names = [line[1:] for line in text.splitlines() if line.startswith("n/")]
    return len(names), sum(name.startswith(f"{root}/") for name in names) if root else None


def release(pid):
    """Drop Docker's VM's caches; '' when done, else why not."""
    name = f"tm-vm-drop-{os.getpid()}"
    docker = ["docker", "--context", "desktop-linux"]
    try:
        run = subprocess.run([*docker, "run", "--rm", "--privileged", "--name", name, IMAGE, "sh", "-c", DROP],
                             capture_output=True, text=True, timeout=180)
    except subprocess.TimeoutExpired:
        subprocess.run([*docker, "rm", "-f", name], capture_output=True)
        return "docker run did not end in 180 s"
    except OSError as e:
        return f"docker: {e}"
    return "" if run.returncode == 0 else f"docker run: {(run.stderr.strip().splitlines() or [f'exit {run.returncode}'])[-1]}"


def said(text, root):
    if text is None:
        return "an unknown number of files (lsof could not list them)"
    files, under = counts(text, root)
    return f"{files} files" + ("" if under is None else f", {under} under the checkout")


def over(text):
    return text is None or counts(text, None)[0] > MAX


def report(releasing, always=False, root=None, ps_text=None, list_files=listing, drop=release):
    """Say what Docker's VM holds; with `releasing`, drop its caches when over MAX, uncountable, or `always`.
    Returns 1 while it stays over MAX or uncountable, else 0."""
    root = root or checkout()
    if ps_text is None:
        ps_text = subprocess.run(["ps", "-axo", "pid=,comm="], capture_output=True, text=True).stdout
    found = [(pid, list_files(pid)) for pid in vm_pids(ps_text)]
    # Docker's is the one with its disk open; with none known (lsof could not list), each uncountable one may be
    docker = [(pid, text) for pid, text in found if text is not None and DISK in text] \
        or [(pid, text) for pid, text in found if text is None]
    if not docker:
        return 0
    pid, text = docker[0]
    if not (releasing and (always or over(text))):
        hint = f" (over {MAX}): make leftovers END=1 drops its caches" if over(text) else ""
        print(f"Docker's VM {pid} holds {said(text, root)}{hint}")
        return int(over(text))
    why = drop(pid)
    if why:
        print(f"could not drop the Docker VM's caches: {why}")
    after = list_files(pid)
    print(f"Docker's VM {pid} held {said(text, root)}; {said(after, root)} now")
    return int(over(after))


def main(argv):
    if argv not in ([], ["--release"]):
        sys.exit(__doc__.split("\n\n")[1])
    return report(bool(argv), always=True)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
