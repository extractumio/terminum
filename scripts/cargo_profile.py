# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Take Cargo's locks before copying or pruning a target profile; mark a target folder cargo did not make."""
from contextlib import contextmanager, ExitStack
import fcntl
import plistlib
import subprocess
import sys


LEGACY_LOCK = ".cargo-lock"
# In Cargo's acquisition order. Older Cargo takes only LEGACY_LOCK; newer Cargo can share it.
LOCKS = (".cargo-build-lock", LEGACY_LOCK, ".cargo-artifact-lock")
# cargo's own tag (https://bford.info/cachedir/), which backup tools skip
CACHE_TAG = ("Signature: 8a477f597d28d172789f06886806bc55\n"
             "# This file is a cache directory tag created by cargo.\n"
             "# For information about cache directory tags see https://bford.info/cachedir/\n")
# Time Machine's exclusion of one item, as cargo and `tmutil addexclusion` set it
BACKUP_EXCLUDE = "com.apple.metadata:com_apple_backup_excludeItem"
BACKUP_EXCLUDED = plistlib.dumps("com.apple.backupd", fmt=plistlib.FMT_BINARY).hex()


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


def mark_cache(target):
    """Marks `target` as cargo marks a target folder it makes: its cache tag and, on macOS, Time Machine's exclusion
    (written directly: tmutil takes seconds a call). None, or why it is not excluded."""
    (target / "CACHEDIR.TAG").write_text(CACHE_TAG)
    if sys.platform != "darwin":
        return None
    run = subprocess.run(["xattr", "-wx", BACKUP_EXCLUDE, BACKUP_EXCLUDED, str(target)], capture_output=True, text=True)
    return run.returncode and (run.stderr.strip() or f"xattr exited {run.returncode}") or None
