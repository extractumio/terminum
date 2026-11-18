#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""`make install` in the install turn (spec AC-104): one install at a time in this repository, since every install
stages into the primary checkout's dist/package and restarts the one terminumd. The turn is the one a landing's
install and the integration queue's After hold (worktree.INSTALL_TURN); an install by hand waits for theirs, saying
whose, and theirs wait for it. The `make install` those start inside the turn they hold runs at once: HELD in its
environment.

  install_turn.py <command> [<argument>...]   runs the command in the turn; its exit status is the command's"""
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from worktree import INSTALL_TURN, Turn  # noqa: E402

# set by whoever holds the install turn for the installs it starts: theirs would wait for it forever
HELD = "TM_INSTALL_TURN_HELD"


def main(argv, say=print):
    if not argv:
        sys.exit("usage: install_turn.py <command> [<argument>...]")
    if os.environ.get(HELD) == "1":
        os.execvp(argv[0], argv)
    with Turn(Path.cwd(), INSTALL_TURN) as turn:
        turn.take_as(f"make install in {Path.cwd()} (pid {os.getpid()})",
                     lambda holder: say(f"make install: {holder} holds the install turn: waiting for its end", flush=True))
        return subprocess.run(argv).returncode


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except KeyboardInterrupt:
        sys.exit(130)
