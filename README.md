# terminum

Terminals that live in a private tmux server, read in a browser on the Mac or on a phone: a
session list, one terminal view with modes for reading, exact grids and full-screen programs,
and an IDE-style Files panel that follows the shown pane's folder. The successor of
[iterm-enhancer](https://github.com/extractumio/iterm-enhancer), feature for feature, without
iTerm2.

## What it is

- **terminumd**, one Rust server on `127.0.0.1:47900`: the tmux control client of a private
  tmux server (`tmux -L terminum`, started by terminumd with `~/.terminum/tmux.conf`) and of each
  remote host over ssh; it serves the page and the Files panel and never emulates a terminal:
  screens are tmux's own grid (`capture-pane`), only the rows that changed are sent.
- **The page** (`web/static`): the session list (tmux sessions, windows, panes; coding agents'
  states), the terminal in **Wrap**, **Grid** or **Fit**, **Screen mode** for full-screen
  programs (the pane is sized to the view, keys and the mouse reach the program), widgets for
  code, Markdown, diagrams and images in the output, folds, bookmarks, search, hot keys for
  phones, paste and upload of files.
- **The Files panel** (`ui/`): the tree of the shown pane's folder, a viewer and editor with
  highlighting and rendered Markdown, create, rename, Trash, live refresh; on a wide window it
  is docked beside the terminal, on a phone it is a view in its place.
- **Sizing**: the pane follows the screen it is read on; by default the page that typed last
  sizes it (`settings.json` `"size"`: `latest`, `largest` or `manual`); "Fit to this screen"
  and "Restore size" in the menu.
- **Sessions outlive everything**: terminals live in tmux; terminumd runs as a launchd agent
  and comes back after a crash; after a reboot the sessions, windows, splits and folders are
  restored (`"restore": true`).

## Install

```bash
curl -fsSL https://github.com/extractumio/terminum/releases/latest/download/install.sh | sh
```

Needs macOS 14+ and tmux 3.2+ (`brew install tmux`); nothing else. The installer checks the
release's signature, installs into `~/.terminum/` (`builds/<build>`, `current`, the command
`~/.terminum/bin/terminum`, which is terminumd itself), loads the launchd agent
`io.terminum.terminumd` and opens the page. launchd holds `127.0.0.1:47900` from then on, so no
other program can take the page's port while terminumd restarts or is upgraded.

From a checkout: `make install` (Rust, Node 26 and the Xcode Command Line Tools needed);
`make test` runs the unit tests.

## Use

```bash
terminum open                 # a one-time link that signs this browser in (valid 60 s)
terminum status               # build, pid, port, tmux servers and sessions, hosts, web access, pages
terminum signout              # every page signs in again
terminum upgrade              # the latest signed release (--to vX.Y.Z); rollback; uninstall
terminum restart              # load the launchd agent again (a new port in settings.json; a port taken back)
terminum logs                 # terminumd's log (-n N, -f)
```

The page asks terminumd to prove it knows the page's sign-in before sending it, and keeps the
sign-in in its own origin's storage, never in its address or a cookie (see
[SECURITY.md](SECURITY.md)). `tmux -L terminum attach` from any terminal shows the same sessions.

## A phone or another computer

```bash
terminum web password         # at least 8 characters
terminum web on               # prints the addresses (http://<mac>:8765/)
terminum web off              # closes the listener and signs every browser out
```

Over a network you do not trust, use HTTPS through `tailscale serve`.

## Remote hosts

```bash
terminum hosts add 'alex@devbox.example -p 2222' --files   # or "Add host" under the list
terminum hosts                                             # each host, its state and files
terminum hosts enable devbox                               # browse its files (copies a helper)
terminum hosts off|on|retry|remove|files-remove devbox
```

A host is its ssh string (options `-p -l -i -J -4 -6 -C`); terminumd holds its tmux session
`terminum` over ssh, reconnects after a drop, and browses its files through a helper it copies
only after you enable it.

## Configure

Everything is in `~/.terminum/` and is read again within a second of a change:

| File | What |
|---|---|
| `settings.json` | `port`, `tmux.socket`, `tmux.history`, `size`, `restore`, `themes` rules (by host and session), `page` defaults (`textSize`, `lines`, `screenMode`, `widgets`, `folds`), `hosts` |
| `themes/*.json` | colors, ANSI palette, font; `local.json` and `remote.json` ship as defaults |
| `hotkeys.json` | the hot keys panel's rows (`"F5"`, `"C-r"`, `"text:git status"`, `"Paste"`) |
| `custom.css` | your stylesheet, loaded after the page's own |
| `tmux.conf` | the private tmux server's configuration (written once; yours to edit) |

A wrong value keeps its default and is said once in the page.

## Specs and contributing

The behavior, limits and evidence are in `docs/specs/` (`terminum.md` and its siblings);
contributor rules in [CLAUDE.md](CLAUDE.md); the release procedure in
[docs/RELEASING.md](docs/RELEASING.md). Licensing: AGPL-3.0 or commercial (`LICENSE`,
`LICENSE-COMMERCIAL.md`); contributions come under the agreement in `CONTRIBUTING.md`.
