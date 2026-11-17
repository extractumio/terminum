# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-86: `make release` publishes nothing until it has signed, never changes a published
release or ships an older version, signs without a prompt and leaves the key out of the
agent. Against a scratch git repository with its own "origin", a private ssh-agent with a
throwaway key, and a fake `gh` that records what it is asked (no network)."""
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release  # noqa: E402

FAKE_GH = """#!/bin/sh
printf '%s\n' "$(echo "$*" | tr '\n' ' ')" >> "$FAKE_GH_LOG"
case "$*" in
  "release view"*isDraft*) [ -n "$FAKE_GH_DRAFT" ] || exit 1; echo "$FAKE_GH_DRAFT" ;;
  "release view"*url*) echo "https://example.invalid/releases/$3" ;;
esac
"""


def git(cwd, *args):
    return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


class ReleaseTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="tmrel-", dir="/tmp"))  # short: the agent's socket path
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(self.tmp)]))
        origin, self.work = self.tmp / "origin.git", self.tmp / "work"
        git(self.tmp, "init", "-q", "--bare", "-b", "main", str(origin))
        git(self.tmp, "init", "-q", "-b", "main", str(self.work))
        for k, v in (("user.name", "t"), ("user.email", "t@example.invalid"), ("commit.gpgsign", "false"), ("tag.gpgsign", "false")):
            git(self.work, "config", k, v)
        (self.work / "f").write_text("1")
        git(self.work, "add", "f")
        git(self.work, "commit", "-qm", "one")
        git(self.work, "remote", "add", "origin", str(origin))
        git(self.work, "push", "-q", "origin", "main")
        bin_ = self.tmp / "bin"
        bin_.mkdir()
        (bin_ / "gh").write_text(FAKE_GH)
        (bin_ / "gh").chmod(0o755)
        self.log = self.tmp / "gh.log"
        self.log.touch()
        agent = subprocess.run(["ssh-agent", "-s", "-a", str(self.tmp / "agent")], check=True, capture_output=True, text=True).stdout
        pid = next(line.split("=")[1].split(";")[0] for line in agent.splitlines() if line.startswith("SSH_AGENT_PID"))
        self.addCleanup(lambda: subprocess.run(["kill", pid]))
        env = mock.patch.dict(os.environ, {"PATH": f"{bin_}:{os.environ['PATH']}", "FAKE_GH_LOG": str(self.log),
                                           "SSH_AUTH_SOCK": str(self.tmp / "agent"), "TM_CODESIGN_IDENTITY": "Developer ID Application: Test"})
        env.start()
        self.addCleanup(env.stop)
        os.environ.pop("FAKE_GH_DRAFT", None)
        self.key = self.tmp / "key"
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "test", "-f", str(self.key)], check=True)
        signers = self.tmp / "signers"
        signers.write_text(f'{release.NAMESPACE} namespaces="{release.NAMESPACE}" {self.key.with_suffix(".pub").read_text()}')
        out = self.tmp / "out"
        out.mkdir()
        for p in [mock.patch.object(release.package, "REPO", self.work), mock.patch.object(release.package, "OUT", out),
                  mock.patch.object(release.signing_key, "KEY", self.key), mock.patch.object(release, "SIGNERS", signers)]:
            p.start()
            self.addCleanup(p.stop)
        self.out = out

    def calls(self):
        return self.log.read_text().splitlines()

    def refused(self, fn, *args):
        with self.assertRaises(SystemExit) as cm:
            fn(*args)
        return str(cm.exception.code)

    def test_a_new_version_comes_from_main_and_above_every_tag(self):
        release.check_source("v1.0.0")
        git(self.work, "tag", "-a", "v1.2.0", "-m", "v1.2.0")
        git(self.work, "push", "-q", "origin", "v1.2.0")
        self.assertIn("not above the newest release tag v1.2.0", self.refused(release.check_source, "v1.1.0"))
        (self.work / "f").write_text("2")
        self.assertIn("the tree has changes", self.refused(release.check_source, "v1.3.0"))
        git(self.work, "commit", "-qam", "two")
        self.assertIn("HEAD is not on origin/main", self.refused(release.check_source, "v1.3.0"))
        self.assertIn("v1.2.0 exists and is not HEAD", self.refused(release.check_source, "v1.2.0"))

    def test_a_head_moved_during_the_build_is_not_tagged(self):
        head = release.check_source("v1.0.0")
        self.assertEqual(head, git(self.work, "rev-parse", "HEAD"))
        release.check_unmoved(head)
        (self.work / "f").write_text("2")
        git(self.work, "commit", "-qam", "another session's merge, fast-forwarded")
        self.assertIn("HEAD moved", self.refused(release.check_unmoved, head))

    def test_a_published_release_is_never_changed(self):
        os.environ["FAKE_GH_DRAFT"] = "false"
        self.assertIn("already published", self.refused(release.check_unpublished, "v1.0.0"))
        os.environ["FAKE_GH_DRAFT"] = "true"
        self.assertTrue(release.check_unpublished("v1.0.0"), "a draft of an earlier run is completed")
        del os.environ["FAKE_GH_DRAFT"]
        self.assertFalse(release.check_unpublished("v1.0.0"))

    @unittest.skipUnless(sys.platform == "darwin", "a release is signed on the maintainer's Mac (the key's passphrase in its Keychain)")
    def test_signing_needs_no_prompt_and_leaves_the_key_out_of_the_agent(self):
        sums = self.out / "SHA256SUMS"
        sums.write_text("# release v1.0.0\nabc  x\n")
        release.sign(sums)
        self.assertTrue(release.verify(sums, self.out / "SHA256SUMS.sig"))
        self.assertNotIn("test", subprocess.run(["ssh-add", "-l"], capture_output=True, text=True).stdout)

    @unittest.skipUnless(sys.platform == "darwin", "a release is signed on the maintainer's Mac (the key's passphrase in its Keychain)")
    def test_a_key_whose_passphrase_is_not_in_the_keychain_fails_without_asking(self):
        subprocess.run(["ssh-keygen", "-q", "-p", "-N", "not-in-any-keychain", "-P", "", "-f", str(self.key)], check=True)
        sums = self.out / "SHA256SUMS"
        sums.write_text("x\n")
        self.assertIn("could not load the release key", self.refused(release.sign, sums))
        self.assertFalse((self.out / "SHA256SUMS.sig").exists())

    def test_publishing_tags_then_fills_a_draft_then_publishes(self):
        for f in (release.package.TARBALL, "install.sh", "SHA256SUMS", "SHA256SUMS.sig"):
            (self.out / f).write_text(f)
        url = release.publish("v1.0.0", draft=False, notes="")
        self.assertEqual(git(self.tmp / "origin.git", "rev-parse", "v1.0.0^{commit}"), git(self.work, "rev-parse", "HEAD"))
        verbs = [" ".join(c.split()[:2]) for c in self.calls()]
        self.assertEqual(verbs, ["release create", "release upload", "release edit", "release view"])
        self.assertIn("--draft", self.calls()[0])
        self.assertTrue(url.endswith("/v1.0.0"))
        self.log.write_text("")
        release.publish("v1.0.0", draft=True, notes="")  # an earlier run stopped after tagging
        self.assertNotIn("release create", " ".join(self.calls()))

    def test_notes_preserve_markdown_and_refresh_a_resumed_draft(self):
        notes = "# Changes\n\n- Restore ⌘Q state.\n- Literal `$(ignored)` and quotes: 'x'.\n"
        real_gh, bodies, paths = release.gh, [], []

        def capture(*args, **kwargs):
            if "--notes-file" in args:
                path = Path(args[args.index("--notes-file") + 1])
                bodies.append(path.read_text(encoding="utf-8"))
                paths.append(path)
            return real_gh(*args, **kwargs)

        with mock.patch.object(release, "gh", side_effect=capture):
            release.publish("v1.0.0", draft=False, notes=notes)
            release.publish("v1.0.0", draft=True, notes=notes)
        self.assertEqual(len(bodies), 3, "create, final edit and resumed final edit receive notes")
        self.assertTrue(all(body == bodies[0] for body in bodies))
        self.assertTrue(bodies[0].startswith(notes + "\n"))
        self.assertIn("Install or upgrade:", bodies[0])
        self.assertIn(release.signing_key.fingerprint(), bodies[0])
        self.assertEqual(paths[0], paths[1], "one file is shared by create and final edit")
        self.assertTrue(all(not path.exists() for path in paths), "temporary files are removed")

    def land(self, name, data=None, link=None):
        path = self.work / name
        path.parent.mkdir(parents=True, exist_ok=True)
        if link:
            path.symlink_to(link)
        else:
            path.write_bytes(data)
        git(self.work, "add", name)
        git(self.work, "commit", "-qm", name)
        git(self.work, "push", "-q", "origin", "main")
        return path

    def test_missing_unreadable_or_empty_notes_abort_before_build(self):  # AC-200
        self.assertIn("no release notes: write docs/releases/v1.0.0.md first, landed on origin/main",
                      self.refused(release.release_notes, "v1.0.0"))
        path = self.land("docs/releases/v1.0.0.md", "# Changes\n\nText.\n".encode())
        self.assertEqual(release.release_notes("v1.0.0"), "# Changes\n\nText.\n")
        path.write_text("unlanded words\n")
        self.assertEqual(release.release_notes("v1.0.0"), "# Changes\n\nText.\n", "what landed, not the tree's file")
        git(self.work, "checkout", "-q", "--", ".")
        self.land("docs/releases/v1.0.0.md", b" \n")
        self.assertIn("are empty", self.refused(release.release_notes, "v1.0.0"))
        self.land("docs/releases/v1.0.0.md", b"\xff\xfe")
        with mock.patch.object(release, "check_source", return_value="abc"), mock.patch.object(release, "check_unpublished") as unpublished, \
                mock.patch.object(release.package, "main") as build, mock.patch.object(release, "sign") as sign, \
                mock.patch.object(release, "publish") as publish:
            self.assertIn("cannot read release notes", self.refused(release.main, "v1.0.0", "abc"))
            unpublished.assert_not_called()
            build.assert_not_called()
            sign.assert_not_called()
            publish.assert_not_called()

    def test_notes_landed_after_the_pin_are_read(self):  # AC-200
        pin = git(self.work, "rev-parse", "HEAD")
        self.land("docs/releases/v1.0.0.md", b"# Changes\n")
        git(self.work, "checkout", "-q", "--detach", pin)
        self.assertEqual(release.release_notes("v1.0.0"), "# Changes\n")

    def test_failed_upload_keeps_the_release_unpublished_and_cleans_notes(self):
        real_gh, paths = release.gh, []

        def fail_upload(*args, **kwargs):
            if args[:2] == ("release", "upload"):
                raise SystemExit("upload failed")
            if "--notes-file" in args:
                paths.append(Path(args[args.index("--notes-file") + 1]))
            return real_gh(*args, **kwargs)

        with mock.patch.object(release, "gh", side_effect=fail_upload):
            self.assertIn("upload failed", self.refused(release.publish, "v1.0.0", False, "# Changes"))
        self.assertFalse(any(call.startswith("release edit") for call in self.calls()))
        self.assertTrue(paths and all(not path.exists() for path in paths))

    def test_notes_never_follow_file_or_ancestor_links_outside_reviewed_source(self):  # AC-200
        external = self.tmp / "external"
        (external / "releases").mkdir(parents=True)
        (external / "releases/v1.0.0.md").write_text("# Unreviewed text", encoding="utf-8")
        self.land("docs/releases/v1.0.0.md", link=external / "releases/v1.0.0.md")
        self.assertIn("symlink", self.refused(release.release_notes, "v1.0.0"))
        git(self.work, "rm", "-q", "-r", "docs")
        git(self.work, "commit", "-qm", "no docs")
        self.land("docs", link=external)
        self.assertIn("no release notes", self.refused(release.release_notes, "v1.0.0"), "a linked folder holds no landed notes")

    def test_a_release_needs_the_developer_id(self):
        os.environ["TM_CODESIGN_IDENTITY"] = " "
        self.assertIn("TM_CODESIGN_IDENTITY is not set", self.refused(release.check_settings))

    def test_a_package_without_both_linux_builds_is_not_released(self):
        with mock.patch.object(release, "check_source", return_value="abc"), mock.patch.object(release, "release_notes", return_value="x"), \
                mock.patch.object(release, "check_unpublished", return_value=False), mock.patch.object(release, "run"), \
                mock.patch.object(release.package, "main"), mock.patch.object(release, "sign") as sign, \
                mock.patch.object(release, "publish") as publish:
            self.assertIn("no linux-x86_64 terminumd", self.refused(release.main, "v1.0.0", "abc"))
        sign.assert_not_called()
        publish.assert_not_called()

    def test_release_ui_uses_the_package_version_instead_of_inherited_checkout_id(self):
        for platform in ("linux-x86_64", "linux-aarch64"):
            binary = self.out / release.package.NAME / "agents" / platform / "terminumd"
            binary.parent.mkdir(parents=True)
            binary.touch()
        with mock.patch.dict(os.environ, {"TM_BUILD": "checkout-id"}), \
                mock.patch.object(release, "check_source", return_value="abc"), mock.patch.object(release, "check_unpublished", return_value=False), \
                mock.patch.object(release, "release_notes", return_value="# Changes\n"), mock.patch.object(release, "run") as run, mock.patch.object(release.package, "main") as build, \
                mock.patch.object(release, "check_unmoved") as unmoved, \
                mock.patch.object(release, "sign") as sign, mock.patch.object(release, "publish", return_value="url") as publish:
            order = mock.Mock()
            for name, fn in (("run", run), ("build", build), ("sign", sign), ("unmoved", unmoved), ("publish", publish)):
                order.attach_mock(fn, name)
            release.main("v1.0.0", "abc")
            ui_call = next(call for call in run.call_args_list if call.args == ("npm", "run", "-s", "build"))
            self.assertEqual(ui_call.kwargs["env"]["TM_BUILD"], "v1.0.0")
            self.assertEqual(os.environ["TM_BUILD"], "checkout-id", "the override is local to the UI build")
            build.assert_called_once_with(["--build", "v1.0.0", "--release"])
            self.assertEqual([call[0] for call in order.mock_calls], ["run", "run", "build", "sign", "unmoved", "publish"])

    def test_a_rehearsal_builds_and_signs_tags_pushes_publishes_nothing(self):  # AC-104
        for platform in ("linux-x86_64", "linux-aarch64"):
            binary = self.out / release.package.NAME / "agents" / platform / "terminumd"
            binary.parent.mkdir(parents=True)
            binary.touch()
        never = mock.Mock(side_effect=AssertionError("a rehearsal must not reach this"))
        with mock.patch.object(release, "check_settings"), mock.patch.object(release, "check_source", return_value="abc1234567"), \
                mock.patch.object(release, "check_unmoved"), mock.patch.object(release, "run", return_value="") as run, \
                mock.patch.object(release.package, "main") as build, mock.patch.object(release, "sign", never), \
                mock.patch.object(release, "publish", never), mock.patch.object(release, "gh", never), \
                mock.patch.object(release, "release_notes", never), mock.patch.object(release, "check_unpublished", never):
            release.main("v1.0.0", "abc1234567", rehearsing=True)
        build.assert_called_once_with(["--build", "v1.0.0", "--release"])
        self.assertEqual([c.args[:2] for c in run.call_args_list], [("npm", "ci"), ("npm", "run")])

    def test_what_the_gate_could_not_run_or_passed_only_on_retry_goes_into_the_notes(self):  # AC-104, AC-201
        for platform in ("linux-x86_64", "linux-aarch64"):
            binary = self.out / release.package.NAME / "agents" / platform / "terminumd"
            binary.parent.mkdir(parents=True)
            binary.touch()
        with mock.patch.object(release, "check_settings"), mock.patch.object(release, "check_source", return_value="abc"), \
                mock.patch.object(release, "check_unpublished", return_value=False), mock.patch.object(release, "release_notes", return_value="# Changes\n"), \
                mock.patch.object(release, "run", return_value=""), mock.patch.object(release.package, "main"), mock.patch.object(release, "sign"), \
                mock.patch.object(release, "check_unmoved"), mock.patch.object(release, "publish", return_value="url") as publish:
            release.main("v1.0.0", "abc", "the Mac test runner's checks did not run: x", retried=["page webBoardPanel", "live live"])
        notes = publish.call_args.args[2]
        self.assertIn("**Not verified:** the Mac test runner's checks did not run: x", notes)
        self.assertIn("**Passed only on retry, the first run red:** page webBoardPanel, live live", notes)

    def test_a_commit_the_checks_did_not_see_is_refused(self):  # AC-104
        self.assertIn("HEAD moved since the full checks", self.refused(release.check_gate, "bbbb", "aaaa"))
        self.assertIn("HEAD moved since the full checks", self.refused(release.check_gate, "bbbb", None), "no commit seen is no pass")
        release.check_gate("aaaa", "aaaa")


if __name__ == "__main__":
    unittest.main()
