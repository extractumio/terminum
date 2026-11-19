#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The kept trees' build cache under a bound (spec AC-206): the integration candidate's tree at its Check's start
(scripts/land_stages.py), `.worktrees/release` and `.worktrees/rehearse` when a run takes them
(scripts/release_tree.py). Under the bound nothing is removed. Over it, each executable's object files that its
debug map no longer names go first: on macOS every incremental build of a crate leaves its previous objects in
`deps` beside the new ones, and nothing else removes them. The executables, libraries, fingerprints and the
incremental cache stay, so the next build stays incremental. Still over, `cargo clean`. A build folder cargo
holds one of its locks on (LOCKS: a build runs there) is left as it is, and no `cargo clean` runs then.

  target_bound.py [<tree>]   keeps the bound of <tree>'s terminumd/target (default: this checkout)"""
from contextlib import ExitStack
import fcntl
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import worktree  # noqa: E402

BOUND_GIB = 20   # a warm tree's own build is about 10 GiB: debug, release and the agents' targets
GIB = 1 << 30
LOCKS = (".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock")   # cargo holds each while it builds there


def gib(size):
    return f"{size / GIB:.1f} GiB"


def disk_use(path):
    """Bytes on disk under `path`, as du counts them: each file once however many links it has. An APFS clone counts
    whole though it shares its blocks with the primary checkout's: the bound errs toward pruning early."""
    return int(subprocess.run(["du", "-sk", str(path)], capture_output=True, text=True, check=True).stdout.split()[0]) * 1024


def build_folders(target):
    """cargo's build folders: target/<profile> and target/<triple>/<profile>, each with its lock."""
    return sorted(lock.parent for pattern in ("*/", "*/*/") for lock in target.glob(pattern + LOCKS[0]))


def held_by_us(folder, stack):
    """Whether we hold `folder`'s build locks now (until `stack` closes), as cargo's own build does, so a build that
    starts meanwhile waits for us: False while a build holds one."""
    for name in LOCKS:
        if not (folder / name).exists():
            continue
        fd = os.open(folder / name, os.O_RDWR)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            os.close(fd)
            return False
        stack.callback(os.close, fd)   # closing it frees the lock
    return True


def debug_map(executable, run=subprocess.run):
    """The names of the object files `executable`'s debug map points at (macOS's N_OSO entries, as `nm -ap` lists
    them); None when nm cannot tell."""
    try:
        listed = run(["nm", "-ap", str(executable)], capture_output=True, text=True)
    except OSError:
        return None
    if listed.returncode:
        return None
    return {Path(line.split(" OSO ", 1)[1].strip()).name for line in listed.stdout.splitlines() if " OSO " in line}


def stale_objects(deps, names_of=debug_map):
    """Each executable unit's objects (`<unit>.<…>.rcgu.o`) that its executable (`<unit>`) does not name, by file name,
    so a moved tree's map still matches. A unit with no executable (its link failed), or whose map is empty or unknown
    (Linux keeps no objects; a stripped executable), keeps all of its objects."""
    units = {}
    for name in os.listdir(deps):
        if name.endswith(".rcgu.o"):
            units.setdefault(name.split(".", 1)[0], []).append(name)
    for unit, objects in units.items():
        executable = deps / unit
        if not (executable.is_file() and os.access(executable, os.X_OK)):
            continue
        if used := names_of(executable):
            yield from (deps / name for name in objects if name not in used)


def cargo_clean(tree):
    """`cargo clean`, then the dependencies' builds cloned from the primary checkout again, as a new tree gets them
    (worktree.py link): only terminumd's own build is cold. What it did, in words."""
    subprocess.run(["cargo", "clean", "--quiet", "--manifest-path", str(Path(tree) / "terminumd" / "Cargo.toml")], check=True)
    return worktree.link_target(Path(tree).resolve(), worktree.primary_of(tree))


def bound(tree, limit=BOUND_GIB * GIB, say=print, clean=cargo_clean, names_of=debug_map):
    """Keeps `tree`'s terminumd/target under `limit` (bytes; default the bound), saying what it did in one line."""
    target = Path(tree) / "terminumd" / "target"
    if not target.is_dir():
        return say("target: no build yet")
    size = disk_use(target)
    if size <= limit:
        return say(f"target: {gib(size)}, under its bound of {gib(limit)}: nothing removed")
    busy = []
    with ExitStack() as held:
        for folder in build_folders(target):
            if not held_by_us(folder, held):
                busy.append(folder.relative_to(target).as_posix())
            elif (folder / "deps").is_dir():
                for path in stale_objects(folder / "deps", names_of):
                    path.unlink(missing_ok=True)
    left = disk_use(target)
    words = f"target: {gib(size)}, over its bound of {gib(limit)}: {gib(size - left)} of objects no build uses removed, {gib(left)} left"
    if busy:
        words += f"; a build runs in {', '.join(busy)}: left as it is"
    if left > limit:
        words += "; still over: " + ("no cargo clean while a build runs" if busy else f"cargo clean; {clean(tree)}")
    return say(words)


def keep(tree, say=print):
    """bound() for a stage: a failure is said and stops nothing (the stage's own work goes on)."""
    try:
        bound(tree, say=say)
    except (OSError, ValueError, subprocess.CalledProcessError) as failed:
        say(f"target: its bound was not kept: {failed}")


if __name__ == "__main__":
    keep(Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent))
