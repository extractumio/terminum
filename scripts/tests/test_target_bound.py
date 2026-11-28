# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""scripts/target_bound.py (spec AC-206): a kept tree's build cache under its bound; on a stand-in target in a
temporary folder, and one real executable's debug map where the platform's compiler makes one."""
import fcntl
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import cargo_profile  # noqa: E402
import target_bound  # noqa: E402
from testrun import backup_exclusion as excluded, short_tmp  # noqa: E402

UNIT = "terminumd-0123456789abcdef"
KIB = 1024
NO_REFILL = lambda tree, names: []   # noqa: E731


class Target:
    """A stand-in tree with a debug folder: an executable, the objects it names, older ones, what stays."""
    def setUp(self):
        self.tree = Path(tempfile.mkdtemp(prefix="tm-bound-", dir=short_tmp()))
        self.addCleanup(shutil.rmtree, self.tree, ignore_errors=True)
        self.target = self.tree / "terminumd" / "target"
        self.deps = self.folder("debug") / "deps"
        self.write(self.deps / UNIT, 64, executable=True)
        self.current = [self.write(self.deps / f"{UNIT}.cgu{n}.new.rcgu.o", 64) for n in range(2)]
        self.old = [self.write(self.deps / f"{UNIT}.cgu{n}.{s}.rcgu.o", 256) for n in range(2) for s in ("s1", "s2")]
        self.kept = [self.write(self.deps / "libtokio-89abcdef01234567.rlib", 64),
                     self.write(self.folder("debug") / "incremental" / "terminumd-x" / "s-1" / "query-cache.bin", 64),
                     self.write(self.folder("debug") / ".fingerprint" / UNIT / "bin-terminumd", 4)]
        (self.target / "CACHEDIR.TAG").write_text(cargo_profile.CACHE_TAG)   # as cargo leaves it
        self.said, self.cleaned = [], []

    def folder(self, profile):
        folder = self.target / profile
        folder.mkdir(parents=True, exist_ok=True)
        (folder / ".cargo-lock").touch()
        return folder

    def write(self, path, kib, executable=False):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(os.urandom(kib * KIB))
        if executable:
            path.chmod(0o755)
        return path


class Bound(Target, unittest.TestCase):
    def hold(self, name=".cargo-lock"):
        """A build folder's lock, held as cargo holds it while it builds, until the test ends."""
        lock = self.deps.parent / name
        lock.touch()
        fd = os.open(lock, os.O_RDWR)
        self.addCleanup(os.close, fd)
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return fd

    def bound(self, limit, refill=NO_REFILL):
        names = {p.name for p in self.current}
        target_bound.bound(self.tree, limit, self.said.append, self.clean, lambda exe: names, refill)

    def clean(self, folder):
        self.cleaned.append(folder.relative_to(self.target).as_posix())
        target_bound.empty(folder)

    def size(self):
        return target_bound.disk_use(self.target)

    def over_once_pruned(self):
        """A bound the target is 64 KiB over once its old objects went."""
        return self.size() - sum(p.stat().st_size for p in self.old) - 64 * KIB

    def test_under_the_bound_nothing_is_removed(self):  # AC-206
        before = self.size()
        self.bound(before)
        self.assertTrue(all(p.exists() for p in self.old + self.current + self.kept))
        self.assertEqual(self.cleaned, [])
        self.assertRegex(self.said[-1], r"^target: \d+\.\d GiB, under its bound of \d+\.\d GiB: nothing removed$")

    def test_over_it_the_objects_no_build_uses_go_and_the_current_build_stays(self):  # AC-206
        self.bound(self.size() - 512 * KIB)
        self.assertFalse(any(p.exists() for p in self.old))
        self.assertTrue(all(p.exists() for p in self.current + self.kept + [self.deps / UNIT]),
                        "the executable, the objects it names, libraries, fingerprints, the incremental cache stay")
        self.assertEqual(self.cleaned, [], "under the bound once they went: nothing emptied")
        self.assertIn("of objects no build uses removed", self.said[-1])
        self.assertNotIn("still over", self.said[-1])

    def test_still_over_after_that_debug_is_emptied_and_release_and_the_cross_targets_kept(self):  # AC-206
        self.write(self.folder("release") / "deps" / "librelease.rlib", 64)
        self.write(self.folder("x86_64-unknown-linux-musl/release") / "deps" / "libcross.rlib", 64)
        self.bound(self.over_once_pruned())
        self.assertFalse(any(p.exists() for p in self.old))
        self.assertEqual(self.cleaned, ["debug"], "the folder that grows, and nothing once under")
        self.assertRegex(self.said[-1], r"; still over: debug emptied, \d+\.\d GiB left$")

    def test_still_over_after_debug_the_largest_of_the_rest_next(self):  # AC-206
        self.write(self.folder("aarch64-apple-darwin/debug") / "deps" / "libd.rlib", 4)
        small = self.write(self.folder("release") / "deps" / "libsmall.rlib", 4)
        self.write(self.folder("x86_64-apple-darwin/release") / "deps" / "liblarge.rlib", 128)
        self.bound(64 * KIB)   # under once the large one went
        self.assertEqual(self.cleaned, ["debug", "aarch64-apple-darwin/debug", "x86_64-apple-darwin/release"],
                         "every debug folder, however small, before the largest of the rest")
        self.assertTrue(small.exists())

    def test_release_is_kept_where_emptying_it_would_not_get_the_target_under(self):  # AC-206
        """Over in a folder that is no build folder (target/doc): release and the cross builds would go for nothing."""
        self.write(self.target / "doc" / "index.html", 512)
        release = self.write(self.folder("release") / "deps" / "librelease.rlib", 64)
        self.bound(256 * KIB)
        self.assertEqual(self.cleaned, ["debug"])
        self.assertTrue(release.exists())
        self.assertIn("; the rest kept: emptying it would not get it under", self.said[-1])

    def test_the_clones_a_worktree_gets_back_do_not_empty_release(self):  # AC-206
        """Its clones count whole (du), though they share the primary's blocks: under once emptied is under."""
        release = self.write(self.folder("release") / "deps" / "librelease.rlib", 64)

        def refill(tree, names):
            self.write(self.target / "debug" / "deps" / "libtokio.rlib", 1024)
            return [f"{name}: its dependencies' builds cloned from the primary checkout" for name in names]
        self.bound(self.over_once_pruned(), refill)
        self.assertEqual(self.cleaned, ["debug"])
        self.assertTrue(release.exists())
        self.assertRegex(self.said[-1], r"still over: debug emptied, \d+\.\d GiB left; debug: its dependencies' builds cloned from the primary checkout$")

    def test_it_empties_a_folder_under_the_locks_it_pruned_it_under(self):  # AC-206
        """No cargo starts in the folder between its pruning and its emptying or refill."""
        free = []

        def clean(folder):
            with cargo_profile.try_lock(folder) as held:
                free.append(held)

        def refill(tree, names):
            with cargo_profile.try_lock(self.target / names[0]) as held:
                free.append(held)
            return []
        target_bound.bound(self.tree, 1, self.said.append, clean, lambda exe: set(), refill)
        self.assertEqual(free, [False, False])

    def test_a_build_running_in_the_tree_is_never_cleaned_under_it(self):  # AC-206
        release = self.folder("release")
        stale_release = self.write(release / "deps" / f"{UNIT}.cgu0.s1.rcgu.o", 256)
        self.write(release / "deps" / UNIT, 4, executable=True)
        for name in cargo_profile.LOCKS:   # any of cargo's locks
            fd = self.hold(name)
            self.bound(1)
            fcntl.flock(fd, fcntl.LOCK_UN)
            self.assertTrue(all(p.exists() for p in self.old + self.current), f"{name}: the folder a build runs in is left as it is")
            self.assertFalse(stale_release.exists(), "a folder no build holds is pruned")
            self.assertEqual(self.cleaned, [], "nothing emptied while a build runs")
            self.assertIn("a build runs in debug: left as it is", self.said[-1])
            self.assertIn("still over: nothing emptied while a build runs", self.said[-1])

    def test_it_holds_the_build_locks_while_it_prunes(self):  # AC-206
        """As cargo's own build does, so a build that starts meanwhile waits for it."""
        free = []

        def names(exe):
            with cargo_profile.try_lock(self.deps.parent) as held:
                free.append(held)
            return {p.name for p in self.current}
        target_bound.bound(self.tree, 1, self.said.append, self.clean, names, NO_REFILL)
        self.assertEqual(free, [False])

    def test_objects_kept_when_the_map_is_unknown_empty_or_the_unit_has_no_executable(self):  # AC-206
        orphan = self.write(self.deps / "launchd-fedcba9876543210.cgu0.s1.rcgu.o", 4)
        for unknown in (None, set()):
            target_bound.bound(self.tree, 1, self.said.append, lambda folder: None, lambda exe: unknown, NO_REFILL)
            self.assertTrue(all(p.exists() for p in self.old + self.current + [orphan]))

    def test_no_target_yet(self):  # AC-206
        shutil.rmtree(self.target)
        self.bound(1)
        self.assertEqual(self.said, ["target: no build yet"])

    def test_the_default_bound(self):  # AC-206
        with mock.patch.object(target_bound, "disk_use", return_value=20 * target_bound.GIB):
            target_bound.bound(self.tree, say=self.said.append, clean=self.clean)
        self.assertEqual(self.said, ["target: 20.0 GiB, under its bound of 20.0 GiB: nothing removed"])

    def test_a_failure_is_said_and_stops_nothing(self):  # AC-206
        with mock.patch.object(target_bound, "bound", side_effect=PermissionError("denied")):
            target_bound.keep(self.tree, self.said.append)
        self.assertEqual(self.said, ["target: its bound was not kept: denied"])

    def test_a_target_cargo_did_not_make_is_marked_as_cargo_marks_its_own(self):  # AC-206
        (self.target / "CACHEDIR.TAG").unlink()
        self.bound(1 << 40)
        self.assertEqual((self.target / "CACHEDIR.TAG").read_text(), cargo_profile.CACHE_TAG)
        if sys.platform == "darwin":
            self.assertEqual(excluded(self.target), cargo_profile.BACKUP_EXCLUDED, "out of Time Machine's backups")
        with mock.patch.object(cargo_profile, "mark_cache", return_value="denied") as mark:
            self.bound(1 << 40)
            mark.assert_not_called()   # tagged: cargo's, or marked already
            (self.target / "CACHEDIR.TAG").unlink()
            self.bound(1 << 40)
        self.assertEqual(self.said[-2:], ["target: not excluded from Time Machine: denied", self.said[-1]])
        self.assertIn("nothing removed", self.said[-1], "a mark it cannot write stops nothing")

    @unittest.skipUnless(sys.platform == "darwin", "Time Machine")
    def test_an_exclusion_it_cannot_write_leaves_no_tag_so_the_next_bound_tries_again(self):  # AC-206
        (self.target / "CACHEDIR.TAG").unlink()
        refused = subprocess.CompletedProcess([], 1, "", "xattr: [Errno 1] Operation not permitted\n")
        with mock.patch.object(cargo_profile.subprocess, "run", return_value=refused):
            self.assertEqual(cargo_profile.mark_cache(self.target), "xattr: [Errno 1] Operation not permitted")
        self.assertFalse((self.target / "CACHEDIR.TAG").exists())

    def test_make_build_keeps_the_bound_before_cargo_builds(self):  # AC-206
        make = (Path(__file__).resolve().parents[2] / "Makefile").read_text()
        self.assertLess(make.index("\tpython3 scripts/target_bound.py\n"), make.index("\tcd terminumd && cargo build --locked --release"))

    def test_disk_use_counts_a_linked_file_once(self):  # AC-206
        before = self.size()
        os.link(self.old[0], self.deps / "a-link")
        self.assertEqual(self.size(), before)


