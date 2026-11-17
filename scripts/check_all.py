#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The full run of CLAUDE.md §8, the release's gate (spec AC-104, AC-146, AC-201), on a content: <id> names the
content of every tracked and new file (check_changed.tree_id), so committing a checked tree keeps its passes.
- A pass per step: a step passed on this content, in this tree or any of the repository (gate_record), is skipped and
  said; --force, and a rehearsal, run every step; --by release counts only passes younger than 48 hours. Content that differs only in
  docs (no file the package carries) from a commit whose content passed whole runs the scan alone.
- Before any step it builds what the steps read and no pass covers (the UI, terminumd's debug build).
- Each step runs under check_step.py's guard (AC-102), its output in .checks/<id>.<step>.log, with the environment
  ALLOWED alone and TM_TIMING=report: nothing of its caller's reaches a test, and a measured budget a check exceeds
  under this run's own load is said ("TIMING over budget"), judged by --timing on a quiet machine.
- Once every step had its first run, a red step of real programs runs its red checks once more (gate_retry.py).
- It starts on a quiet machine (scripts/load_gate.py, AC-146): exit 75 when the load stays high for 15 minutes; a
  release or a rehearsal waited already and says so with --by. Every run leaves a line in the record (gate_record).

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
from check_changed import changed_files, is_doc, red, tree_id  # noqa: E402
from check_logs import bounded_log, preserve_attempt, tail  # noqa: E402
from check_step import LEFT  # noqa: E402
import gate_record  # noqa: E402
import gate_retry  # noqa: E402
import load_gate  # noqa: E402
from package import COPY  # noqa: E402

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
    e2e = sorted(str(p.relative_to(root)) for p in root.glob("scripts/e2e_*.py") if "remote_sshd" not in p.name)
    found = [("test", "make test"),
             ("clippy", "cd terminumd && cargo clippy --locked --all-targets -- -D warnings"),
             ("panel", "cd ui && node test/e2e_panel.mjs"),
             ("live", "node web/test/e2e_live.mjs"),
             ("e2e", gate_retry.e2e_command(e2e)),   # but Docker's, which take built binaries: their own step
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


def counts(notes, fresh_h=None):
    """Whether a kept pass still counts: young enough, and a Docker step that skipped only while Docker still cannot run."""
    if notes is None or fresh_h is not None and not gate_record.young(notes, fresh_h):
        return False
    return not notes.get("skip") or not docker_can_run()


def docker_can_run():
    return subprocess.run([sys.executable, "scripts/check_docker.py", "--can-run"], cwd=REPO, capture_output=True).returncode == 0


def docs_only_since(root, fresh_h=None):
    """The commit with a full pass from which this content differs only in docs, or None."""
    found = gate_record.newest_full_pass(root, "HEAD", fresh_h)
    if not found:
        return None
    files, _ = changed_files(found[0], cwd=root)
    return found[0] if all(is_gate_doc(f) for f in files) else None


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
        print(f"FAIL {name} ({entry['seconds']} s): {log}", flush=True)
        print("".join(f"     {line}\n" for line in tail(log)), end="", flush=True)


def plan(root, ident, force, fresh_h):
    """[(name, command)] to run; what is skipped is said."""
    docs = None if force else docs_only_since(root, fresh_h)
    todo = []
    for name, command in steps(root, ident):
        notes = None if force else gate_record.passed(root, ident, name)
        if counts(notes, fresh_h):
            print(f"skip {name}: passed on {notes.get('at', '?')}" + (f" (SKIP docker: {notes['skip']})" if notes.get("skip") else ""))
        elif docs and name != "security":
            print(f"skip {name}: a docs-only change since {docs[:9]}'s full pass")
        else:
            todo.append((name, command))
    return todo, docs


def first_runs(root, ident, todo, env):
    """Each step once, in order: {name: (entry, command, its log, when it began)}."""
    runs = {}
    for name, command in todo:
        log, began = root / ".checks" / f"{ident}.{name}.log", time.time()
        code, seconds = attempt(root, log, command, env)
        entry = {"result": "pass" if code == 0 else "fail", "seconds": seconds}
        if code == LEFT:
            entry["cause"] = "a leftover (AC-102)"
        said(name, entry, log)
        runs[name] = (entry, command, log, began, code)
    return runs


def retries(root, ident, runs, env, retry):
    """The red steps of real programs, once more each, after every first run."""
    for name, (entry, command, log, began, code) in runs.items():
        if code in (0, LEFT):
            continue
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


def keep(root, ident, runs):
    for name, (entry, *_rest) in runs.items():
        timing = [line.strip() for line in tail(root / ".checks" / f"{ident}.{name}.log", 400) if line.startswith("TIMING over budget")]
        if timing:
            entry["timing"] = timing
        if entry["result"] != "pass":
            gate_record.forget(root, ident, name)
            continue
        notes = {k: entry[k] for k in ("retried", "timing") if k in entry}
        skip = root / ".checks" / f"{ident}.docker-skip"
        if name == "docker" and skip.exists():
            notes["skip"] = skip.read_text().strip()
        gate_record.keep(root, ident, notes, name)


def gate(root, ident, args):
    fresh_h = FRESH_H if args.by == "release" else None
    todo, docs = plan(root, ident, args.force or args.by == "rehearse", fresh_h)   # a rehearsal tests the machine too
    if todo and args.by == "check" and load_gate.main([]) == load_gate.BUSY:
        return load_gate.BUSY
    began = time.monotonic()
    for log in (root / ".checks").glob("*.log*"):   # earlier contents' logs; their passes stay
        if not log.name.startswith((f"{ident}.", f".{ident}.", "changed", "timing.")):
            log.unlink(missing_ok=True)
    warn_setup(root)
    env, retry = allowed_env(), gate_retry.Retry(root, ident)
    if any(name != "security" for name, _ in todo):   # what the steps read, which no pass covers
        todo = [("build", BUILD)] + todo
    runs = first_runs(root, ident, todo, env)
    retries(root, ident, runs, env, retry)
    keep(root, ident, {k: v for k, v in runs.items() if k != "build"})
    failed = [name for name, (entry, *_rest) in runs.items() if entry["result"] != "pass"]
    changed = not failed and tree_id(root) != ident
    if not failed and not changed:
        notes = gate_retry.whole_notes(root, ident, [name for name, _ in steps(root, ident)])
        gate_record.keep(root, ident, {**notes, **({"docs_only_since": docs} if docs else {})})
    head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, capture_output=True, text=True).stdout.strip()
    clean = not subprocess.run(["git", "status", "--porcelain"], cwd=root, capture_output=True, text=True).stdout.strip()
    gate_record.append(root, {"kind": "gate", "by": args.by, "content": ident, "commit": head if clean else None,
                              "result": "fail" if failed else "changed" if changed else "pass", "ran": len(runs),
                              "seconds": round(time.monotonic() - began), "steps": {k: v[0] for k, v in runs.items()}})
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
    if not subprocess.run(["git", "config", "core.hooksPath"], cwd=root, capture_output=True, text=True).stdout.strip():
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
    fresh_h = FRESH_H if args.by == "release" else None
    if args.passed:   # a pass without Docker counts while Docker still cannot run
        return 0 if counts(whole, fresh_h) else 1
    if not args.force and args.by != "rehearse" and counts(whole, fresh_h):
        if whole.get("skip"):
            print(f"SKIP docker: {whole['skip']}")
        print(f"check: passed already on {ident} at {whole.get('at', '?')} (--force runs it again)")
        return 0
    return gate(root, ident, args)


if __name__ == "__main__":
    sys.stdout.reconfigure(line_buffering=True)
    sys.exit(main(sys.argv[1:]))
