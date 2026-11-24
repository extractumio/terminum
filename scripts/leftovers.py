#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""What test runs left on this machine (spec AC-102); --end removes it. `make leftovers` says,
`make leftovers END=1` and `make release` remove.

The one exception to ending nothing by a pattern (CLAUDE.md §6). Nothing younger than OLD_S goes,
so a run of another session is safe:
- the one test helper that outlives its test, TEST_HELPER, with no parent (ppid 1): ended by its
  pid with SIGTERM, its command read again just before; one that stays is named, never SIGKILLed;
- a tmux server of a `tm-*` name, older than OLD_S, whose every socket lay in a test's folder and is gone
  with it, so nothing reaches it but its pid: ended by its pid with SIGTERM, as that helper (every tmux
  server has parent 1); one whose socket went from elsewhere (a /tmp cleaner) is only named;
- a dead `tm-*` socket of tmux (a connection refused): removed;
- a folder of this user with a prefix tests use, nothing in it recent or held open: removed.
Named only, since either can be someone's work: a live tmux server on a `tm-*` socket, and any
other test binary or terminumd of a build folder with no parent."""
import os
from pathlib import Path
import re
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from process_cwd import InspectionError, socket_paths  # noqa: E402

OLD_S = 6 * 3600
TEST_HELPER = re.compile(r"/\S*/terminumd/target/(?:debug|release)/deps/terminumd-[0-9a-f]{16} --exact cli::tests::fake_terminumd( |$)")
# a test's own socket name (testrun's `own`: tm-test-live-<pid>), never the user's `-L terminum`
TEST_TMUX = re.compile(r"(?:\S*/)?tmux(?: \S+)*? -L tm-\S+(?: |$)")
BUILD_OUTPUT = re.compile(r"/\S*/terminumd/target/(?:debug|release)/(?:terminumd|deps/terminumd-[0-9a-f]{16})( |$)")
# what tests name their folders; test_leftovers.py fails on a test's prefix missing here
FOLDER_PREFIXES = ("tm-", "tma-", "tmb-", "tme-", "tmh-", "tmi-", "tml-", "tmln-", "tmlo-", "tmlp-", "tmlw-",
                   "tmr-", "tmr9-", "tmrs-", "tmrel-", "tmsq-", "tmv-", "tmw-", "tmz-", "tmc.", "tmps-", "fssh-", ".fssh-")


def age_s(etime):
    """Seconds of ps's etime: [[dd-]hh:]mm:ss."""
    days, _, clock = etime.rpartition("-")
    seconds = 0
    for part in clock.split(":"):
        seconds = seconds * 60 + int(part)
    return seconds + int(days or 0) * 86400


def in_test_folder(path):
    """Whether `path` lies in a folder named as tests name theirs (check_step.py's tmc.*, a test's tml-*)."""
    return any(part.startswith(FOLDER_PREFIXES) and part not in FOLDER_PREFIXES for part in Path(path).parts[:-1])


def orphans(ps_text):
    """(pid, command) of each process with no parent (ppid 1) older than OLD_S."""
    for line in ps_text.splitlines():
        fields = line.split(None, 3)
        if len(fields) == 4 and fields[0].isdigit() and fields[1] == "1" and age_s(fields[2]) >= OLD_S:
            yield int(fields[0]), fields[3]


def strays(ps_text):
    """(pid, command, whether to end it) of each process no test run owns any more."""
    found = []
    for pid, command in orphans(ps_text):
        if TEST_HELPER.match(command):
            found.append((pid, command, True))
        elif BUILD_OUTPUT.match(command):
            found.append((pid, command, False))
    return found


def lost_servers(ps_text, bound):
    """(pid, command, whether to end it) of each old test tmux server with no parent: ended only when
    every socket it bound (`bound`: (pid, command, path) of this user's processes, or None when unknown)
    lay in a test's folder and is gone; one still reachable is a live socket's, said there."""
    paths = {}
    for pid, _, path in bound or []:
        paths.setdefault(pid, []).append(path)
    found = []
    for pid, command in orphans(ps_text):
        if TEST_TMUX.match(command):
            if pid not in paths:
                found.append((pid, command, False))
            elif not any(os.path.lexists(path) for path in paths[pid]):
                found.append((pid, command, all(in_test_folder(path) for path in paths[pid])))
    return found


def processes():
    return subprocess.run(["ps", "-axo", "pid=,ppid=,etime=,command="], capture_output=True, text=True, check=True).stdout


