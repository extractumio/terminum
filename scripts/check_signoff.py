#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Whether every commit of an outside pull request carries its author's `Signed-off-by:` line,
the record of CONTRIBUTING.md's contributor license agreement. Without it the pull request's code
is not taken (CLAUDE.md §9 "license").

    python3 scripts/check_signoff.py 12            # pull request #12 of this repository (gh)
    python3 scripts/check_signoff.py --range A..B  # commits of a local range (git)

Exit 0: all signed off; 1: some are not (each named, with how to fix it); 2: the commits could not be read.
"""
import argparse
import json
import re
import subprocess
import sys

# one pass, no overlapping quantifiers: commit messages are a contributor's text
TRAILER = re.compile(r"^Signed-off-by:[^<\n]*<([^<>\s]+@[^<>\s]+)>[ \t]*$", re.MULTILINE)

FIX = """How to fix it, in the pull request's branch (N: your commits in it):
  git rebase --signoff HEAD~N
  git push --force-with-lease
A merge commit, an employer's rights: CONTRIBUTING.md "Sign off every commit"."""


def signed_off(message: str, author_email: str) -> bool:
    """A `Signed-off-by:` trailer with the commit author's own address (case aside)."""
    want = author_email.strip().lower()
    return bool(want) and any(email.lower() == want for email in TRAILER.findall(message))


def unsigned(commits: list[dict]) -> list[dict]:
    """The commits (`sha`, `email`, `message`) lacking their author's sign-off."""
    return [c for c in commits if not signed_off(c["message"], c["email"])]


def from_pr(number: str) -> list[dict]:
    if not number.isdigit():
        raise ValueError(f"not a pull request number: {number}")
    jq = ".[] | {sha, email: .commit.author.email, message: .commit.message}"   # one commit per line, every page
    out = subprocess.run(["gh", "api", "--paginate", "--jq", jq, f"repos/{{owner}}/{{repo}}/pulls/{number}/commits"],
                         capture_output=True, text=True, check=True).stdout
    return [json.loads(line) for line in out.splitlines() if line.strip()]


def from_range(spec: str, cwd: str | None = None) -> list[dict]:
    out = subprocess.run(["git", "log", "--format=%H%x00%ae%x00%B%x1e", spec],
                         capture_output=True, text=True, check=True, cwd=cwd).stdout
    commits = []
    for record in filter(str.strip, out.split("\x1e")):
        sha, email, message = record.strip("\n").split("\x00", 2)
        commits.append({"sha": sha, "email": email, "message": message})
    return commits


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("pr", nargs="?", help="pull request number")
    ap.add_argument("--range", dest="spec", help="a git revision range instead, e.g. origin/main..HEAD")
    args = ap.parse_args(argv)
    if bool(args.pr) == bool(args.spec):
        ap.error("give a pull request number or --range, one of them")
    try:
        commits = from_pr(args.pr) if args.pr else from_range(args.spec)
    except (subprocess.CalledProcessError, OSError, ValueError, KeyError) as e:
        print(f"check_signoff: the commits could not be read: {getattr(e, 'stderr', '') or e}".strip(), file=sys.stderr)
        return 2
    if not commits:
        print("check_signoff: no commits to check", file=sys.stderr)
        return 2
    missing = unsigned(commits)
    for c in missing:
        print(f"{c['sha'][:10]} by {c['email']}: no 'Signed-off-by: <name> <{c['email']}>' line")
    if missing:
        print(f"\n{len(missing)} of {len(commits)} commits not signed off.\n{FIX}")
        return 1
    print(f"all {len(commits)} commits signed off by their authors")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
