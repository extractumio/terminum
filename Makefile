# terminum — build and install from a checkout (users install the package:
# curl … install.sh | sh, then the terminum command; README).
#   make install    build the package from this checkout and install it (first install or
#                   upgrade; a build that does not come up healthy is rolled back)
#   make upgrade    the same
#   make rollback   back to the previous build
#   make uninstall  stop terminumd, remove the launchd agent, builds and command (keeps state, logs)
#   make toolchain  the pinned cross toolchain (zig, cargo-zigbuild) for the Linux builds
#   make agents     terminumd for the four platforms (AC-86)
#   make package    the release package in dist/package (tarball, install.sh, SHA256SUMS)
#   make release TAG=vX.Y.Z   tag HEAD, build, sign (key via ssh-agent and the Keychain) and
#                   publish; only when the maintainer asks for it (CLAUDE.md §10)
#   make signing-key       the maintainer's release key (once)
#   make test       unit tests (Rust, scripts, UI, page)

ifeq ($(shell id -u),0)
$(error Run make as your user, not root: terminum lives in your home folder)
endif

# the maintainer's own settings, never committed: TM_CODESIGN_IDENTITY, the Developer ID
# that signs the macOS terminumd (scripts/package.py; without it, ad hoc)
-include local.mk
export TM_CODESIGN_IDENTITY

# one id per set of sources, compiled into terminumd (option_env!("TM_BUILD")) and the UI
BUILD := $(shell python3 scripts/package.py id)
export TM_BUILD := $(BUILD)
# the day it was built, shown under the session list
export TM_BUILD_DATE := $(shell date +%Y-%m-%d)
# which terminumd sources: an agent on a remote host works with this terminumd when the ids match
export TM_AGENT_ID := $(shell python3 scripts/agents.py id)

.PHONY: all build ui terminumd test install upgrade rollback uninstall package agents toolchain release signing-key

# this Mac's own architecture (sysctl: right under Rosetta too), the package's terminumd for it
PLATFORM := $(shell [ "$$(sysctl -n hw.optional.arm64 2>/dev/null)" = 1 ] && echo macos-aarch64 || echo macos-x86_64)
# the installed command: the newest build's terminumd
TERMINUM := $${TM_HOME:-$$HOME/.terminum}/bin/terminum

all: terminumd
build: terminumd

# exactly the lockfile, no install scripts
ui/node_modules: ui/package.json ui/package-lock.json
	cd ui && npm ci --ignore-scripts --no-fund --no-audit
	touch $@

ui: ui/node_modules
	cd ui && npm run -s build

terminumd: ui
	cd terminumd && cargo build --locked --release

test: ui
	cd terminumd && cargo test --locked
	# the release tooling (developer scripts); the command's own tests are in cargo test
	python3 -m unittest discover -s scripts/tests
	cd ui && npx tsc -p . && npm test
	node --test web/test/*.test.mjs

install: ui
	python3 scripts/package.py --stage
	dist/package/terminum/agents/$(PLATFORM)/terminumd install

upgrade: install

rollback:
	$(TERMINUM) rollback

uninstall:
	$(TERMINUM) uninstall

package: ui
	python3 scripts/package.py

# Remote hosts (AC-86): the pinned cross toolchain once, then terminumd for macOS
# arm64/x86_64 and Linux x86_64/arm64
toolchain:
	python3 scripts/agents.py toolchain

agents: ui
	python3 scripts/agents.py build

release:
	@test -n "$(TAG)" || { echo "usage: make release TAG=v0.3.0"; exit 2; }
	python3 scripts/release.py "$(TAG)"

# the maintainer's release key, once (run it in your own terminal: it asks for a passphrase)
signing-key:
	python3 scripts/signing_key.py
