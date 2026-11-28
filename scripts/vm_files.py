#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Files Docker Desktop's VM holds open on a Mac (spec AC-102). The VM shares the Mac's folders over VirtioFS and
keeps every file a container read through a bind mount open until the VM's kernel forgets it: the container's
end and the folder's deletion free nothing, and a VM up for days can fill the Mac's file table (ENFILE).
Dropping the VM's dentry and inode caches frees them; its containers keep running.

  vm_files.py            say what Docker's VM holds
  vm_files.py --release  and drop its caches (after a bind mount one could not avoid)"""
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from testrun import container_name  # noqa: E402
from worktree import common_dir, Refused  # noqa: E402

MAX = 10_000
VM = "/com.apple.Virtualization.VirtualMachine"   # the end of the path of Virtualization.framework's VM process
DISK = "/com.docker.docker/Data/vms/"              # the disk only Docker Desktop's VM holds open
IMAGE = "alpine:3.20"                               # the image the Docker checks already use
DROP = "sync; echo 2 > /proc/sys/vm/drop_caches"   # a privileged container may write /proc/sys
DOCKER = ["docker", "--context", "desktop-linux"]


def checkout():
    """The primary checkout, which holds every worktree, its toolchain and its checks' folders; None outside one."""
    try:
        return common_dir(Path(__file__).resolve().parent).parent
    except (OSError, Refused):
        return None


def vm_pids(ps_text):
    """Pids of Virtualization.framework's VM processes in leftovers.processes()'s `ps` (it runs with no arguments)."""
    rows = [line.split(None, 3) for line in ps_text.splitlines()]
    return [int(row[0]) for row in rows if len(row) == 4 and row[0].isdigit() and row[3].strip().endswith(VM)
            and " " not in row[3].strip()]


def counts(text, root):
    """(Docker's disk among them, files, of them under `root`) of an `lsof -Fn` listing; a deleted file keeps its name."""
    disk, files, under = False, 0, 0
    prefix = f"n{root}/" if root else None
    for line in text.splitlines():
        if line.startswith("n/"):
            files += 1
            disk = disk or DISK in line
            under += bool(prefix and line.startswith(prefix))
    return disk, files, under if root else None


def listing(pid, root):
    """counts() of what `pid` holds; None unless lsof lists it all (it dies, or stops short, when the table is full)."""
    try:
        run = subprocess.run(["lsof", "-n", "-p", str(pid), "-Fn"], capture_output=True, text=True,
                             errors="replace", timeout=120)
    except (OSError, subprocess.SubprocessError):
        return None
    return counts(run.stdout, root) if run.returncode == 0 and run.stdout else None


def release(pid):
    """Drop Docker's VM's caches; '' when done, else why not."""
    name = container_name("vm-drop")
    try:
        run = subprocess.run([*DOCKER, "run", "--rm", "--privileged", "--name", name, IMAGE, "sh", "-c", DROP],
                             capture_output=True, text=True, timeout=180)
    except subprocess.TimeoutExpired:
        subprocess.run([*DOCKER, "rm", "-f", name], capture_output=True)
        return "docker run did not end in 180 s"
    except OSError as e:
        return f"docker: {e}"
    if run.returncode == 0:
        return ""
    lines = run.stderr.strip().splitlines()
    return f"docker run: {lines[-1] if lines else f'exit {run.returncode}'}"


def said(held):
    """(what `held` says, whether it is over MAX or unknown)."""
    if held is None:
        return "an unknown number of files (lsof could not list them)", True
    _, files, under = held
    return f"{files} files" + ("" if under is None else f", {under} under the checkout"), files > MAX


def report(releasing, always=False, root=None, ps_text=None, list_files=listing, drop=release):
    """Say what Docker's VM holds; with `releasing`, drop its caches when over MAX, uncountable, or `always`.
    Returns 1 while it stays over MAX or uncountable, else 0."""
    if ps_text is None:
        ps_text = subprocess.run(["ps", "-axo", "pid=,ppid=,etime=,command="], capture_output=True, text=True).stdout
    pids = vm_pids(ps_text)
    if not pids:
        return 0
    root = root or checkout()
    docker, unknown = None, None
    for pid in pids:   # Docker's is the one with its disk open; with none known, the first lsof could not list
        held = list_files(pid, root)
        if held is None:
            unknown = unknown or pid
        elif held[0]:
            docker = pid
            break
    if docker is None and unknown is None:
        return 0
    pid = docker or unknown
    before, over = said(held if docker else None)
    if releasing and (always or over):
        why = drop(pid)
        if why:
            print(f"could not drop the Docker VM's caches: {why}")
        after, over = said(list_files(pid, root))
        print(f"Docker's VM {pid} held {before}; {after} now")
        return int(over)
    hint = f" (over {MAX}): make leftovers END=1 drops its caches" if over else ""
    print(f"Docker's VM {pid} holds {before}{hint}")
    return int(over)


def main(argv):
    if argv not in ([], ["--release"]):
        sys.exit(__doc__.split("\n\n")[1])
    return report(bool(argv), always=True)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
