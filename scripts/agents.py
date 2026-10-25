#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Build terminumd for remote hosts (AC-86): one script on the Mac and on the runner. On a host
terminumd runs as the files agent (`terminumd --agent`).

Apple targets build with cargo; Linux targets build static (musl) with cargo-zigbuild and
zig from the `ziglang` wheel, both pinned and kept in ./.toolchain. Output goes to
dist/agents/<agent id>/<platform>/terminumd and is reused while the agent id (a hash of
terminumd's sources and manifests) is unchanged.

    python3 scripts/agents.py id                       print the agent id
    python3 scripts/agents.py toolchain                install the pinned toolchain (once)
    python3 scripts/agents.py build [PLATFORM ...]     build (default: all four)
"""
import argparse
import hashlib
import os
import platform as host_platform
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TERMINUMD = REPO / "terminumd"
# the runner keeps it in its home (TM_TOOLCHAIN): checkout cleans the repository every run
TOOLCHAIN = Path(os.environ.get("TM_TOOLCHAIN") or REPO / ".toolchain")
DIST = REPO / "dist/agents"
ZIG = "0.15.2"            # the ziglang wheel (zig as linker for the Linux targets)
ZIGBUILD = "0.23.4"       # cargo-zigbuild
TARGETS = {"macos-aarch64": "aarch64-apple-darwin", "macos-x86_64": "x86_64-apple-darwin",
           "linux-x86_64": "x86_64-unknown-linux-musl", "linux-aarch64": "aarch64-unknown-linux-musl"}


class Failed(Exception):
    pass


def agent_id(repo=REPO):
    """12 hex of sha256 over terminumd's sources and manifests: equal ids speak the same API."""
    h = hashlib.sha256()
    root = repo / "terminumd"
    for f in sorted([*(root / "src").rglob("*.rs"), root / "Cargo.toml", root / "Cargo.lock"]):
        if f.is_file():
            h.update(str(f.relative_to(repo)).encode() + b"\0" + f.read_bytes() + b"\0")
    return h.hexdigest()[:12]


def run(cmd, env=None, cwd=TERMINUMD):
    print("+", " ".join(map(str, cmd)), flush=True)
    r = subprocess.run(cmd, cwd=cwd, env=env)
    if r.returncode != 0:
        raise Failed(f"{cmd[0]} exited {r.returncode}")


def tool_env():
    """PATH with the pinned toolchain first (cargo-zigbuild finds zig through `python3 -m ziglang`)."""
    env = dict(os.environ)
    env["PATH"] = os.pathsep.join([str(TOOLCHAIN / "bin"), str(TOOLCHAIN / "venv/bin"), env.get("PATH", "")])
    return env


def toolchain(platforms):
    """Rust targets, then zig and cargo-zigbuild for the Linux targets, pinned, in ./.toolchain."""
    targets = [TARGETS[p] for p in platforms]
    if shutil.which("rustup"):
        run(["rustup", "target", "add", *targets])
    if any(p.startswith("linux-") for p in platforms):
        if not (TOOLCHAIN / "venv/bin/python3").exists():
            run([sys.executable, "-m", "venv", str(TOOLCHAIN / "venv")], cwd=REPO)
        run([str(TOOLCHAIN / "venv/bin/python3"), "-m", "pip", "install", "--quiet", f"ziglang=={ZIG}"], cwd=REPO)
        if not (TOOLCHAIN / "bin/cargo-zigbuild").exists():
            run(["cargo", "install", "--locked", "cargo-zigbuild", "--version", ZIGBUILD, "--root", str(TOOLCHAIN)], cwd=REPO)


def mac_platform():
    """This Mac's own architecture: under Rosetta, Python would say x86_64 on Apple Silicon."""
    r = subprocess.run(["sysctl", "-n", "hw.optional.arm64"], capture_output=True, text=True)
    return "macos-aarch64" if r.stdout.strip() == "1" or host_platform.machine() == "arm64" else "macos-x86_64"


def this_platform():
    """The platform of the terminumd this machine runs: a Mac's own architecture, or Linux's."""
    return mac_platform() if host_platform.system() == "Darwin" else native()


def native():
    """The platform this machine builds without cross-compiling."""
    system = {"Darwin": "macos", "Linux": "linux"}.get(host_platform.system(), "?")
    machine = {"arm64": "aarch64", "aarch64": "aarch64", "x86_64": "x86_64", "AMD64": "x86_64"}.get(host_platform.machine(), "?")
    return f"{system}-{machine}"


def build(platforms):
    """Build each platform's agent once per agent id; returns {platform: path}."""
    aid = agent_id()
    env = tool_env()
    env.update(TM_AGENT_ID=aid, TM_BUILD=env.get("TM_BUILD") or f"agent-{aid}")
    out = {}
    for p in platforms:
        if p not in TARGETS:
            raise Failed(f"unknown platform {p} (one of {', '.join(TARGETS)})")
        dest = DIST / aid / p / "terminumd"
        if not dest.is_file():
            target = TARGETS[p]
            if p.startswith("linux-"):  # the same way on the Mac and on the Linux runner
                if not (TOOLCHAIN / "bin/cargo-zigbuild").exists():
                    raise Failed("the Linux toolchain is missing: run make toolchain")
                run(["cargo", "zigbuild", "--locked", "--release", "--target", target], env=env)
            elif native().startswith("macos-"):
                run(["cargo", "build", "--locked", "--release", "--target", target], env=env)
            else:
                raise Failed(f"{p} builds only on macOS")
            dest.parent.mkdir(parents=True, exist_ok=True)
            tmp = dest.with_name(".terminumd.new")  # checked before it is cached under its id
            shutil.copy2(TERMINUMD / "target" / target / "release/terminumd", tmp)
            got = subprocess.run([str(tmp), "--agent-id"], capture_output=True, text=True).stdout.strip() if p == native() else aid
            if got != aid:
                tmp.unlink()
                raise Failed(f"{target} terminumd says agent id {got}, expected {aid}")
            tmp.replace(dest)
        out[p] = dest
    return out



def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("id")
    sub.add_parser("platform", help="this machine's platform, as a package names its terminumd")
    t = sub.add_parser("toolchain")
    t.add_argument("platforms", nargs="*")
    b = sub.add_parser("build")
    b.add_argument("platforms", nargs="*")
    a = p.parse_args(argv)
    try:
        if a.cmd == "id":
            return print(agent_id())
        if a.cmd == "platform":
            return print(this_platform())
        if a.cmd == "toolchain":
            return toolchain(a.platforms or list(TARGETS))
        for plat, path in build(a.platforms or list(TARGETS)).items():
            print(f"{plat}: {path.relative_to(REPO)}")
    except Failed as e:
        sys.exit(str(e))


if __name__ == "__main__":
    main()
