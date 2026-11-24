# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-102, AC-104: process working-directory metadata; failed inspection is never an empty snapshot."""
import os
from pathlib import Path
import shutil
import subprocess

QUERY_S = 15


class InspectionError(OSError):
    """The working directories could not be inspected completely."""


def directories():
    """{pid: (command, cwd)} without environments; callers decide directory boundaries."""
    try:
        if Path("/proc/self/cwd").exists():
            return linux_directories()
        if not shutil.which("lsof"):
            raise InspectionError("no /proc and no lsof")
        return mac_directories()
    except (OSError, subprocess.SubprocessError, ValueError) as error:
        raise InspectionError(f"cannot inspect process working directories: {error}") from error


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
    """lsof's -F pcn records of `selection`; a busy machine can miss one deadline, and partial output of a
    timed-out query is never used. `empty_ok`: lsof's status 1 with no output means none matched."""
    for attempt in range(2):
        try:
            return subprocess.run(["lsof", *selection, "-F", "pcn"], capture_output=True, text=True,
                                  check=True, timeout=QUERY_S).stdout
        except subprocess.TimeoutExpired:
            if attempt:
                raise
        except subprocess.CalledProcessError as error:
            if empty_ok and error.returncode == 1 and not error.stdout:
                return ""
            raise


def mac_directories():
    out = lsof("-a", "-d", "cwd")
    found = {}
    pid = command = None
    # lsof separates fields with LF; a path can contain other Unicode line separators.
    for line in out.split("\n"):
        tag, value = line[:1], line[1:]
        if tag == "p":
            pid, command = int(value), None
        elif tag == "c":
            command = value
        elif tag == "n":
            if pid is None or command is None or not os.path.isabs(value):
                raise InspectionError("incomplete lsof working-directory record")
            found[pid] = (command, value)
    if not found:
        raise InspectionError("lsof returned no working directories")
    return found


def bound_in(folder):
    """{pid: command} of the processes holding a unix socket bound to a path in `folder`: a tmux server
    leaves its run's process group and working directory when it daemonizes, and only its socket, in
    the run's TMUX_TMPDIR, says whose it is (AC-102)."""
    folder = os.path.realpath(folder)
    return {pid: command for pid, command, path in socket_paths()
            if path == folder or path.startswith(folder + os.sep)}


def socket_paths():
    """(pid, command, path) of each unix socket of this user's processes bound to a path; the path as it was
    bound, so a server whose socket file was removed still names it. Never an empty list on failure."""
    try:
        if Path("/proc/net/unix").exists():
            return linux_socket_paths()
        if not shutil.which("lsof"):
            raise InspectionError("no /proc and no lsof")
        return mac_socket_paths()
    except (OSError, subprocess.SubprocessError, ValueError) as error:
        raise InspectionError(f"cannot inspect unix sockets: {error}") from error


def mac_socket_paths():
    found = []
    pid = command = None
    for line in lsof("-a", "-u", str(os.getuid()), "-U", empty_ok=True).split("\n"):
        tag, value = line[:1], line[1:]
        if tag == "p":
            pid, command = int(value), None
        elif tag == "c":
            command = value
        elif tag == "n" and os.path.isabs(value):   # a peer's address (->0x…) or none is no path
            if pid is None or command is None:
                raise InspectionError("incomplete lsof socket record")
            found.append((pid, command, value))
    return found


def linux_socket_paths(proc=Path("/proc")):
    paths = {}
    for line in (proc / "net/unix").read_text().splitlines()[1:]:
        fields = line.split(None, 7)   # Num RefCount Protocol Flags Type St Inode Path
        if len(fields) == 8 and fields[7].startswith("/"):
            paths[f"socket:[{fields[6]}]"] = fields[7]
    found = []
    for entry in proc.iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if entry.stat().st_uid != os.getuid():
                continue
            fds = list((entry / "fd").iterdir())
            command = (entry / "comm").read_text().strip()
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue   # ended meanwhile, or not readable: then not this user's to end
        for fd in fds:
            try:
                link = os.readlink(fd)
            except OSError:
                continue   # closed meanwhile
            if link in paths:
                found.append((int(entry.name), command, paths[link]))
    return found
