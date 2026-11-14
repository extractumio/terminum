# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-196: check logs use exactly the daemon's Rust rotation core, with no Cargo bootstrap."""
from contextlib import contextmanager
import hashlib
import os
from pathlib import Path
import subprocess
import sys


DRAIN_SECONDS = 10


def sink(root):
    source_root = Path(__file__).resolve().parent.parent
    sources = [source_root / name for name in ("scripts/log_sink.rs", "terminumd/src/logs/rotation.rs", "terminumd/src/fslock.rs")]
    digest = hashlib.sha256(b"".join(path.read_bytes() for path in sources)).hexdigest()[:16]
    folder = root / ".checks"
    folder.mkdir(exist_ok=True)
    binary = folder / "log-sink"
    stamp = folder / "log-sink.source"
    if not binary.is_file() or not stamp.is_file() or stamp.read_text() != digest:
        temp = folder / f"log-sink.{os.getpid()}.new"
        subprocess.run(["rustc", "--edition=2024", "-O", str(sources[0]), "-o", str(temp)], check=True)
        temp.replace(binary)
        stamp.write_text(digest)
    return binary


@contextmanager
def bounded_log(root, path):
    """The parent owns and reaps the sink on success, failure, cancellation and exceptions."""
    proc = subprocess.Popen([str(sink(root)), str(path)], stdin=subprocess.PIPE)
    try:
        yield proc.stdin
    finally:
        aborting = sys.exc_info()[0] is not None
        try:
            proc.stdin.close()
        except BrokenPipeError:
            pass
        try:
            status = proc.wait(timeout=DRAIN_SECONDS)
        except subprocess.TimeoutExpired:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
            status = proc.returncode
            if not aborting:
                raise RuntimeError(f"check log sink did not finish: {path}")
        if status and not aborting:
            raise RuntimeError(f"check log sink failed ({status}): {path}")


def tail(path, lines=15):
    """Only the tail of each bounded segment is needed for a failed-step report."""
    kept = []
    for part in (Path(str(path) + ".2"), Path(str(path) + ".1"), path):
        try:
            with part.open("rb") as stream:
                stream.seek(0, 2)
                stream.seek(max(0, stream.tell() - 16_384))
                kept.extend(stream.read().decode(errors="replace").splitlines()[-lines:])
            kept = kept[-lines:]
        except FileNotFoundError:
            pass
    return kept


def main(argv):
    if len(argv) == 2 and argv[0] == "--tail":
        for line in tail(Path(argv[1])):
            print("     " + line)
        return 0
    if len(argv) < 2:
        raise SystemExit("usage: check_logs.py <log> <command...> | --tail <log>")
    import sys
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from check_cancel import run_guard
    root = Path(__file__).resolve().parent.parent
    with bounded_log(root, Path(argv[0])) as out:
        return run_guard([sys.executable, "-I", str(root / "scripts/check_step.py")], argv[1:], cwd=root, stdout=out)


if __name__ == "__main__":
    import sys
    sys.exit(main(sys.argv[1:]))
