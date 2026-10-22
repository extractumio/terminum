# Using terminum

The full guide: install, commands, the page, your own tmux, other devices, remote hosts and
configuration. The overview is in the [README](../README.md).

## What it is

- **terminumd**, one Rust server on `127.0.0.1:48080`: the tmux control client of a private
  tmux server (`tmux -L terminum`, started by terminumd with `~/.terminum/tmux.conf`) and of each
  remote host over ssh; it serves the page and the Files panel and never emulates a terminal:
  screens are tmux's own grid (`capture-pane`), only the rows that changed are sent.
- **The page** (`web/static`): the session list (tmux sessions, windows, panes; coding agents'
  states), the terminal in **Wrap**, **Grid** or **Fit**, **Screen mode** for full-screen
  programs (the pane is sized to the view, keys and the mouse reach the program), widgets for
  code, Markdown, diagrams and images in the output, folds, bookmarks, search, hot keys for
  phones, paste and upload of files.
- **The Files panel** (`ui/`): the tree of the shown pane's folder, a viewer and editor with
  highlighting and rendered Markdown (its ```mermaid fences drawn as diagrams, "Source" shows
  their text), create, rename, Trash, live refresh; on a wide window it is docked beside the
  terminal, open or closed for each session, its width set by dragging its border with the
  terminal; on a phone it is a view in its place.
- **Sizing**: the pane follows the page you use: the one that opens it, comes to the front or
  types there sizes it to its view (45×14 to 240×70 cells); a window of several panes (Claude
  Code's teammates split it) keeps its layout. Text is never scaled: a pane narrower or wider than
  your view re-flows to it, lines tmux or the program wrapped joined again (a full-screen program
  such as mc is drawn as its grid, scaled when wider). Switching sessions shows the new one at once
  at your text size.
- **Sessions outlive everything**: terminals live in tmux; terminumd runs as a launchd agent
  and comes back after a crash; after a reboot, or when a tmux server was killed, the sessions,
  windows, splits and folders are restored: terminum's own, each host's and the sessions of your
  own tmux you attached (`"restore": true`).

## Install

```bash
curl -fsSL https://github.com/extractumio/terminum/releases/latest/download/install.sh | sh
```

Needs macOS 14+ and tmux 3.2+ (`brew install tmux`); nothing else. The installer checks the
release's signature, installs into `~/.terminum/` (`builds/<build>`, `current`, the command
`~/.terminum/bin/terminum`, which is terminumd itself), loads the launchd agent
`io.terminum.terminumd` and says how to set the password. launchd holds `127.0.0.1:48080` from
then on, so no other program can take the page's port while terminumd restarts or is upgraded.

From a checkout: `make install` (Rust, Node 26 and the Xcode Command Line Tools needed);
`make test` runs the unit tests.

## Use

```bash
terminum password             # the password every browser signs in with (8+ characters)
terminum open                 # the page in this Mac's browser: http://127.0.0.1:48080/
terminum status               # build, pid, port, password set, tmux servers and sessions, hosts, pages
terminum signout              # every browser signs in again
terminum upgrade              # the latest signed release (--to vX.Y.Z); rollback; uninstall
terminum restart              # load the launchd agent again (a new port in settings.json; a port taken back)
terminum logs                 # terminumd's log (-n N, -f)
terminum hooks install        # Claude Code's and Codex's own hooks: exact states, prompts, reminders
```

Every browser, on the Mac or another device, signs in with the password; until one is set the
page says how. The page asks terminumd to prove it knows the page's sign-in before sending it,
and keeps the sign-in in its own origin's storage, never in its address or a cookie (see
[SECURITY.md](../SECURITY.md)). `tmux -L terminum attach` from any terminal shows the same sessions.

- **Quick shell**: the button in the header (or "Quick shell below" / "on the left" in the menu)
  opens a shell of its own under the shown pane, in its folder, for quick commands; drag its edge
  to size it, ⇄ moves it, × closes it. It is a tmux window marked with its pane, listed under it.
- **Coding agents**: with `terminum hooks install` Claude Code (a plugin of terminum's, added
  with `claude plugin`) and Codex (entries in `~/.codex/hooks.json`; trust them once in Codex's
  `/hooks`) tell terminum when they work, wait for an approval or an answer, stop, fail or run
  subagents; without them terminum reads their screens. `terminum hooks` says what is set up,
  `terminum hooks remove` takes terminum's hooks out.
- **Sent history**: the speech bubble under the split shows what you sent in the pane's folder
  (its prompts and commands, kept in `~/.terminum/sessions`), newest first: ↑ older, ↓ newer;
  Jump to message shows where it was sent, Insert into chat types it again (without Enter). On a
  wide window small bars on the terminal's right edge mark where your messages (white), the
  agent's answers ended (pale blue), bookmarks (orange) and reminders (pink) stand in the
  history: press one to go there. Settings' Side marks turns them off.
- **Ask**: select text in the terminal and press the blue speech bubble with a question mark (under
  the selection's orange [+]); type a question and Add to Chat pastes the
  selection quoted as "Quote #0007", your question and a rule into the input, without Enter.
  The number counts up per session.
- **Reminders**: pick Reminder in a bookmark's dialog; the pale fuchsia bell comes up when an agent
  of its project ends a turn (on a phone it blinks). Close puts it down until the next turn ends,
  Resolve ends it. Insert into chat, on a reminder, a bookmark's bar or a sent message, pastes it
  (a bookmark's note and its text) into the session's input, without Enter.
  Bookmarks and reminders made in a project's session show in every session of that project;
  made outside projects, only in their own session. Once their project is gone (deleted, or its
  last session closed), or a session outside projects closes, they wait under Bookmarks /
  Reminders below the sessions (Open all).
- **The view** is the pane sized to the browser window you use (in front, or the phone in your
  hand), from 45×14 (an iPhone in portrait) to 240×70 cells; a pane of another width re-flows to
  the window (a full-screen program is scaled to fit). The menu has 20 color schemes (one for every browser, kept in settings.json),
  the Markdown text size and the history lines of new terminals. Option or ⌘ with ↑/↓ pages.
- **Keys of an IDE**: ⌘[ and ⌘] go back and forward through the last 20 sessions you showed,
  ⌘F searches the history, ⌘B opens or closes Files. In a coding agent's input, Backspace or
  Delete over text you selected deletes it, as in a text field.
- **The list**: each row's Clone opens a window beside it in its folder; a right click or a long
  press on a row opens its menu (Clone, Add to Project, Close, moves). Projects are named groups
- **The header** names the shown pane under its title: its folder (the copy button copies its full
  path), tmux's `session:window`, then what runs there.
  of sessions in a color of their own, with [+] for a new window in the project's folder, the same
  on every browser. A pale fuchsia blob marks a session whose agent finished while you were away.
  ⌘Z sends the program's undo (Claude Code, zsh, bash, fish); ⌘⇧Z redoes where it can (fish).
- **A screenshot** pasted into the terminal (⌘V) is saved in `~/.terminum/tmp` (on a host, in the
  host's `~/.terminum/tmp`) and its path is pasted: Claude Code takes it as an image.
- **A file dropped** on the page (or picked with Upload) is uploaded as a screenshot is and its
  path pasted, up to 100 MB; give a larger file by its path (a browser never tells a page where a
  dropped file is).

## Your own tmux sessions

```bash
terminum tmux attach work                 # or "Attach tmux sessions" under the list: a session of plain `tmux`
terminum tmux attach work --socket dev    # one of `tmux -L dev`
terminum tmux attach work --host devbox   # one of a host's tmux (the import icon on its group)
terminum tmux                             # what is attached and whether it runs
terminum tmux detach work                 # terminum forgets it; it goes on running
```

terminum shows the session as one of its own, never closes a session there, and sizes its
windows to the page that shows them, as any pane; when no page in sight shows one, it gets
tmux's own size back. If that whole tmux server is gone (a reboot, the server
killed), terminum makes the attached sessions again with their windows and folders; a session
you close while the server runs stays closed. When you
add a host whose tmux runs other sessions, terminum offers them; later the import icon on the
host's group does. Each attached session on a host is one more ssh connection.

## A phone or another computer

terminumd listens only on `127.0.0.1:48080`. Other devices reach it through `tailscale serve`,
which gives it an HTTPS address on your tailnet; they sign in with the same password.

### Over Tailscale

Tailscale gives one name per machine, `https://<mac>.<tailnet>.ts.net`, with one service on
each port. Pick where terminum goes:

| Setup | terminum at | Steps |
| --- | --- | --- |
| Nothing else on the Mac's name | `https://<mac>.<tailnet>.ts.net/` | 1–3 below |
| Another service keeps the Mac's name (such as iterm-enhancer) | `https://terminum.<tailnet>.ts.net/` | 1 below, then the Tailscale Service |

1. Set the password, if you have not yet:

   ```bash
   terminum password
   ```

2. Serve it over HTTPS. This replaces what port 443 served: move another service first (below).

   ```bash
   tailscale serve --bg --https=443 http://127.0.0.1:48080
   ```

3. Open `https://<mac>.<tailnet>.ts.net/` on a device in your tailnet and sign in with the
   password.

Without a tagged machine for a Service, another service on the Mac's name can move to a port
of its own instead, such as iterm-enhancer on 8765:

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
  in `~/.terminum/state/web.json`.
- A device that comes through a proxy cannot use Finder or type a path into a terminal from the
  Files panel; the rest works as on the Mac.
- `tailscale funnel` (the public internet) is refused: terminum answers it with 403.
- To stop serving it: `tailscale serve --https=443 off`.
- `tailscale serve --bg` and `--service` are kept by tailscaled and come back after a restart
  of the Mac. The program behind each address must be running too: terminumd starts at login;
  iterm-enhancer runs only while iTerm2 is open.

#### A name of its own: a Tailscale Service

`https://terminum.<tailnet>.ts.net` is a Tailscale Service. It leaves the Mac's name to another
service, and it can be served only by a tagged machine (`tag:personal` below):

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
4. Open `https://terminum.<tailnet>.ts.net/` and sign in with the password.

5. Give the Mac's name back to the other service if step 2 of the first list took it (it then
   opened terminum under both names), and check:

   ```bash
   tailscale serve --bg --https=443 http://127.0.0.1:8765   # the Mac's name → iterm-enhancer
   tailscale serve status   # <mac>…:443 → 8765; terminum…ts.net (svc:terminum) → 48080
   ```

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
contributor rules in [CLAUDE.md](../CLAUDE.md); the release procedure in
[docs/RELEASING.md](RELEASING.md). Licensing: AGPL-3.0 or commercial (`LICENSE`,
`LICENSE-COMMERCIAL.md`); contributions come under the agreement in `CONTRIBUTING.md`. The
coding agents' marks in `web/static/agents/` are their owners' trademarks, outside both
licenses (`NOTICE`).
