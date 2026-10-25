# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-08 / AC-86: the package holds terminumd and data only (no script for an interpreter),
its tarball names no owner, and the build id follows every packaged source."""
import os
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest import mock
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
            self.assertIn(f"agents/{package.agents.this_platform()}/terminumd", files)
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
            # one package for every OS, under the Mac's old name too (installs before Linux ask for it)
            sums = dict(reversed(l.split("  ")) for l in (root / "out/SHA256SUMS").read_text().splitlines()[1:])
            self.assertEqual(sums[package.TARBALL], sums[package.MAC_TARBALL])
            self.assertEqual(package.TARBALL, "terminum.tar.gz")

    def test_a_release_without_the_linux_builds_fails(self):
        with mock.patch.object(package, "build_macos", return_value={}), \
             mock.patch.object(package.agents, "this_platform", return_value="macos-aarch64"), \
             mock.patch.object(package.agents, "build", side_effect=package.agents.Failed("no zig")):
            self.assertEqual(package.binaries("v9", None, False), {}, "make install: said, not refused")
            with self.assertRaises(package.agents.Failed):
                package.binaries("v9", None, False, release=True)


class SigningTest(unittest.TestCase):
    """AC-08: macOS terminumd is signed as io.terminum.terminumd, with the maintainer's Developer ID
    only when TM_CODESIGN_IDENTITY names it; a release refuses less."""

    def ran(self, code=0, err=""):
        return mock.Mock(returncode=code, stderr=err, stdout="")

    def test_the_identity_is_only_ever_the_one_named(self):
        with mock.patch.dict(os.environ, {"TM_CODESIGN_IDENTITY": ""}), mock.patch("sys.stderr") as said:
            self.assertEqual(package.signer(False), "-")
            self.assertIn("signed ad hoc", "".join(c.args[0] for c in said.write.call_args_list))
            with self.assertRaises(package.agents.Failed) as e:
                package.signer(True)
            self.assertIn("TM_CODESIGN_IDENTITY is not set", str(e.exception))
        with mock.patch.dict(os.environ, {"TM_CODESIGN_IDENTITY": "Developer ID Application: Alex (TEAM)"}):
            self.assertEqual(package.signer(True), "Developer ID Application: Alex (TEAM)")

    def test_codesign_names_the_identifier_and_a_release_checks_the_team(self):
        b = Path("/tmp/x/terminumd")
        with mock.patch.object(package.subprocess, "run", return_value=self.ran()) as run:
            package.codesign(b, "-", False)
        self.assertEqual(run.call_args.args[0], ["codesign", "--force", "-s", "-", "-i", "io.terminum.terminumd", "--timestamp=none", str(b)])
        signed = "Identifier=io.terminum.terminumd\nTeamIdentifier=ABCDE12345\n"
        with mock.patch.object(package.subprocess, "run", side_effect=[self.ran(), self.ran(err=signed)]) as run:
            package.codesign(b, "Developer ID Application: Alex (TEAM)", True)
        self.assertIn("--timestamp", run.call_args_list[0].args[0])
        for info in ("Identifier=io.terminum.terminumd\nTeamIdentifier=not set\n", "Identifier=terminumd-1234\nTeamIdentifier=ABCDE12345\n"):
            with mock.patch.object(package.subprocess, "run", side_effect=[self.ran(), self.ran(err=info)]), \
                    self.assertRaises(package.agents.Failed):
                package.codesign(b, "Developer ID Application: Alex (TEAM)", True)

    def test_a_failed_or_hung_codesign_stops_the_package(self):
        b = Path("/tmp/x/terminumd")
        with mock.patch.object(package.subprocess, "run", return_value=self.ran(1, "errSecInternalComponent")), \
                self.assertRaises(package.agents.Failed) as e:
            package.codesign(b, "-", False)
        self.assertIn("errSecInternalComponent", str(e.exception))
        with mock.patch.object(package.subprocess, "run", side_effect=subprocess.TimeoutExpired("codesign", 60)), \
                self.assertRaises(package.agents.Failed) as e:
            package.codesign(b, "-", False)
        self.assertIn("over 60 s", str(e.exception))

    @unittest.skipUnless(sys.platform == "darwin", "a package made on Linux holds that machine's terminumd only, never signed")
    def test_a_mach_o_is_signed_and_a_wrapper_script_is_not(self):
        with tempfile.TemporaryDirectory() as tmp:
            for body, signed in ((b"\xcf\xfa\xed\xfe" + b"\0" * 60, True), (b"#!/bin/sh\n", False)):
                binary = Path(tmp) / "terminumd"
                binary.write_bytes(body)
                with mock.patch.object(package, "codesign") as sign, mock.patch.dict(os.environ, {"TM_CODESIGN_IDENTITY": "x"}):
                    pkg = package.stage("v9.0.0", Path(tmp) / "out", given=str(binary), only_stage=True)
                self.assertEqual(sign.called, signed, body[:4])
                if signed:
                    self.assertEqual(sign.call_args.args, (pkg / "agents" / package.agents.mac_platform() / "terminumd", "x", False))


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
