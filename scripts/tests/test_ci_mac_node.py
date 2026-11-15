# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-104: execute Node selection and SQLite preflight without changing a machine's runtime."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from testrun import short_tmp

REPO = Path(__file__).resolve().parents[2]
NODE = """#!/bin/bash
if [ "$1" = --version ]; then echo "${NODE_VERSION:-v26.7.0}"; exit "${VERSION_RC:-0}"; fi
echo "probe:$*" >> "$LOG"
exit "${PROBE_RC:-0}"
"""


class CiMacNodeTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="tm-node-", dir=short_tmp())
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.command(self.bin, "dirname", '#!/bin/sh\nexec /usr/bin/dirname "$@"\n')
        self.command(self.bin, "brew", '#!/bin/bash\necho "brew:$*" >> "$LOG"\n[ -n "${BREW_PREFIX:-}" ] || exit 1\necho "$BREW_PREFIX"\n')

    def command(self, folder, name, text):
        folder.mkdir(exist_ok=True, parents=True)
        file = folder / name
        file.write_text(text)
        file.chmod(0o755)

    def select(self, override="", **env):
        return subprocess.run(["/bin/bash", "-c", 'source scripts/ci_mac_node.sh "$1" || exit $?; node --version; /usr/bin/env node --version',
                               "ci_mac", str(override)], cwd=REPO, capture_output=True, text=True, timeout=10,
                              env={**os.environ, "PATH": str(self.bin), "LOG": str(self.root / "log"),
                                   "BREW_PREFIX": "", "NODE_VERSION": "v26.7.0", "VERSION_RC": "0", "PROBE_RC": "0", **env})

    def test_supported_path_node_is_kept_for_children(self):
        self.command(self.bin, "node", NODE)
        result = self.select()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"using v26.7.0 at {self.bin}/node", result.stdout)
        self.assertTrue(result.stdout.endswith("v26.7.0\nv26.7.0\n"))
        self.assertEqual((self.root / "log").read_text(), "probe:scripts/ci_mac_sqlite.mjs\n")

    def test_missing_unsupported_or_broken_node_is_a_prerequisite_error(self):
        for version in (None, "v24.3.0", "v25.0.0", "v27.0.0", "v126.0.0"):
            with self.subTest(version=version):
                if version:
                    self.command(self.bin, "node", NODE)
                result = self.select(NODE_VERSION=version or "")
                self.assertEqual(result.returncode, 4, result.stderr)
                self.assertIn("Node 26 required", result.stderr)
                self.assertIn("TM_MAC_NODE_BIN", result.stderr)
        self.assertEqual(self.select(VERSION_RC="1").returncode, 4)
        self.assertNotIn("probe:", (self.root / "log").read_text())

    def test_installed_homebrew_keg_replaces_an_unsupported_path_node_for_this_run(self):
        self.command(self.bin, "node", NODE.replace("v26.7.0", "v24.3.0"))
        keg = self.root / "keg"
        self.command(keg / "bin", "node", NODE)
        result = self.select(BREW_PREFIX=str(keg), NODE_VERSION="")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"at {keg}/bin/node", result.stdout)
        self.assertIn("brew:--prefix node@26", (self.root / "log").read_text())
        self.assertTrue(result.stdout.endswith("v26.7.0\nv26.7.0\n"))

    def test_explicit_directory_is_literal_and_authoritative(self):
        chosen = self.root / "Node 26's bin"
        self.command(chosen, "node", NODE)
        self.command(self.bin, "node", NODE.replace("v26.7.0", "v24.3.0"))
        result = self.select(chosen)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"at {chosen}/node", result.stdout)
        self.assertNotIn("brew:", (self.root / "log").read_text())

    def test_bad_explicit_directory_never_falls_back(self):
        self.command(self.bin, "node", NODE)
        old = self.root / "old"
        self.command(old, "node", NODE.replace("v26.7.0", "v24.3.0"))
        for folder in ("relative/bin", self.root / "absent", old):
            with self.subTest(folder=folder):
                result = self.select(folder, NODE_VERSION="")
                self.assertEqual(result.returncode, 4, result.stderr)
                self.assertIn("Node 26 required", result.stderr)
        self.assertFalse((self.root / "log").exists())

    def test_sqlite_failure_is_a_prerequisite_error(self):
        self.command(self.bin, "node", NODE)
        result = self.select(PROBE_RC="1")
        self.assertEqual(result.returncode, 4, result.stderr)
        self.assertIn("working node:sqlite numbered parameters required", result.stderr)
        self.assertNotIn("v26.7.0\nv26.7.0", result.stdout)

    def test_real_sqlite_probe_on_supported_node(self):
        node = shutil.which("node")
        version = subprocess.check_output([node, "--version"], text=True) if node else ""
        if not version.startswith("v26."):
            self.skipTest("real SQLite probe requires Node 26; selection tests use stubs")
        result = subprocess.run([node, "scripts/ci_mac_sqlite.mjs"], cwd=REPO, capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("numbered parameters passed", result.stdout)


if __name__ == "__main__":
    unittest.main()
