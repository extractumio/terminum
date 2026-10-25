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

Then a release is one command, with no prompt: `make release TAG=vX.Y.Z` refuses unless the
tree is clean, the version is above every `v*` tag and `HEAD` is `origin/main`; it installs
the UI's dependencies with `npm ci --ignore-scripts`, builds terminumd with `cargo --locked`
for macOS arm64 and x86_64 and Linux x86_64 and aarch64 (`make toolchain` once: zig and
cargo-zigbuild pinned in `.toolchain/`; a release without the Linux builds fails, since a Linux
server installs from the same package), packs `terminum.tar.gz` and the same bytes as
`terminum-macos.tar.gz` (the name installs before 0.22 ask for: keep it until a release notes
a floor version below which `terminum upgrade` is no longer supported), signs `SHA256SUMS` through ssh-agent (the key loaded
for two minutes with the Keychain's passphrase, then removed), checks the signature, and only
then tags `HEAD`, pushes the tag and publishes the GitHub release once all its files are
attached. A failure before signing leaves nothing public.

Each macOS terminumd is signed with the maintainer's Developer ID as `io.terminum.terminumd`
(it embeds `terminumd/macos/Info.plist`), so macOS keeps the folder access the user grants
across upgrades (spec AC-08). The identity is named, never guessed: `TM_CODESIGN_IDENTITY` in
the uncommitted `local.mk` beside the Makefile:

```make
TM_CODESIGN_IDENTITY := Developer ID Application: <name> (<team id>)
```

`make release` refuses without it, signs with Apple's secure timestamp (online) and checks
each binary's Team ID; codesign gets 60 s, so a keychain dialog waiting for "Always Allow"
fails the build instead of hanging it. No hardened runtime and no notarization: the tarball
arrives by curl, which sets no quarantine. Without `local.mk`, `make install` signs ad hoc and
says so; macOS then asks again for protected folders after an upgrade.

Release notes go in `docs/releases/vX.Y.Z.md`, committed before `main` is pushed; the release
command refuses an empty or unreadable file and a symlink out of the tree.

Who else could ship code to users: an upgrade installs only what this key signed, and the key
exists only on the maintainer's Mac (and its backups). Left: whoever controls the
maintainer's GitHub account can replace a release's `install.sh`, which a *first*
`curl | sh` trusts; a program running as the maintainer while the Keychain is unlocked can
sign. Keep the key and its passphrase backed up: a new key makes existing installs refuse
upgrades until they are installed again with the one-line installer. The key is shared with
iterm-enhancer, so a leaked key is replaced for both: a new key in both repositories'
`release-signers` and `install.sh`, a release of each, and every install of each installed
again with its one-line installer.

## Builds for remote hosts

`make agents` builds terminumd for `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` (static Linux builds with the
pinned zig); `make package` puts them into `dist/package` (`terminum/agents/<platform>/terminumd`,
`BUILD`, `AGENT_ID`, `release-signers`, licenses; no scripts) next to `install.sh`, which runs
the package's terminumd for the Mac's architecture with `install`: the macOS build is the
installer, the server and the `terminum` command at once. The helper a host gets is the build
for its platform (`uname -sm`), named by its agent id.

## CI

Two places, both the owner's: GitHub Actions on the Linux runner of the CI VM for every push
(`.github/workflows/ci.yml`), and the macOS checks on the LAN's Mac test runner over ssh
(`make test-mac`, `scripts/ci_mac.sh`; `mac-runner` by default, `TM_MAC_RUNNER` names another), the
way the owner's Mac pool runs tests: no GitHub runner on a Mac, whose user can do more than a
CI job should. `make test-mac` is evidence, not a gate: run it before a release and after a
change to macOS-only code (launchd, signing, folders, Finder).

The Linux job runs `make test`, clippy with warnings as errors, `e2e_install.sh`, the Files
panel's and the page's browser checks (Chromium and WebKit) against a private terminumd and
tmux, and `security_check.sh`. Timing checks scale by the load per core (`web/test/check.mjs`),
since the VM is shared with coding agents. A real `systemctl --user` run is not part of it:
the runner has no user manager, and giving it one (linger) would let a job leave a unit running
outside the runner's confinement; the systemd half is covered by its unit tests (a stub
systemctl) and checked by hand on a disposable machine before a release that changes it.

### The runner (once, on the CI VM)

the owner's network notes' `scripts/runner setup script` sets up each runner of that VM under its own
unprivileged user in `a shared slice` (in those notes). terminum's profile:
user `<runner user>`, runner `<runner name>`, labels `ci-vm,terminum`, its own memory
cap of 1536M, and as extra packages `tmux build-essential` and the browsers' system libraries
(`npx -y playwright-core@<ui's version> install-deps chromium webkit`, as root). The slice holds
2 GB for every runner of the VM together: a terminum job (cargo, two browsers, terminumd and
tmux) can peak near 1.5 GB, so a concurrent job of another project may fail for memory. Measure the
first jobs' peak (`systemctl show <unit> -p MemoryPeak`) and raise the slice or move CI to a VM
of its own if they collide. The registration token travels on stdin, never on a command line:

```bash
gh api -X POST repos/extractumio/terminum/actions/runners/registration-token --jq .token \
  | ssh <runner machine> 'read -r ACTIONS_RUNNER_INPUT_TOKEN; export ACTIONS_RUNNER_INPUT_TOKEN;
      RUNNER_VERSION=… RUNNER_SHA256=… /root/runner setup script terminum'
```

The repository's settings keep everyone else's code off the runner: Actions › General ›
"Require approval for all external contributors" (`gh api -X PUT
repos/extractumio/terminum/actions/permissions/fork-pr-contributor-approval -f
approval_policy=all_external_contributors`), the runner registered to this repository alone,
and the workflow's `permissions: {}`. Never approve a fork's run (CLAUDE.md §10).
