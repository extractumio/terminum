#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Every checkout's build cache under a bound and out of backups (spec AC-206, which says where it is kept). A target
without cargo's cache tag (a clone made it) is marked as cargo marks its own. Under the bound nothing is removed. Over it,
each executable's object files that its debug map no longer names go first: on macOS every incremental build of a
crate leaves its previous objects in `deps` beside the new ones, and nothing else removes them. The executables,
libraries, fingerprints and the incremental cache stay, so the next build stays incremental. Still over, build
folders are emptied one at a time, under the locks still held, until it is under: the `debug` ones (they grow) first,
`release` and the cross-compiled ones (`make install` packages them) last, only where that gets it under. In a
worktree, each emptied folder gets the primary's dependencies' builds cloned back. If Cargo holds a profile lock, that
profile is left in place and nothing is emptied.

  target_bound.py [<tree>]   keeps the bound of <tree>'s terminumd/target (default: this checkout)"""
from contextlib import ExitStack
import os
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import cargo_profile  # noqa: E402
import worktree  # noqa: E402

BOUND_GIB = 20   # a warm tree's own build is about 10 GiB: debug, release and the agents' targets
GIB = 1 << 30


def gib(size):
    return f"{size / GIB:.1f} GiB"


def disk_use(path):
    """Bytes on disk under `path`, as du counts them: each file once however many links it has. An APFS clone counts
    whole though it shares its blocks with the primary checkout's: the bound errs toward pruning early."""
    return disk_uses([path])[Path(path)]


def disk_uses(paths):
    """{path: bytes} of each of `paths` as disk_use counts them, in one du run."""
    listed = subprocess.run(["du", "-sk", *map(str, paths)], capture_output=True, text=True, check=True).stdout
    return {Path(path): int(size) * 1024 for size, path in (line.split("\t", 1) for line in listed.splitlines())}


def build_folders(target):
    """cargo's build folders: target/<profile> and target/<triple>/<profile>, each with its lock."""
    return sorted(lock.parent for pattern in ("*/", "*/*/") for lock in target.glob(pattern + cargo_profile.LEGACY_LOCK))


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


def empty(folder):
    """Everything in a build folder whose locks this run holds but those locks, so a cargo waiting on them waits on
    the same files."""
    for entry in folder.iterdir():
        if entry.name in cargo_profile.LOCKS:
            continue
        if entry.is_dir() and not entry.is_symlink():
            shutil.rmtree(entry)
        else:
            entry.unlink()


REFILLED = {"cloned": "{}: its dependencies' builds cloned from the primary checkout",
            "busy": "{}: the primary's is busy, cargo builds it whole",
            "no clone": "{}: no clone on this disk, cargo builds it whole"}


def refill(tree, names):
    """In a worktree, the primary's idle same folders' dependencies' builds cloned into its emptied ones `names`, as a
    new tree gets them; what it did, in words, a line a folder (none in the primary checkout)."""
    primary, target = worktree.primary_of(tree), Path(tree) / "terminumd" / "target"
    if Path(tree).resolve() == primary:
        return []
    sources = [(name, primary / "terminumd" / "target" / name) for name in names]
    return [REFILLED[worktree.clone_profile(source, target / name)].format(name)
            for name, source in sources if source.is_dir()]


def empty_until_under(target, folders, left, limit, clean):
    """Empties `folders` (their locks held) one at a time until `target` is under `limit`: the `debug` ones (they grow)
    first, then the rest, the largest first within each; one of the rest only while emptying it and those after it
    could get it under. The names emptied, the bytes left, and why it stopped over, if it did."""
    sizes = disk_uses(folders)
    order = sorted(folders, key=lambda folder: (folder.name != "debug", -sizes[folder]))
    emptied, stopped = [], None
    for at, folder in enumerate(order):
        if left <= limit:
            break
        if folder.name != "debug" and left - sum(sizes[rest] for rest in order[at:]) > limit:
            stopped = "the rest kept: emptying it would not get it under"
            break
        clean(folder)
        emptied.append(folder.relative_to(target).as_posix())
        left -= sizes[folder]   # before any refill: its clones share the primary's blocks
    return emptied, disk_use(target) if emptied else left, stopped


def bound(tree, limit=BOUND_GIB * GIB, say=print, clean=empty, names_of=debug_map, refill=refill):
    """Keeps `tree`'s terminumd/target under `limit` (bytes; default the bound), saying what it did in one line."""
    target = Path(tree) / "terminumd" / "target"
    if not target.is_dir():
        return say("target: no build yet")
    if not (target / "CACHEDIR.TAG").exists() and (failed := cargo_profile.mark_cache(target)):   # cargo did not make it
        say(f"target: not excluded from Time Machine: {failed}")
    size = disk_use(target)
    if size <= limit:
        return say(f"target: {gib(size)}, under its bound of {gib(limit)}: nothing removed")
    busy, free = [], []
    with ExitStack() as held:
        for folder in build_folders(target):
            if not held.enter_context(cargo_profile.try_lock(folder)):
                busy.append(folder.relative_to(target).as_posix())
                continue
            free.append(folder)
            if (folder / "deps").is_dir():
                for path in stale_objects(folder / "deps", names_of):
                    path.unlink(missing_ok=True)
        left = disk_use(target)
        words = f"target: {gib(size)}, over its bound of {gib(limit)}: {gib(size - left)} of objects no build uses removed, {gib(left)} left"
        if busy:
            words += f"; a build runs in {', '.join(busy)}: left as it is"
        if left > limit and busy:
            words += "; still over: nothing emptied while a build runs"
        elif left > limit:
            emptied, left, stopped = empty_until_under(target, free, left, limit, clean)
            words += f"; still over: {', '.join(emptied) or 'nothing'} emptied, {gib(left)} left"
            words += "".join(f"; {note}" for note in [stopped, *refill(tree, emptied)] if note)   # under the locks
    return say(words)


def keep(tree, say=print):
    """bound() for a stage: a failure is said and stops nothing (the stage's own work goes on)."""
    try:
        bound(tree, say=say)
    except (OSError, ValueError, subprocess.CalledProcessError) as failed:
        say(f"target: its bound was not kept: {failed}")


if __name__ == "__main__":
    keep(Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent))
