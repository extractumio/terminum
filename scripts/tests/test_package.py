# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-08 / AC-86: the package holds terminumd and data only (no script for an interpreter),
its tarball names no owner, and the build id follows every packaged source."""
import os
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import package  # noqa: E402


class PackageTest(unittest.TestCase):
    def test_a_staged_package_is_terminumd_and_data_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "terminumd"
            binary.write_text("#!/bin/sh\n")
            pkg = package.stage("v9.0.0", Path(tmp) / "out", given=str(binary), only_stage=True)
            files = sorted(str(p.relative_to(pkg)) for p in pkg.rglob("*") if p.is_file())
            self.assertIn(f"agents/{package.agents.mac_platform()}/terminumd", files)
            self.assertIn("release-signers", files)
            self.assertEqual((pkg / "BUILD").read_text(), "v9.0.0\n")
            self.assertFalse([f for f in files if f.endswith((".py", ".sh")) or f == "terminum"], files)

    def test_the_package_tarball_names_no_owner(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            pkg = root / "out/terminum"
            (pkg / "agents/macos-aarch64").mkdir(parents=True)
            (pkg / "BUILD").write_text("v9.0.0\n")
            (pkg / "agents/macos-aarch64/terminumd").write_text("x")
            package.archive(pkg, root / "out")
            with tarfile.open(root / "out" / package.TARBALL) as t:
                for m in t.getmembers():
                    self.assertEqual((m.uid, m.gid, m.uname, m.gname, m.mtime), (0, 0, "", "", 0), m.name)
            self.assertTrue((root / "out/SHA256SUMS").read_text().startswith("# release v9.0.0\n"))


class BuildIdTest(unittest.TestCase):
    def test_every_packaged_source_changes_the_id_and_build_output_does_not(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            inputs = ["terminumd/src/main.rs", "terminumd/src/cli/mod.rs", "terminumd/Cargo.lock",
                      "terminumd/assets/themes/local.json", "ui/src/main.ts", "ui/package.json", "web/static/app.js",
                      "release-signers"]
            for rel in inputs:
                (root / rel).parent.mkdir(parents=True, exist_ok=True)
                (root / rel).write_text("original")
            env = {**os.environ, "HOME": str(root / "home"), "PYTHONPATH": str(Path(package.__file__).parent)}
            code = "import package,sys; from pathlib import Path; print(package.build_id(Path(sys.argv[1])))"

            def build_id():
                return subprocess.run([sys.executable, "-c", code, str(root)], env=env,
                                      check=True, capture_output=True, text=True).stdout.strip()

            before = build_id()
            self.assertEqual(before, build_id(), "same sources, same id")
            for rel in inputs:
                with self.subTest(path=rel):
                    (root / rel).write_text("changed")
                    after = build_id()
                    self.assertNotEqual(before, after)
                    before = after
            for generated in ("ui/dist/main.js", "terminumd/target/release/terminumd", "ui/.build-id"):
                (root / generated).parent.mkdir(parents=True, exist_ok=True)
                (root / generated).write_text("generated")
            self.assertEqual(before, build_id())


if __name__ == "__main__":
    unittest.main()
