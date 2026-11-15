# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Shared check pools (spec AC-104): kernel-held slots in the repository's common git folder."""
from contextlib import contextmanager
from pathlib import Path
import time

from worktree import Turn

LOOK_S = 2


@contextmanager
def slot(root, step, pool, amount, waiting_for, landing=False, say=print, sleep=time.sleep):
    """Hold a free worker slot, or the landing's reserved slot, until the caller leaves the context."""
    names = [f"terminum-{pool}.land.slot"] if landing else [f"terminum-{pool}.{n}.slot" for n in range(amount)]
    turns, told = [Turn(root, name) for name in names], False
    while True:
        for turn in turns:
            with turn:
                if turn.take():
                    turn.hold_as(f"{Path(root).name} {step}")
                    yield
                    return
        if not told:
            say(f"slot: {step} waits for {waiting_for}: {', '.join(t.holder() for t in turns)}")
            told = True
        sleep(LOOK_S)
