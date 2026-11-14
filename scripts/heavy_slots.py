#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Heavy builds of all this repository's trees take turns (spec AC-104): a cargo step of a tree's checks, and
a landing's install, runs holding one of a few slots; the landing in the landing turn has one of its own, so
it never waits behind trees at work. A slot is a file in git's common folder that the kernel frees with its
holder; TM_HEAVY_SLOTS sets how many (default: the cores' count / 8, at least 1: each cargo run takes every
core)."""
from contextlib import contextmanager
import os
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from worktree import Turn  # noqa: E402

LOOK_S = 2


def count():
    try:
        return max(1, int(os.environ["TM_HEAVY_SLOTS"]))
    except (KeyError, ValueError):
        return max(1, (os.cpu_count() or 2) // 8)


@contextmanager
def slot(root, step, landing=False, say=print, sleep=time.sleep):
    """Holds a slot while `step` runs: the landing's own when `landing`, else the first of the others free."""
    names = ["terminum-heavy.land.slot"] if landing else [f"terminum-heavy.{n}.slot" for n in range(count())]
    turns, told = [Turn(root, name) for name in names], False
    while True:
        for turn in turns:
            with turn:
                if turn.take():
                    turn.hold_as(f"{Path(root).name} {step}")
                    yield
                    return
        if not told:
            say(f"slot: {step} waits for one of {len(turns)} heavy-build slots: {', '.join(t.holder() for t in turns)}")
            told = True
        sleep(LOOK_S)