class Refill(Target, unittest.TestCase):
    """empty() and refill(): what stays in an emptied folder, and what a worktree gets back from the primary."""
    def setUp(self):
        super().setUp()
        self.primary = Path(tempfile.mkdtemp(prefix="tm-bound-", dir=short_tmp()))
        self.addCleanup(shutil.rmtree, self.primary, ignore_errors=True)
        self.source = self.primary / "terminumd" / "target" / "debug"
        self.dep = self.write(self.source / "deps" / "libtokio-0011223344556677.rlib", 4)
        self.write(self.source / "deps" / f"{UNIT}.rlib", 4)
        self.write(self.source / ".fingerprint" / "tokio-0011223344556677" / "lib-tokio", 1)
        (self.source / ".cargo-lock").touch()

    def refill(self, primary):
        target_bound.empty(self.deps.parent)
        with mock.patch.object(target_bound.worktree, "primary_of", return_value=primary.resolve()):
            return target_bound.refill(self.tree, ["debug"])

    def test_the_locks_stay_and_the_rest_goes(self):  # AC-206
        for name in cargo_profile.LOCKS:
            (self.deps.parent / name).touch()
        self.assertEqual(self.refill(self.tree), [], "the primary checkout: nothing to clone from")
        self.assertEqual(sorted(p.name for p in self.deps.parent.iterdir()), sorted(cargo_profile.LOCKS))

    @mock.patch.object(target_bound.worktree, "CLONE", ["cp", "-R"])
    def test_a_worktree_gets_the_primarys_dependencies_builds_back(self):  # AC-206
        # Exercise cache filtering even on filesystems without reflinks.
        self.assertEqual(self.refill(self.primary), ["debug: its dependencies' builds cloned from the primary checkout"])
        self.assertTrue((self.deps / self.dep.name).exists())
        self.assertFalse((self.deps / f"{UNIT}.rlib").exists(), "terminumd's own build: the tree rebuilds it")
        self.assertTrue((self.deps.parent / ".fingerprint" / "tokio-0011223344556677" / "lib-tokio").exists())

    def test_the_primarys_busy_profile_is_not_cloned(self):  # AC-206
        with cargo_profile.try_lock(self.source) as held:
            self.assertTrue(held)
            self.assertEqual(self.refill(self.primary), ["debug: the primary's is busy, cargo builds it whole"])
        self.assertFalse(self.deps.exists())

    def test_no_clone_on_this_disk(self):  # AC-206
        with mock.patch.object(target_bound.worktree, "clone_builds", return_value=False):
            self.assertEqual(self.refill(self.primary), ["debug: no clone on this disk, cargo builds it whole"])


@unittest.skipUnless(sys.platform == "darwin" and shutil.which("cc") and shutil.which("nm"), "macOS's debug map")
class DebugMap(unittest.TestCase):
    def test_the_objects_an_executable_names(self):  # AC-206
        folder = Path(tempfile.mkdtemp(prefix="tm-bound-", dir=short_tmp()))
        self.addCleanup(shutil.rmtree, folder, ignore_errors=True)
        (folder / "a.c").write_text("int main(void) { return 0; }\n")
        subprocess.run(["cc", "-g", "-c", "a.c", "-o", "unit.cgu0.s2.rcgu.o"], cwd=folder, check=True)
        subprocess.run(["cc", "unit.cgu0.s2.rcgu.o", "-o", "unit"], cwd=folder, check=True)
        self.assertEqual(target_bound.debug_map(folder / "unit"), {"unit.cgu0.s2.rcgu.o"})
        self.assertIsNone(target_bound.debug_map(folder / "a.c"), "nm cannot read it: unknown")


if __name__ == "__main__":
    unittest.main()
