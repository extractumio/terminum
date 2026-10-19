#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Build the Mac package (AC-08, AC-86): one folder, terminum/, that installs itself. Nothing
in it needs an interpreter: the Mac's terminumd is the installer, the server and the
`terminum` command (`terminumd install`; install.sh runs it).

    BUILD, AGENT_ID                   which build this is; which terminumd sources
    agents/<platform>/terminumd       terminumd (the page embedded) for macOS arm64/x86_64 and
                                      Linux x86_64/arm64, the Linux pair for remote hosts' files
    release-signers, LICENSE files    the release key upgrades check; the licenses, NOTICE (marks)

The Mac's own terminumd is the one of its architecture (chosen at install). `--stage`
leaves the folder for `make install`; without it a tarball and SHA256SUMS are written too,
plus install.sh, for a release. `--bin FILE` packs an already built terminumd for this Mac
only (scripts/e2e_install.sh); `--out DIR` writes elsewhere than dist/package. `id` prints
the build id of this checkout's sources (the Makefile compiles it into terminumd).

    python3 scripts/package.py [--stage] [--build ID] [--bin FILE] [--out DIR]
    python3 scripts/package.py id
"""
import argparse
import hashlib
import os
import shutil
import subprocess
import time
import sys
import tarfile
from pathlib import Path

import agents

REPO = agents.REPO
OUT = REPO / "dist/package"
NAME = "terminum"
TARBALL = "terminum-macos.tar.gz"
COPY = ["release-signers", "LICENSE", "LICENSE-COMMERCIAL.md", "NOTICE", "README.md"]
# every source that goes into a build (its id); build output never counts
SOURCES = ["terminumd/src", "terminumd/assets", "terminumd/Cargo.toml", "terminumd/Cargo.lock",
           "ui/src", "ui/public", "ui/build.mjs", "ui/package.json", "ui/package-lock.json", "web/static",
           "release-signers"]


def build_id(repo=REPO):
    """`<commit>-<hash of every source that goes into a build>`: a change anywhere gives a
    new id; the same sources give the same id."""
    h = hashlib.sha256()
    for rel in SOURCES:
        root = repo / rel
        if root.is_file():
            files = [root]
        elif root.is_dir():
            files = sorted(p for p in root.rglob("*") if p.is_file() and "__pycache__" not in p.parts)
        else:
            continue
        for f in files:
            h.update(str(f.relative_to(repo)).encode() + b"\0" + f.read_bytes() + b"\0")
    r = subprocess.run(["git", "-C", str(repo), "rev-parse", "--short", "HEAD"], capture_output=True, text=True)
    return f"{r.stdout.strip() or 'nogit'}-{h.hexdigest()[:8]}"


def build_macos(build, platforms):
    """terminumd for Mac architectures, compiled with this build id (the installer checks it)."""
    env = agents.tool_env()
    env.update(TM_BUILD=build, TM_AGENT_ID=agents.agent_id(), TM_BUILD_DATE=time.strftime("%Y-%m-%d"))
    out = {}
    for platform in platforms:
        target = agents.TARGETS[platform]
        agents.run(["cargo", "build", "--locked", "--release", "--target", target], env=env)
        out[platform] = agents.TERMINUMD / "target" / target / "release/terminumd"
    return out


def binaries(build, given, stage):
    """{platform: terminumd}: `given` for this Mac only; for `make install` (stage) this
    Mac's own build; for a release both Mac builds; the Linux pair when the toolchain is there."""
    if given:
        return {agents.mac_platform(): Path(given)}
    found = build_macos(build, [agents.mac_platform()] if stage else ["macos-aarch64", "macos-x86_64"])
    try:
        found.update(agents.build(["linux-x86_64", "linux-aarch64"]))
    except agents.Failed as e:  # a developer without the cross toolchain: Mac only, said loudly
        print(f"warning: no Linux helpers in this package ({e}); remote hosts' files cannot be enabled. "
              "Run make toolchain.", file=sys.stderr)
    return found


def stage(build, out, given=None, only_stage=False):
    pkg = out / NAME
    shutil.rmtree(pkg, ignore_errors=True)
    pkg.mkdir(parents=True)
    for rel in COPY:
        shutil.copy2(REPO / rel, pkg / rel)
    for platform, path in binaries(build, given, only_stage).items():
        (pkg / "agents" / platform).mkdir(parents=True)
        shutil.copy2(path, pkg / "agents" / platform / "terminumd")
        os.chmod(pkg / "agents" / platform / "terminumd", 0o755)
    (pkg / "BUILD").write_text(build + "\n")
    (pkg / "AGENT_ID").write_text(agents.agent_id() + "\n")
    return pkg


def anonymous(info):
    """No owner names or ids and one time in the tarball: it says nothing of who built it."""
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    info.mtime = 0
    return info


def archive(pkg, out):
    tar = out / TARBALL
    with tarfile.open(tar, "w:gz") as t:
        t.add(pkg, arcname=NAME, filter=anonymous)
    shutil.copy2(REPO / "scripts/install.sh", out / "install.sh")
    # the signed list names its release, so an older one cannot be served as the latest
    sums = f"# release {(pkg / 'BUILD').read_text().strip()}\n" + "".join(
        f"{hashlib.sha256((out / f).read_bytes()).hexdigest()}  {f}\n" for f in (TARBALL, "install.sh"))
    (out / "SHA256SUMS").write_text(sums)
    return [tar, out / "install.sh", out / "SHA256SUMS"]


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if argv[:1] == ["id"]:
        return print(build_id())
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--stage", action="store_true", help="only the folder (make install)")
    p.add_argument("--build", default=os.environ.get("TM_BUILD"))
    p.add_argument("--bin", help="an already built terminumd for this Mac only")
    p.add_argument("--out", type=Path, default=OUT)
    a = p.parse_args(argv)
    if not a.build:
        sys.exit("no build id: set TM_BUILD or --build")
    if a.bin and not Path(a.bin).is_file():
        sys.exit(f"no terminumd at {a.bin}")
    out = a.out.resolve()
    try:
        pkg = stage(a.build, out, a.bin, a.stage)
        print(pkg)
        if not a.stage:
            for f in archive(pkg, out):
                print(f)
    except agents.Failed as e:
        sys.exit(str(e))
    return pkg


if __name__ == "__main__":
    main()
