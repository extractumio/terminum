# Releasing

How releases are built, signed and published. When a release may happen is in
[CLAUDE.md](../CLAUDE.md) §10; how users install is in the [README](../README.md).

## Signed releases

Installs and upgrades accept a release only if its `SHA256SUMS` is signed by the
maintainer's release key (an ed25519 ssh key; `ssh-keygen -Y`, built into macOS). Its public
half is in `release-signers` and in `scripts/install.sh`, and every package carries it, so
`terminum upgrade` checks with the key of the build already installed. The key is the
maintainer's one release key, shared with iterm-enhancer: `~/.config/iterm-enhancer/release-key`
(`TM_RELEASE_KEY` names another), its passphrase in the login Keychain; terminum signs with it
in its own namespace, `terminum-release`, so a signature of one product never verifies as the
other's. On a Mac without that key, once, in your own terminal:

```bash
make signing-key        # the key (TM_RELEASE_KEY); writes the public half
git add release-signers scripts/install.sh && git commit -m "Release key"
ssh-add --apple-use-keychain ~/.config/iterm-enhancer/release-key \
  && ssh-add -d ~/.config/iterm-enhancer/release-key.pub              # keep it in the Keychain, not loaded
```

Then a release is one command, with no prompt: `make release TAG=vX.Y.Z`. What it refuses and
does, in order: [terminum-remote.md](specs/terminum-remote.md) AC-86. It needs `make toolchain` once (zig and
cargo-zigbuild pinned in `.toolchain/`), builds every platform below ("Builds for remote hosts"), and packs
`terminum.tar.gz` and the same bytes as `terminum-macos.tar.gz` (the name installs before 0.22 ask
for: keep it until a release notes a floor version below which `terminum upgrade` is no longer supported).

Each macOS terminumd is signed with the maintainer's Developer ID as `io.terminum.terminumd`
(it embeds `terminumd/macos/Info.plist`), so macOS keeps the folder access the user grants
across upgrades (spec AC-08). The identity is named, never guessed: `TM_CODESIGN_IDENTITY` in
the uncommitted `local.mk` beside the Makefile:

```make
TM_CODESIGN_IDENTITY := Developer ID Application: <name> (<team id>)
```

What `make release` and `make install` do with it or without it: spec AC-08 and its §9 "macOS
identity"; codesign's limit means a keychain dialog waiting for "Always Allow" fails the build
instead of hanging it. No hardened runtime and no notarization: the tarball arrives by curl, which
sets no quarantine.

A build's compiler or linker `warning:` is said, by a release and by `make install`, and stops neither.

Release notes go in `docs/releases/vX.Y.Z.md`, landed from a worktree before `make release` (what it refuses there: AC-86).

Who else could ship code to users is in [SECURITY.md](../SECURITY.md) ("The installer and
releases"). Keep the key and its passphrase backed up: a new key makes existing installs refuse
upgrades until they are installed again with the one-line installer. A leaked key is replaced
for both products: a new key in both repositories'
`release-signers` and `install.sh`, a release of each, and every install of each installed
again with its one-line installer.

## Before a release

A release promotes a commit that already passed the full run (spec AC-200, AC-201, AC-202):

- **Every night** the project's schedule "Nightly full run" runs `make rehearse` on `origin/main` (every step) and then
  `make check CHECK_ARGS=--timing` there; a red step is a task. Its passes, a pass per step and content kept in git's
  common folder, are what a release reuses.
- **`make release TAG=vX.Y.Z`** fetches and pins the newest commit of `origin/main` whose content passed in the last 48
  hours, or differs from one only in docs, else `origin/main`; `PIN=<commit>` pins another commit of `origin/main`. It says what it pinned and how many commits
  since the last full pass. In `.worktrees/release`, detached at the pin and kept between runs, the pinned commit's own
  code runs the rest: leftovers cleared (`scripts/leftovers.py --end`, spec AC-102), the full run (only the steps
  without a pass; after a docs-only change the scan alone), the Mac test runner's checks when `TM_MAC_RUNNER` is set
  (a pass kept per content too), the build, the signature, the tag on the pin, the publication. Landings, Afters and
  installs go on meanwhile; nothing of a release is in the primary checkout. Its notes are read as `origin/main`
  holds them, so they may land after the pin.
- **Only failing tests stop it.** The full run waits for a quiet machine, one at a time (`scripts/load_gate.py`, spec
  AC-146); a release finding the nightly's run in the turn waits for its end and reuses its passes. A red step of real
  programs runs its red checks once more (AC-201); one that passes only then is said in red, printed as a task for
  the run's agent, and named in the release notes. A Mac test runner that could not run its checks (asleep, busy for
  half an hour, a dropped link, over an hour) is said at the end and in the notes under Not verified.
