#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The full run of CLAUDE.md §8, the release's gate (spec AC-104, AC-146, AC-201), on a content: <id> names the
content of every tracked and new file (check_changed.tree_id), so committing a checked tree keeps its passes.
- A pass per step (each e2e script is one): a step passed on this content, in this tree or any of the repository
  (gate_record), is skipped and said; --force, and a rehearsal, run every step; a release counts only passes younger
  than 48 hours. Content that differs only in docs (no file the package carries) from a commit whose content passed
  whole runs the scan alone.
- Before any step it builds what the steps read and no pass covers (the UI, terminumd's debug build); red, nothing runs.
- Each step runs under check_step.py's guard (AC-102), its output in .checks/<id>.<step>.log, with the environment
  ALLOWED alone and TM_TIMING=report: nothing of its caller's reaches a test, and a measured budget a check exceeds
  under this run's own load is said ("TIMING over budget"), judged by --timing on a quiet machine.
- Once every step had its first run, a red step of real programs runs once more (gate_retry.py).
- One full run at a time across trees, on a quiet machine (scripts/load_gate.py, AC-146): exit 75 when the load stays
  high for 15 minutes; a release finding a rehearsal's run in the turn waits for its end and reuses its passes.
  Every run leaves a line in the record (gate_record).

  make check   or   python3 scripts/check_all.py [--force | --passed | --timing] [--by check|release|rehearse]
  --passed: 0 if this content passed whole; --timing: the checks with budgets alone, judged, after a quiet-machine wait"""
import argparse
import os
from pathlib import Path
import platform
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_cancel import run_guard  # noqa: E402
from check_changed import changed_files, is_doc, red, runnable_e2e, tail, tree_id  # noqa: E402
from check_logs import bounded_log, preserve_attempt  # noqa: E402
from check_step import LEFT  # noqa: E402
import gate_record  # noqa: E402
import gate_retry  # noqa: E402
import load_gate  # noqa: E402
from package import COPY  # noqa: E402
from worktree import git  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
# what a step may see of its caller's environment: the machine's, never a session's, a run's or an ssh agent's (AC-201)
ALLOWED = ("PATH", "HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "TERM",
           "XDG_CONFIG_HOME", "TM_PRIVATE_PATTERNS", "TM_HEAVY_SLOTS", "TM_LIVE_SLOTS",
           "CARGO_HOME", "RUSTUP_HOME", "PLAYWRIGHT_BROWSERS_PATH", "DOCKER_HOST", "DOCKER_CONTEXT", "DOCKER_CONFIG")
FRESH_H = 48   # a release reuses passes this young: tmux, node, the browsers or macOS may change under older ones
BUILD = "make ui && cd terminumd && cargo build --locked"
# the checks that measure a wall-clock budget (AC-201's list): --timing runs them alone, judged
TIMED = [("e2e_hooks", "python3 scripts/e2e_hooks.py"), ("e2e_isolation", "python3 scripts/e2e_isolation.py"),
         ("board", "node web/test/e2e_live.mjs chromium board")]


def steps(root, ident):
    """[(name, shell command)] in order."""
    # Docker's take built binaries: their own step; an imported helper (e2e_ws.py) is no check
    e2e = sorted(p for p in root.glob("scripts/e2e_*.py") if "remote_sshd" not in p.name and runnable_e2e(p))
    found = [("test", "make test"),
             ("clippy", "cd terminumd && cargo clippy --locked --all-targets -- -D warnings"),
             ("panel", "cd ui && node test/e2e_panel.mjs"),
             ("live", "node web/test/e2e_live.mjs"),
             *((p.stem, f"python3 scripts/{p.name}") for p in e2e),
             ("docker", f"python3 scripts/check_docker.py {ident}"),
             ("install", "TERMINUMD_BIN=terminumd/target/debug/terminumd bash scripts/e2e_install.sh")]
    if platform.system() == "Darwin":   # e2e_all.mjs measures a Mac's browsers
        found.append(("page", "node web/test/e2e_all.mjs"))
    return found + [("security", "bash scripts/security_check.sh")]


def allowed_env(source=os.environ):
    return {**{k: source[k] for k in ALLOWED if k in source}, "TM_TIMING": "report"}


def is_gate_doc(path):
    """Docs that change no test's verdict: not a file the package carries, nor an agent's skill or setting."""
    return is_doc(path) and path not in COPY and not path.startswith((".claude/", "skills/"))


def docker_can_run():
    return subprocess.run([sys.executable, "scripts/check_docker.py", "--can-run"], cwd=REPO, capture_output=True).returncode == 0


def counts(notes, fresh_h=None):
    """Whether a kept pass still counts: young enough, and a Docker step that skipped only while Docker still cannot run."""
    if notes is None or fresh_h is not None and not gate_record.young(notes, fresh_h):
        return False
    return not notes.get("skip") or not docker_can_run()


def docs_only_since(root, fresh_h=None):
    """The commit with a full pass from which this content differs only in docs, or None."""
    found = gate_record.newest_full_pass(root, "HEAD", fresh_h)
    return found[0] if found and all(is_gate_doc(f) for f in changed_files(found[0], cwd=root)[0]) else None


def attempt(root, log, command, env):
    """One run of a command under the guard, its output in log: (exit code, seconds)."""
    started = time.monotonic()
    with bounded_log(root, log) as out:
        code = run_guard([sys.executable, "-I", str(root / "scripts" / "check_step.py")], ["sh", "-c", command],
                         cwd=root, stdout=out, env=env)
    return code, round(time.monotonic() - started)


def said(name, entry, log):
    if entry["result"] == "pass":
        print(f"ok   {name} ({entry['seconds']} s)" + (" on its retry" if entry.get("retried") else ""), flush=True)
    else:
        print(f"FAIL {name} ({entry['seconds']} s): {log}\n{tail(log)}", end="", flush=True)


def plan(root, ident, force, fresh_h):
    """([(name, command)] to run, {name: notes} of the passes kept, the docs-only base or None); skips said."""
    todo, kept, docs = [], {}, None if force else docs_only_since(root, fresh_h)
    for name, command in steps(root, ident):
        notes = None if force else gate_record.passed(root, ident, name)
        if counts(notes, fresh_h):
            print(f"skip {name}: passed on {notes.get('at', '?')}" + (f" (SKIP docker: {notes['skip']})" if notes.get("skip") else ""))
            kept[name] = notes
        elif docs and name != "security":
            print(f"skip {name}: a docs-only change since {docs[:9]}'s full pass")
            kept[name] = {}
        else:
            todo.append((name, command))
    return todo, kept, docs


def run_steps(root, ident, todo, env, retry):
    """Each step once, in order, then each red step of real programs once more: {name: its record entry}."""
    runs, again = {}, {}
    for name, command in todo:
        log, began = root / ".checks" / f"{ident}.{name}.log", time.time()
        code, seconds = attempt(root, log, command, env)
        runs[name] = {"result": "pass" if code == 0 else "fail", "seconds": seconds}
        if code == LEFT:
            runs[name]["cause"] = "a leftover (AC-102)"
        elif code:
            again[name] = (command, began)
        said(name, runs[name], log)
    for name, (command, began) in again.items():
        entry, log = runs[name], root / ".checks" / f"{ident}.{name}.log"
        first = log.with_name(f"{ident}.{name}.first.log")
        preserve_attempt(log, first)
        command, checks, notes = retry.decide(name, command, first, began)
        entry.update(notes)
        if not command:
            preserve_attempt(first, log)
            continue
        code, seconds = attempt(root, log, command, env)
        entry["seconds"] += seconds
        if code == 0:
            entry.update(result="pass", retried=[f"{name} {c}" for c in checks])
            entry.pop("cause", None)
            retry.passed_on_retry(name, checks, first)
        said(name, entry, log)
    for name, entry in runs.items():
        log = root / ".checks" / f"{ident}.{name}.log"
        if timing := [line.strip() for line in gate_retry.lines(log) if line.startswith("TIMING over budget")]:
            entry["timing"] = timing
    return runs


def keep(root, ident, runs, kept):
    """Each green step's pass; the whole run's notes, its steps': what was retried, a budget over, Docker's skip."""
    for name, entry in runs.items():
        if entry["result"] != "pass":
            gate_record.forget(root, ident, name)
            continue
        kept[name] = {k: entry[k] for k in ("retried", "timing") if k in entry}
        skip = root / ".checks" / f"{ident}.docker-skip"
        if name == "docker" and skip.exists():
            kept[name]["skip"] = skip.read_text().strip()
        gate_record.keep(root, ident, kept[name], name)
    whole = {"retried": [r for n in kept.values() for r in n.get("retried", [])],
             "timing": [t for n in kept.values() for t in n.get("timing", [])],
             "skip": next((n["skip"] for n in kept.values() if n.get("skip")), None)}
    return {k: v for k, v in whole.items() if v}


def wait_turn(turn, root, by):
    """The full run's turn and a quiet machine (AC-146): None to go on, else why not."""
    if by == "release" and not turn.take() and turn.holder().split(" ", 1)[0] == "rehearse":
        print(f"release: {turn.holder()}'s full run holds the turn: waiting for its end, its passes reused", flush=True)
        turn.take(wait=True)
    if why := load_gate.wait(busy=turn.busy):
        print(f"check: the machine is busy: {why} {load_gate.MINUTES}: nothing ran", file=sys.stderr)
        return why
    turn.hold_as(f"{by} {git('rev-parse', '--short=9', 'HEAD', cwd=root)}")
    return None


def gate(root, ident, by, force, fresh_h):
    todo, kept, docs = plan(root, ident, force, fresh_h)
    retry, runs, began = gate_retry.Retry(root), {}, time.monotonic()
    if todo:
        with load_gate.Turn(root) as turn:
            if wait_turn(turn, root, by):
                return load_gate.BUSY
            if by == "release":   # what the wait may have brought: a rehearsal's passes
                todo, kept, docs = plan(root, ident, force, fresh_h)
            began, env = time.monotonic(), allowed_env()
            for log in (root / ".checks").glob("*.log*"):   # earlier contents' logs; their passes stay
                if not log.name.startswith((f"{ident}.", f".{ident}.", "changed", "timing.")):
                    log.unlink(missing_ok=True)
            warn_setup(root)
            if any(name != "security" for name, _ in todo):   # what the steps read, which no pass covers
                log = root / ".checks" / f"{ident}.build.log"
                code, seconds = attempt(root, log, BUILD, env)
                built = {"result": "pass" if code == 0 else "fail", "seconds": seconds}
                said("build", built, log)
                if code:
                    runs["build"] = built
            if not runs:
                runs = run_steps(root, ident, todo, env, retry)
    return finish(root, ident, by, runs, kept, docs, round(time.monotonic() - began), retry)


def finish(root, ident, by, runs, kept, docs, seconds, retry):
    whole = keep(root, ident, runs, kept)
    failed = [name for name, entry in runs.items() if entry["result"] != "pass"]
    changed = not failed and tree_id(root) != ident
    if not failed and not changed:
        gate_record.keep(root, ident, {**whole, **({"docs_only_since": docs} if docs else {})})
    head = git("rev-parse", "HEAD", cwd=root)
    gate_record.append(root, {"kind": "gate", "by": by, "content": ident,
                              "commit": head if ident == gate_record.content_of(root, head) else None,
                              "result": "fail" if failed else "changed" if changed else "pass", "ran": len(runs),
                              "seconds": seconds, "steps": runs})
    retry.say_last()
    if failed:
        print(f"check: FAIL on {ident}: {' '.join(failed)}")
        return 1
    if changed:
        print(f"check: the tree changed during the run ({ident} → {tree_id(root)}): no pass recorded")
        return 1
    print(f"check: PASS on {ident} (remote changes also: scripts/e2e_remote.py; install changes: scripts/e2e_install.sh; "
          "macOS-only code: make test-mac)")
    return 0


def warn_setup(root):
    """The commit-time scan (CLAUDE.md §2) only knows other machines' names from the patterns file."""
    if not git("config", "core.hooksPath", cwd=root, check=False).stdout.strip():
        print("WARN no git hooks: make hooks (security_check before each commit)")
    config = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
    if not os.environ.get("TM_PRIVATE_PATTERNS") and not (config / "terminum" / "private-patterns").is_file():
        print("WARN no private patterns: ~/.config/terminum/private-patterns (other machines' names)")


def timing(root):
    """The checks with budgets alone, judged (no TM_TIMING), after a quiet-machine wait: the nightly's last step."""
    if load_gate.main([]) == load_gate.BUSY:
        return load_gate.BUSY
    env = {k: v for k, v in allowed_env().items() if k != "TM_TIMING"}
    failed = []
    for name, command in [("build", BUILD), *TIMED]:
        log = root / ".checks" / f"timing.{name}.log"
        code, seconds = attempt(root, log, command, env)
        said(name, {"result": "pass" if code == 0 else "fail", "seconds": seconds}, log)
        if code:
            failed.append(name)
            if name == "build":
                break
    print(red(f"timing: FAIL {' '.join(failed)}") if failed else "timing: PASS")
    return 1 if failed else 0


def main(argv):
    parser = argparse.ArgumentParser(prog="check_all.py")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--passed", action="store_true")
    parser.add_argument("--timing", action="store_true")
    parser.add_argument("--by", choices=("check", "release", "rehearse"), default="check")
    args = parser.parse_args(argv)
    root = REPO
    (root / ".checks").mkdir(exist_ok=True)
    if args.timing:
        return timing(root)
    ident = tree_id(root)
    whole = gate_record.passed(root, ident)
    force, fresh_h = args.force or args.by == "rehearse", FRESH_H if args.by == "release" else None   # a rehearsal tests the machine too
    if args.passed:   # a pass without Docker counts while Docker still cannot run
        return 0 if counts(whole, fresh_h) else 1
    if not force and counts(whole, fresh_h):
        if whole.get("skip"):
            print(f"SKIP docker: {whole['skip']}")
        print(f"check: passed already on {ident} at {whole.get('at', '?')} (--force runs it again)")
        return 0
    return gate(root, ident, args.by, force, fresh_h)


if __name__ == "__main__":
    sys.stdout.reconfigure(line_buffering=True)
    sys.exit(main(sys.argv[1:]))
