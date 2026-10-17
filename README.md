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

- **Quick shell**: the button in the header (or "Quick shell below" / "on the left" in the menu)
  opens a shell of its own under the shown pane, in its folder, for quick commands; drag its edge
  to size it, ⇄ moves it, × closes it. It is a tmux window marked with its pane, listed under it.
- **A screenshot** pasted into the terminal (⌘V) is saved in `~/.terminum/tmp` (on a host, in the
  host's `~/.terminum/tmp`) and its path is pasted: Claude Code takes it as an image.

## Your own tmux sessions

```bash
terminum tmux attach work                 # or "Attach tmux sessions" under the list: a session of plain `tmux`
terminum tmux attach work --socket dev    # one of `tmux -L dev`
terminum tmux attach work --host devbox   # one of a host's tmux (the import icon on its group)
terminum tmux                             # what is attached and whether it runs
terminum tmux detach work                 # terminum forgets it; it goes on running
```

terminum shows the session as one of its own, but never starts that tmux server, never makes
or closes a session there, and resizes its windows only once you type in the page. When you
add a host whose tmux runs other sessions, terminum offers them; later the import icon on the
host's group does. Each attached session on a host is one more ssh connection.

## A phone or another computer

```bash
terminum web password         # at least 8 characters
terminum web on               # prints the addresses (http://<mac>:48080/)
terminum web off              # closes the listener and signs every browser out
```

Over a network you do not trust, use HTTPS through `tailscale serve`.

### Over Tailscale

Use this to open terminum at `https://<mac>.<tailnet>.ts.net` from your tailnet only.

1. Let terminum listen only on this Mac, on a free port. Put this in
   `~/.terminum/state/web.json` (keep the other keys when the file exists), then
   `terminum web off` if web access is on:

   ```json
   { "port": 48080, "host": "127.0.0.1" }
   ```

2. Set the password and switch web access on:

   ```bash
   terminum web on      # asks for a password the first time
   ```

3. Serve it over HTTPS. This replaces what port 443 served: move another service first (below).

   ```bash
   tailscale serve --bg --https=443 http://127.0.0.1:48080
   ```

4. Open `https://<mac>.<tailnet>.ts.net/` on a device in your tailnet and sign in with the
   password.

Tailscale gives one name per machine. Another service on it, such as iterm-enhancer on 8765,
gets a port of its own:

```bash
tailscale serve --https=443 off                            # if it had the address without a port
tailscale serve --bg --https=8443 http://127.0.0.1:8765    # it moves to https://<mac>.<tailnet>.ts.net:8443
tailscale serve status                                     # :443 → 48080 (terminum), :8443 → 8765
```

Notes:

- Give `--https` every time: `tailscale serve --bg 48080` also serves port 443, and replaces
  what is served there without a word.
- `http://<mac>.<tailnet>.ts.net` does not open: `tailscale serve` cannot redirect port 80 to
  HTTPS. Type the `https://` address.
- terminum answers to any `*.ts.net` name. Another name a proxy forwards goes in `allow_hosts`
  in `web.json`.
- To also reach terminum over plain HTTP on your local network, set `"host": "0.0.0.0"`.
- To stop serving it: `tailscale serve --https=443 off`.

A name of its own, such as `https://terminum.<tailnet>.ts.net`, is a Tailscale Service. It
can be served only by a tagged machine (`tag:personal` below):

1. In the tailnet policy (admin console → Access controls), let your devices reach the
   service and let the Mac host it without a manual approval:

   ```json
   "grants": [{ "src": ["tag:personal"], "dst": ["svc:terminum"], "ip": ["tcp:443"] }],
   "autoApprovers": { "services": { "svc:terminum": ["tag:personal"] } }
   ```

2. In admin console → Services, define `terminum` with the endpoint `tcp:443`.
3. Then, on the Mac:

   ```bash
   tailscale serve --service=svc:terminum --https=443 http://127.0.0.1:48080
   ```

   `autoApprovers` approves only a host that advertises after the service is defined. Approve
   the Mac on the service's page when it advertised earlier, or without `autoApprovers`.
4. Open `https://terminum.<tailnet>.ts.net/`. It is an address of its own, with its own
   sign-in.

To remove it: `tailscale serve clear svc:terminum`.

## Remote hosts

```bash
terminum hosts add 'alex@devbox.example -p 2222' --files   # or "Add host" under the list
terminum hosts                                             # each host, its state and files
terminum hosts enable devbox                               # browse its files (copies a helper)
terminum hosts off|on|retry|remove|files-remove devbox
```

A host is its ssh string (options `-p -l -i -J -4 -6 -C`); terminumd holds its tmux session
`terminum` over ssh (and the host's other sessions you attach, above), reconnects after a drop, and browses its files through a helper it copies
only after you enable it.

## Configure

Everything is in `~/.terminum/` and is read again within a second of a change:

| File | What |
|---|---|
| `settings.json` | `port`, `tmux.socket`, `tmux.history`, `size`, `restore`, `themes` rules (by host and session), `page` defaults (`textSize`, `lines`, `screenMode`, `widgets`, `folds`), `hosts` (each with `attach`: sessions of its tmux), `attach` (sessions of your own tmux by socket) |
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