def end(pid, command):
    """SIGTERM if `pid` is still that command; True once it has gone."""
    now = subprocess.run(["ps", "-o", "command=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    if now != command:
        return True
    try:
        os.kill(pid, 15)
    except ProcessLookupError:
        return True
    for _ in range(20):
        time.sleep(0.1)
        if subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()[:1] in ("", "Z"):
            return True
    return False


def sockets(folder, now):
    """(dead, live) `tm-*` sockets of tmux in `folder` older than OLD_S."""
    dead, live = [], []
    for path in sorted(folder.glob("tm-*")) if folder.is_dir() else []:
        try:
            info = path.lstat()
        except FileNotFoundError:
            continue
        if not stat.S_ISSOCK(info.st_mode) or now - info.st_mtime < OLD_S:
            continue
        with socket.socket(socket.AF_UNIX) as probe:
            try:
                probe.connect(str(path))
                live.append(path)
            except ConnectionRefusedError:
                dead.append(path)
            except OSError:
                pass  # no socket by now, or a path too long to reach: not known to be dead
    return dead, live


def open_paths():
    """Every file a process holds open, as `lsof -Fn` names it; None if that cannot be known."""
    try:
        listed = subprocess.run(["lsof", "-Fn"], capture_output=True, text=True, errors="replace", timeout=120).stdout
    except (OSError, subprocess.TimeoutExpired):
        return None
    return {line[1:] for line in listed.splitlines() if line.startswith("n/")} or None


def recent(folder, now):
    """True if anything in `folder`, or a part that cannot be read, is younger than OLD_S."""
    try:
        if now - folder.lstat().st_mtime < OLD_S:
            return True
        for parent, names, files in os.walk(folder):
            for name in names + files:
                if now - os.lstat(os.path.join(parent, name)).st_mtime < OLD_S:
                    return True
    except OSError:
        return True
    return False


def folders(roots, now, held):
    """Folders of this user with a test's prefix, nothing in them recent or in `held`."""
    found = []
    for root in roots:
        try:
            entries = list(os.scandir(root))
        except OSError:
            continue
        for entry in entries:
            if not entry.name.startswith(FOLDER_PREFIXES) or entry.name in FOLDER_PREFIXES:
                continue
            try:
                info = entry.stat(follow_symlinks=False)
            except OSError:
                continue
            path = Path(entry.path)
            if stat.S_ISDIR(info.st_mode) and info.st_uid == os.getuid() and not recent(path, now) \
                    and not any(p.startswith(f"{path}/") for p in held):
                found.append(path)
    return found


def main(argv):
    if argv not in ([], ["--end"]):
        sys.exit("usage: leftovers.py [--end]")
    ending = bool(argv)
    now = time.time()
    socket_folder = Path(os.environ.get("TMUX_TMPDIR", "/tmp")) / f"tmux-{os.getuid()}"
    roots = sorted({Path(tempfile.gettempdir()).resolve(), Path("/tmp").resolve()})
    stuck = 0
    listed = processes()
    try:
        bound = socket_paths()
    except InspectionError as error:
        bound = None
        print(f"{error}: no tmux server is ended")
    for pid, command, ours in strays(listed) + lost_servers(listed, bound):
        if ours and ending:
            gone = end(pid, command)
            stuck += not gone
            print(f"{'ended' if gone else 'still running'} {pid}: {command[:120]}")
        else:
            print(f"{'stray test' if ours else 'named only'} {pid}: {command[:120]}")
    dead, live = sockets(socket_folder, now)
    for path in live:
        print(f"named only, a live tmux server: {path.name} (tmux -L {path.name} kill-server if it is yours)")
    held = open_paths()
    if held is None:
        print("lsof says nothing of what is open: every test folder stays")
    old = [] if held is None else folders(roots, now, held)
    if ending:
        for path in dead:
            try:
                path.unlink(missing_ok=True)
            except OSError as e:
                print(f"could not remove {path}: {e}")
        for path in old:
            shutil.rmtree(path, ignore_errors=True)
        kept = sum(path.exists() for path in old)
        stuck += kept
        print(f"removed {len(dead)} dead tmux sockets, {len(old) - kept} test folders ({kept} could not go)")
    else:
        print(f"{len(dead)} dead tmux sockets, {len(old)} test folders (--end removes them)")
    return 1 if stuck else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
