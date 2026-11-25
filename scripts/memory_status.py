#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The agents' long-term memory at a session's start (CLAUDE.md "Project memory"): project-memory/
INDEX.md, state.md and handoff.md, and whether consolidation is due. Claude Code's SessionStart hook
runs it, so it always exits 0. Only regular files are read: no links out of the folder."""
from datetime import date
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from worktree import common_dir, Refused  # noqa: E402

DUE_DAYS = 14
PRINTED = ("INDEX.md", "state.md", "handoff.md")
# Caps of the files RULES.md's Files table does not list; that table sets the rest.
LINES = {"INDEX.md": 40, "RULES.md": 70}
OTHER_LINES = 150
# A row of RULES.md's Files table: | `name.md` | ... | 150 |, or `none` for a file kept verbatim.
ROW = re.compile(r"^\|\s*`([^`/]+\.md)`\s*\|.*\|\s*(\d+|none)\s*\|\s*$", re.M | re.I)


def limits(memory):
    """Each file's line cap: RULES.md's Files table over the defaults; None is no cap."""
    caps = dict(LINES)
    rules = memory / "RULES.md"
    if regular(rules):
        for name, cap in ROW.findall(rules.read_text(errors="replace")):
            caps[name] = None if cap.lower() == "none" else int(cap)
    return caps


def folder():
    root = Path(__file__).resolve().parent
    try:
        return common_dir(root).parent / "project-memory"
    except Refused:
        return root.parent / "project-memory"


def regular(path):
    return path.is_file() and not path.is_symlink()


def due(memory, today):
    """Why consolidation is due, if it is."""
    reasons = []
    index = (memory / "INDEX.md").read_text()
    stamp = re.search(r"^Last consolidated: (\d{4}-\d{2}-\d{2})", index, re.M)
    try:
        days = (today - date.fromisoformat(stamp[1])).days if stamp else None
    except ValueError:
        days = None
    if days is None:
        reasons.append("INDEX.md has no valid 'Last consolidated: YYYY-MM-DD'")
    elif days > DUE_DAYS:
        reasons.append(f"last consolidated {stamp[1]}, over {DUE_DAYS} days ago")
    caps = limits(memory)
    for path in sorted(memory.glob("*.md")):
        if not regular(path):
            reasons.append(f"{path.name} is not a regular file, not read")
            continue
        limit = caps.get(path.name, OTHER_LINES)
        count = len(path.read_text(errors="replace").splitlines())
        if limit is not None and count > limit:
            reasons.append(f"{path.name} has {count} lines, over {limit}")
        if path.name != "INDEX.md" and f"`{path.name}`" not in index:
            reasons.append(f"{path.name} is not in INDEX.md")
    return reasons


def report(memory, today):
    if not regular(memory / "INDEX.md"):
        return f"Project memory: not found at {memory}. Say so in the report; never create it elsewhere."
    parts = []
    for name in PRINTED:
        if regular(memory / name):
            parts.append(f"--- project-memory/{name}\n{(memory / name).read_text(errors='replace').strip()}")
    reasons = due(memory, today)
    if reasons:
        parts.append("Consolidation is due (project-memory/RULES.md), at the end of this task: " + "; ".join(reasons))
    return "\n".join(parts)


def main(argv):
    try:
        print(report(Path(argv[0]) if argv else folder(), date.today()))
    except OSError as e:
        print(f"Project memory: could not be read: {e}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
