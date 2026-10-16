# Releasing

How releases are built, signed and published. When a release may happen is in
[CLAUDE.md](../CLAUDE.md) §10; how users install is in the [README](../README.md).

## Signed releases

Installs and upgrades accept a release only if its `SHA256SUMS` is signed by the
maintainer's release key (an ed25519 ssh key; `ssh-keygen -Y`, built into macOS). Its public
half is in `release-signers` and in `scripts/install.sh`, and every package carries it, so
`terminum upgrade` checks with the key of the build already installed. Once, in your own
terminal:

```bash
make signing-key        # ~/.config/terminum/release-key (TM_RELEASE_KEY); writes the public half
git add release-signers scripts/install.sh && git commit -m "Release key"
ssh-keygen -p -f ~/.config/terminum/release-key                     # give it a passphrase
ssh-add --apple-use-keychain ~/.config/terminum/release-key \
  && ssh-add -d ~/.config/terminum/release-key.pub                    # keep it in the Keychain, not loaded
```

Then a release is one command, with no prompt: `make release TAG=vX.Y.Z` refuses unless the
tree is clean, the version is above every `v*` tag and `HEAD` is `origin/main`; it installs
the UI's dependencies with `npm ci --ignore-scripts`, builds terminumd with `cargo --locked`
for macOS arm64 and x86_64 and the Linux helpers (`make toolchain` once: zig and
cargo-zigbuild pinned in `.toolchain/`), signs `SHA256SUMS` through ssh-agent (the key loaded
for two minutes with the Keychain's passphrase, then removed), checks the signature, and only
then tags `HEAD`, pushes the tag and publishes the GitHub release once all its files are
attached. A failure before signing leaves nothing public.

Release notes go in `docs/releases/vX.Y.Z.md`, committed before `main` is pushed; the release
command refuses an empty or unreadable file and a symlink out of the tree.

Who else could ship code to users: an upgrade installs only what this key signed, and the key
exists only on the maintainer's Mac (and its backups). Left: whoever controls the
maintainer's GitHub account can replace a release's `install.sh`, which a *first*
`curl | sh` trusts; a program running as the maintainer while the Keychain is unlocked can
sign. Keep the key and its passphrase backed up: a new key makes existing installs refuse
upgrades until they are installed again with the one-line installer.

## Builds for remote hosts

`make agents` builds terminumd for `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` (static Linux builds with the
pinned zig); `make package` puts them into `dist/package` (`terminum/agents/<platform>/terminumd`,
`BUILD`, `AGENT_ID`, `release-signers`, licenses; no scripts) next to `install.sh`, which runs
the package's terminumd for the Mac's architecture with `install`: the macOS build is the
installer, the server and the `terminum` command at once. The helper a host gets is the build
for its platform (`uname -sm`), named by its agent id.
