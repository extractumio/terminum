# terminum

**The agentic IDE for vibe coding, built on tmux and made for working on the go.**

Run Claude Code, Codex and your shells in real terminals on your Mac and your servers, and pick
them up from any browser: the laptop on your desk or the iPhone in your pocket. Every session
lives in tmux, so nothing is lost when you close a tab, switch devices or reboot.

## Why terminum

### Real terminals, on every screen

- **A true terminal in the browser.** tmux does the emulation, so vim, htop, mc, Claude Code and
  Codex look exactly as in a native terminal, with Screen mode for full-screen programs.
- **One workspace, every device.** Start on the Mac, carry on from the phone: the same sessions,
  projects and bookmarks, live, with the pane sized to the screen you read it on.
- **Built for the phone.** A touch-first layout, a hot-key row, paste and file upload: real work
  on an iPhone, not just a peek.
- **Splits and sessions as in tmux.** Split panes, windows, a Quick shell under any pane; attach
  the tmux sessions you already run, without disturbing them.
- **Sessions that never die.** They outlive the browser and the server, and after a reboot come
  back with their windows, splits and folders; a session's menu shows its last 300 lines from
  before the restart.

### Made for coding agents

- **Claude Code and Codex hooks.** One command (`terminum hooks install`) and every session shows
  what its agent does: working, waiting for your approval, done, running subagents. It also adds
  terminum's MCP server, through which the agent keeps the session's plan, progress and notes for
  you (in Codex a marked block in `~/.codex/config.toml`; trust terminum's hooks once in Codex's
  `/hooks` and restart running Codex sessions). Outside terminum the server offers no tools.
- **Plans, progress and notes.** Each agent session shows its plan as a percent in the list;
  the Plan pane has the steps, the agent's ETA and the notes it left you (bugs, follow-ups,
  questions), with Done and Insert into chat. Agents on your ssh hosts too, once enabled per host.
- **Alerts across sessions.** See at a glance which agent waits for you, and which finished while
  you were away, without opening each session.
- **Quote & Ask.** Select any output, ask about it, and the quote and your question go straight
  to the agent.
- **Bookmarks and reminders.** Pin a line of output; a reminder comes up when the agent ends
  its turn.
- **Navigate the conversation.** Your prompts, the agent's answers and your bookmarks are marked
  along the history: jump to any of them, or type an earlier prompt again.
- **Screenshots in one paste, even over ssh.** ⌘V an image into the terminal and Claude Code
  receives it, on your Mac or in a session on a remote server: terminum carries the file to the
  host for you, a thing a plain terminal can't do. Dropped files go the same way.

### An IDE around your terminal

- **Files beside the terminal, local and remote.** A file tree that follows the pane's folder,
  a viewer and an editor with highlighting; create, rename, move to Trash. It works the same in
  an ssh session: browse and edit the server's files next to its terminal, with no sftp client
  or remote editor to set up.
- **Rich output.** Markdown, code, Mermaid diagrams, images and links rendered right inside the
  terminal output, with folds for long blocks.
- **Many projects, one list.** Sessions grouped into colored projects, ⌘[ / ⌘] to go back and
  forth between sessions, search through the history, 20 color schemes.

### Your servers, one page

- **Remote hosts over ssh.** Add a host by its ssh command; terminum keeps its tmux connected and
  reconnects after a drop.
- **Zero setup on the server.** Only tmux is needed there: terminum copies and updates its own
  helper for the host's files (Linux and macOS).

### Private by design

- Listens on loopback only, every browser signs in with your password, other devices come over
  Tailscale's HTTPS; terminal output and files never run script in the page; releases are
  signed. Details in [SECURITY.md](SECURITY.md).

## Install

### On a Mac

```bash
brew install tmux        # tmux 3.2+; macOS 14+
curl -fsSL https://github.com/extractumio/terminum/releases/latest/download/install.sh | sh
```

macOS then asks on the screen whether terminum's terminals may read Desktop, Documents,
Downloads and your cloud folders: Allow, once. Then:

```bash
echo 'export PATH="$HOME/.terminum/bin:$PATH"' >> ~/.zshrc   # the install's tip
terminum password        # set the password every browser signs in with
terminum open            # open it: http://127.0.0.1:48080/
terminum hooks install   # optional: exact states from Claude Code and Codex
```

`terminum upgrade` installs the latest signed release (`terminum rollback` goes back); the page
shows a chip when one is out.

### On a Ubuntu/Debian server

```bash
sudo apt install tmux curl openssh-client
curl -fsSL https://github.com/extractumio/terminum/releases/latest/download/install.sh | sh
terminum password
```

terminumd runs as a systemd user service (`sudo loginctl enable-linger $USER` keeps it, and the
terminals, after you log out). Open the page from your computer through
`ssh -L 48080:127.0.0.1:48080 <server>` at `http://127.0.0.1:48080/`, or with `tailscale serve` on
the server.

### A Linux server's tmux

Install tmux there (`sudo apt install tmux`) and make sure the Mac reaches it with `ssh`; then,
on the Mac:

```bash
terminum hosts add devbox 'alex@devbox.example' --files
```

Its tmux sessions and, with `--files`, its files show on the page; terminum copies its helper
there and keeps it at the Mac's version.

### Over Tailscale

To open terminum on your phone or another computer, serve it on your tailnet and sign in at
`https://<mac>.<tailnet>.ts.net/`:

```bash
tailscale serve --bg --https=443 http://127.0.0.1:48080
```

This takes the Mac's tailnet name. To keep that name for another service, give terminum a name of
its own with a Tailscale Service: see [Over Tailscale](docs/USAGE.md#over-tailscale) in the guide.

## Learn more

- [The guide](docs/USAGE.md): every command, feature, remote host and setting.
- [Architecture](docs/ARCHITECTURE.md): every part on one diagram, a Mac, a Linux server and the browsers.
- [Set it up with a coding agent](skills/terminum-setup/SKILL.md): a skill Claude Code, Codex and the like follow to install, check and repair terminum for you.
- [SECURITY.md](SECURITY.md): the threat model.
- [docs/specs/](docs/specs/): behavior, limits and evidence; contributor rules in
  [CLAUDE.md](CLAUDE.md), releases in [docs/RELEASING.md](docs/RELEASING.md).

terminum is the successor of [iterm-enhancer](https://github.com/extractumio/iterm-enhancer),
without iTerm2. Licensed under AGPL-3.0 or a commercial license (`LICENSE`,
`LICENSE-COMMERCIAL.md`); contributions come under the agreement in `CONTRIBUTING.md`. The coding
agents' marks in `web/static/agents/` are their owners' trademarks, outside both licenses
(`NOTICE`).