- **A red release:** nothing is public. The fix is made in a tree from the pin (`python3 scripts/worktree.py add <slug>
  --from <pin>`), lands through the queue, and `make release TAG=vX.Y.Z PIN=<the fix's commit>` releases the pin and
  the fix alone. A fix with code changes the content, so its full run runs every step again, about 70 minutes; a
  second fix starts from the first fix's commit.
- **The steps' environment** is an allow-list (`ALLOWED` in `scripts/check_all.py`): the maintainer's `TM_TEST_NO_CLAUDE`
  does not reach the full run, whose live checks then need `claude` on `PATH`; no ssh agent reaches a test.
- **The record:** each full run and each release or rehearsal leaves a line in git's common folder,
  `terminum-gate.jsonl`: `python3 scripts/gate_record.py 20` prints the last 20. A release's post-mortem starts there.

`make rehearse TAG=vX.Y.Z` is the same chain in `.worktrees/rehearse`, pinned to `origin/main` (or `PIN=`), every step
run, up to the built package (its macOS builds codesigned with the Developer ID, as for real): no release key, nothing
tagged, pushed or published, no notes needed. Its own target, so a mistyped variable can never publish.

`/simplify` and `/docs-review` are the project's weekly schedule "Weekly review" (spec AC-202), not a release's: a
release ships what `main` has. `make leftovers` says what `scripts/leftovers.py` would remove, `make leftovers END=1`
removes it.

## Builds for remote hosts

`make agents` builds terminumd for the platforms of [terminum-remote.md](specs/terminum-remote.md) AC-86
(Linux with the pinned zig); `make package` puts them into `dist/package` (what it holds:
[terminum-lifecycle.md](specs/terminum-lifecycle.md) §9 Package; no scripts) next to `install.sh`, which runs
the package's terminumd for the Mac's architecture with `install`: the macOS build is the
installer, the server and the `terminum` command at once. The helper a host gets is the build
for its platform (`uname -sm`), named by its agent id.

## CI

Two places, both the owner's: GitHub Actions on the owner's Linux runner for every push
(`.github/workflows/ci.yml`, label `terminum`), and the macOS checks on a Mac test runner of the
owner's network over ssh (`make test-mac`, `scripts/ci_mac.sh`; `TM_MAC_RUNNER` is its ssh name,
in the maintainer's `local.mk`): no GitHub runner on a Mac, whose user can do more than a
CI job should. `make test-mac`, with no real Claude Code (a login and a region it cannot promise), runs from `make release`, and by hand after a change to macOS-only
code (launchd, signing, folders, Finder). `scripts/ci_mac.sh` exits 0 passed, 1 tests failed, 2 they could not run, 3 busy (through make, any failure is make's own 2).

The Mac run requires Node 26. Before rustup, builds or browser checks it selects a supported
Node on the runner's PATH, then tries the installed keg from `brew --prefix node@26`,
and checks `node:sqlite` numbered bindings in memory. To select
another installed runtime, set `TM_MAC_NODE_BIN` to its absolute **runner-side** bin directory
in `local.mk` or the invocation's environment (no colon: PATH uses it as a separator). An explicit directory is authoritative; an
unusable Node fails immediately with its version, path and remedy. The selected bin directory
heads PATH for the whole run, including make, npm and npx. Nothing installs or replaces Node
or changes the runner's global configuration.

The Linux job runs `make test`, clippy with warnings as errors, `e2e_install.sh`, the Files
panel's checks and the page's live checks (`e2e_live.mjs`, Chromium and WebKit) against a
private terminumd and tmux, and `security_check.sh`. The page's own checks (`e2e_all.mjs`)
measure the layout of Safari and Chrome on the Mac, the page's clients: Linux's fonts and
WebKit differ (columns, mouse cells, motion, TIFF), so they run on a Mac (`make test-mac`; on the
maintainer's Mac in a release's `make check` and, for the checks a change touches, in its targeted
pass: spec AC-104). Timing checks scale by the load per core (`web/test/check.mjs`),
since the runner's machine is shared. A real `systemctl --user` run is not part of it:
the runner has no user manager, and giving it one (linger) would let a job leave a unit running
outside the runner's confinement; the systemd half is covered by its unit tests (a stub
systemctl) and checked by hand on a disposable machine before a release that changes it.

