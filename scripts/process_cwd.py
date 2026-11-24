# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-102, AC-104: process working-directory metadata; failed inspection is never an empty snapshot."""
import os
from pathlib import Path
import shutil
import subprocess

QUERY_S = 15


class InspectionError(OSError):
    """The working directories could not be inspected completely."""


def inspect(what, probe, linux, mac):
    """`linux()` where /proc has `probe`, else `mac()` by lsof; any failure an InspectionError naming `what`."""
    try:
        if Path(probe).exists():
            return linux()
        if not shutil.which("lsof"):
            raise InspectionError("no /proc and no lsof")
        return mac()
    except (OSError, subprocess.SubprocessError, ValueError) as error:
        raise InspectionError(f"cannot inspect {what}: {error}") from error


def within(path, tree):
    """Whether `path` is `tree` or under it."""
    return path == tree or path.startswith(tree + os.sep)


def directories():
    """{pid: (command, cwd)} without environments; callers decide directory boundaries."""
    return inspect("process working directories", "/proc/self/cwd", linux_directories, mac_directories)


def linux_directories():
    found = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            folder = os.readlink(entry / "cwd")
            found[int(entry.name)] = ((entry / "comm").read_text().strip(), folder)
        except (FileNotFoundError, ProcessLookupError):
            continue
        except PermissionError:
            try:
                if entry.stat().st_uid != os.getuid():
                    continue  # another user's protected process cannot belong to this run
            except FileNotFoundError:
                continue
            raise
    return found


def lsof(*selection, empty_ok=False):
    """(pid, command, name) of lsof's -F pcn records of `selection`; a busy machine can miss one deadline,
    and partial output of a timed-out query is never used. `empty_ok`: lsof's status 1 with nothing said
    means none matched."""
    for attempt in range(2):
        try:
            out = subprocess.run(["lsof", *selection, "-F", "pcn"], capture_output=True, text=True,
                                 check=True, timeout=QUERY_S).stdout
            break
        except subprocess.TimeoutExpired:
            if attempt:
                raise
        except subprocess.CalledProcessError as error:
            if empty_ok and error.returncode == 1 and not error.stdout and not error.stderr:
                return []
            raise
    records = []
    pid = command = None
    # lsof separates fields with LF; a path can contain other Unicode line separators.
    for line in out.split("\n"):
        tag, value = line[:1], line[1:]
        if tag == "p":
            pid, command = int(value), None
        elif tag == "c":
            command = value
        elif tag == "n":
            if pid is None or command is None:
                raise InspectionError("incomplete lsof record")
            records.append((pid, command, value))
    return records


def mac_directories():
    found = {}
    for pid, command, folder in lsof("-a", "-d", "cwd"):
        if not os.path.isabs(folder):
            raise InspectionError("incomplete lsof working-directory record")
        found[pid] = (command, folder)
    if not found:
        raise InspectionError("lsof returned no working directories")
    return found


def bound_in(folder):
    """The pids holding a unix socket bound to a path in `folder`: a tmux server leaves its run's process
    group and working directory when it daemonizes, and only its socket, in the run's TMUX_TMPDIR, says
    whose it is (AC-102)."""
    return {pid for pid, _ in socket_paths(os.path.realpath(folder))}


def socket_paths(under=None):
    """(pid, path) of each unix socket of this user's processes bound to a path (`under`: in that folder);
    the path as it was bound, so a server whose socket file was removed still names it. Never an empty
    list on failure."""
    keep = (lambda path: os.path.isabs(path)) if under is None else (lambda path: within(path, under))
    return inspect("unix sockets", "/proc/net/unix", lambda: linux_socket_paths(keep), lambda: mac_socket_paths(keep))


def mac_socket_paths(keep):
    # a peer's address (->0x…) is no path
    return [(pid, path) for pid, _, path in lsof("-a", "-u", str(os.getuid()), "-U", empty_ok=True) if keep(path)]


def linux_socket_paths(keep, proc=Path("/proc")):
    paths = {}
    for line in (proc / "net/unix").read_text().splitlines()[1:]:
        fields = line.split(None, 7)   # Num RefCount Protocol Flags Type St Inode Path
        if len(fields) == 8 and fields[7].startswith("/") and keep(fields[7]):
            paths[f"socket:[{fields[6]}]"] = fields[7]
    found = []
    for entry in proc.iterdir() if paths else []:
        if not entry.name.isdigit():
            continue
        try:
            if entry.stat().st_uid != os.getuid():
                continue
            fds = list((entry / "fd").iterdir())
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue   # ended meanwhile, or not readable: then not this user's to end
        for fd in fds:
            try:
                link = os.readlink(fd)
            except OSError:
                continue   # closed meanwhile
            if link in paths:
                found.append((int(entry.name), paths[link]))
    return found
