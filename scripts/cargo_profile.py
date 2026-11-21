# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Take Cargo's locks before copying or pruning a target profile."""
from contextlib import contextmanager, ExitStack
import fcntl


LEGACY_LOCK = ".cargo-lock"
# In Cargo's acquisition order. Older Cargo takes only LEGACY_LOCK; newer Cargo can share it.
LOCKS = (".cargo-build-lock", LEGACY_LOCK, ".cargo-artifact-lock")


@contextmanager
def try_lock(folder):
    """Hold every profile lock until exit, or yield False with none held if the profile is busy."""
    with ExitStack() as held:
        for name in LOCKS:
            # Cargo may create a missing lock on its first build. Keep the inode so both use the same lock.
            lock = held.enter_context((folder / name).open("a+b"))
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                held.close()
                yield False
                return
        yield True