Every landing on main gets a verdict: a run on main is never cancelled by the next push, which
waits; of several waiting, GitHub keeps only the newest, so landings closer together than a run
(about 25 minutes) share one. A push to another branch cancels that branch's older run. A check
that fails now and then is run alone, again and again, by hand: `gh workflow run ci -f live="boardpool webkit" -f times=10`
(the words of `web/test/e2e_live.mjs`), on main's code; it waits apart from the checks' runs and
fails when any of its runs fails, each run's log in its own group; it holds the one runner for up to an hour, its
cap, and main's next verdict waits for it. `gh run view --log` cuts a long
job's log short: read the whole one with `gh api repos/extractumio/terminum/actions/jobs/<job>/logs`.

The pinned Playwright dev dependency supplies the test browsers. Linux WebKit needs libsoup
3.6.6 or later: 3.6.5 can crash its network process when a navigation cancels requests
([upstream fix](https://bugs.webkit.org/show_bug.cgi?id=324897)). Install the matching browsers
after updating `ui/package-lock.json`.

### The runner (once)

An Ubuntu 24.04 machine; the runner under an unprivileged user of its own (no sudo, no docker,
its home closed to others), as a systemd service confined (`ProtectHome=tmpfs`, `PrivateTmp`, its
home bound in) and capped in memory (`MemoryMax=3G`, `OOMPolicy=continue`: a job that runs out
fails alone), labels `terminum`. Packages: `tmux build-essential` and the browsers' system
libraries (`npx -y playwright-core@<ui's version> install-deps chromium webkit`, as root); Rust,
Node and the browsers install per job into the runner's home (rustup-init: version and SHA-256
pinned in the workflow's Rust step, a new rustup changes both; only a runner without rustup
downloads it). rustc building the tests is
the job's largest process and grows with terminumd: keep the cap above the peak each run prints
(below it rustc is killed or stalls); each run also prints the job's cap hits. It builds with
one rustc and no debug info (the workflow's env), since two rustc with it outgrew a 2 GB machine. The registration token travels on stdin,
never on a command line:

```bash
gh api -X POST repos/extractumio/terminum/actions/runners/registration-token --jq .token \
  | ssh <runner machine> 'read -r ACTIONS_RUNNER_INPUT_TOKEN; export ACTIONS_RUNNER_INPUT_TOKEN; …'
```

The maintainer's recipe for it lives with the machine's own records, outside this repository.

The repository's settings keep everyone else's code off the runner: Actions › General ›
"Require approval for all external contributors" (`gh api -X PUT
repos/extractumio/terminum/actions/permissions/fork-pr-contributor-approval -f
approval_policy=all_external_contributors`), the runner registered to this repository alone,
and the workflow's `permissions: {}`. Never approve a fork's run (CLAUDE.md §10). Actions ›
General allows only GitHub's own actions, each pinned to a full commit SHA (`gh api -X PUT
repos/extractumio/terminum/actions/permissions -F enabled=true -f allowed_actions=selected -F
sha_pinning_required=true`, then `…/actions/permissions/selected-actions` with
`github_owned_allowed: true`, `verified_allowed: false`, no patterns): a workflow naming another
action, or a tag instead of a SHA, fails before any step runs.

Two rulesets, with no bypass actors, keep `main` and the releases' tags: "main: no force-push,
no deletion" (`refs/heads/main`: `non_fast_forward`, `deletion`; fast-forward pushes from
`make land` and the integration queue pass) and "v* tags: no update, no deletion"
(`refs/tags/v*`: `update`, `deletion`; `make release` only creates a tag, so a release's fix
after its tag was pushed takes a new version). Read them with `gh api
repos/extractumio/terminum/rulesets/<id>` (ids from `gh api repos/extractumio/terminum/rulesets`).
