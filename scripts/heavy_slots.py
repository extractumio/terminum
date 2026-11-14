#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Heavy builds of all this repository's trees take turns (spec AC-104): a step that builds or tests
terminumd with cargo holds one of a few slots while it runs, while a tree works and while it lands. A slot
is a file in git's common folder that the kernel frees with its holder; TM_HEAVY_SLOTS sets how many
(default: the cores' count / 8, at least 1: each cargo run takes every core)."""
from contextlib import contextmanager, suppress
import os
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from worktree import Turn  # noqa: E402

HEAVY = ("debug-build", "clippy", "cargo")
LOOK_S = 2


def count():
    try:
        return max(1, int(os.environ["TM_HEAVY_SLOTS"]))
    except (KeyError, ValueError):
        return max(1, (os.cpu_count() or 2) // 8)


def holder(turn):
    return turn.path.with_suffix(".holder")


@contextmanager
def slot(root, step, say=print, sleep=time.sleep):
    """Holds a slot while a heavy step runs; any other step runs at once."""
    if step not in HEAVY:
        yield
        return
    turns, told = [Turn(root, f"terminum-heavy.{n}.slot") for n in range(count())], False
    while True:
        for turn in turns:
            with turn:
                if turn.take():
                    holder(turn).write_text(f"{Path(root).name} {step}")
                    yield
                    return
        if not told:
            names = []
            for turn in turns:
                with suppress(OSError):
                    names.append(holder(turn).read_text().strip())
            say(f"slot: {step} waits for one of {len(turns)} heavy-build slots: {', '.join(names) or 'all held'}")
            told = True
        sleep(LOOK_S)
