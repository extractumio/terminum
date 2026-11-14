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


def mac_directories():
    # A busy machine can miss one deadline; never consume partial output from a timed-out query.
    for attempt in range(2):
        try:
            out = subprocess.run(["lsof", "-a", "-d", "cwd", "-F", "pcn"], capture_output=True,
                                 text=True, check=True, timeout=QUERY_S).stdout
            break
        except subprocess.TimeoutExpired:
            if attempt:
                raise
    found = {}
    pid = command = None
    for line in out.splitlines():
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
