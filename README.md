# terminum

**The self-hosted, tmux-native browser IDE for running a project on coding agents: they take a task
from the board to a checked, published commit, and ask you when a decision is yours.**

terminum automates the work on a project so that it moves with as little of you as the work allows:
a task board, a Dispatcher that runs tasks on Claude Code and Codex within the budgets you set, and
an integration queue that publishes only what passes your project's check, beside real tmux
terminals and a file editor for your Mac, your Linux server and any ssh host. No app, no account,
no vendor relay: it runs on your machine, and your devices reach it over your own network.

[![terminum on a monitor, a laptop and a phone: one Claude Code session on all three](docs/images/terminum-devices.jpg)](https://terminum.app/)

1. **You put tasks on the board.** Typed, taken from a terminal's selection or a dropped file, made
   by a schedule, or added by an agent that noticed something and left it undone.
2. **Agents run and integrate them.** Dispatcher starts Claude Code or Codex on a task in a tmux
   window and ends it if it reaches its time budget; the integration queue merges finished commits
   in batches, runs your project's check and publishes only a green batch. Starting tasks by itself
   (Auto-Dispatch) and the queue are switches, off until you turn them on.
3. **You answer what only you can.** An agent that needs you says why, recommends answers and
   waits; a line says so on the session list and the board, at your desk or on your phone.

terminum is built this way: every `Integrate TERM-nnn` commit in this repository's history is one
of its own tasks, checked and published by its own integration queue.

## Features

### From task to published commit

- **Board:** tasks with a type, a severity, an owner, comments, files and subtasks, in TODO,
  IN PROGRESS, WAIT FOR HUMAN, FAILED and DONE.
- **Dispatcher:** press Run, or drop tasks on an agent of the project's pool: it opens a tmux
  window in the project's folder, starts the agent by the command you wrote for it (its
  permissions are yours to set there), moves the card as the session lives and closes the session
  5 minutes after it ends. Run on a busy agent offers its queue: the task starts when a slot frees.
- **Budgets:** no agent runs past its task's time budget; a session over the project's memory
  limits is warned, then ended; each agent's subscription use, its five-hour window and its week,
  shows on the board.
- **Integration queue** (off until you turn it on for a project): commits that merge together form
  a batch, your project's check runs on it, a green batch is pushed, never forced, and a red one
  goes to an agent that returns the task that broke it.
- **Auto-Dispatch** (off until you turn it on for a project): starts TODO tasks by itself, by
  priority or an agent's scoring: P0 and P1, 10 in 24 hours unless you set otherwise; a failed
  round or two failed tasks turn it off, and it says why.
- **Schedules:** a task by the clock: an interval, days of the week, a day of the month or a cron
  line, each with an agent and a time budget; every firing is listed with what went in and what
  came out.
- **Managers:** a project's own session runs, answers and stops its tasks by tools, so one agent
  leads the others.

### Asked for decisions, shown everything

- **Questions in three parts:** an agent that needs you says why, recommends answers and gives the
  details; its task waits in WAIT FOR HUMAN; answer in a comment, press Run, and the same
  conversation goes on.
- **Cross-session alerts:** a task or an agent that waits for you, or failed, shows in every
  session and on the board.
- **Plan and state:** each agent keeps its plan via MCP; the list shows its percent done, its ETA
  and its state: working, waiting, done, subagents.
- **Dashboard:** your share of the operations on tasks beside the agents', how Dispatcher's
  sessions ended, and what they cost in tokens and list-price dollars.
- **Message history:** every prompt and command sent, found by its words, each saying who sent it:
  you, Dispatcher or a manager.

These come with `terminum hooks install` (see [the guide](docs/USAGE.md)).

### A full IDE in the browser

- **Real terminal:** tmux emulates; vim, htop and agents look native.
- **Files:** a tree that follows the terminal's folder, an editor with highlighting, Markdown
  rendered with its Mermaid diagrams.
- **Viewer:** JSON and YAML as a tree, CSV and TSV as tables; pictures, PDF, audio and video play
  in place.
- **Quote & Ask:** select text, comment, sent to the agent as a numbered quote.
- **Rendered chat:** Markdown, tables and Mermaid drawn in the output.
- **Bookmarks and reminders:** mark a line now, come back to it later, or make it a task.

### Local or remote: no difference

- **Same agents:** Claude Code and Codex on your Mac, your Linux server or any ssh host, in one
  session list; Dispatcher runs tasks in sessions on a host the same way.
- **Same tmux:** sessions of your own tmux server, on this machine or a host, attach to the list
  by name.
- **Same files:** browse and edit files on any host, no sftp client needed.
- **Same clipboard:** paste screenshots, drop files (and folders from a Mac's Finder); they land on
  the session's host.
- **Zero server setup:** only tmux needed on the host; reconnects by itself.

### Any screen, nothing lost

- **Phone, iPad, desktop:** the full IDE in the browser, touch-ready.
- **Survives reboots:** sessions, windows and splits come back with their last 300 lines.

### Self-hosted, open source

- **Your data on your machine:** tasks, the message history, the log of what was typed into
  terminals (a password prompt's input is not kept) and the Dashboard's counts of your presses and
  keys (never which key or what text), both kept 180 days, live in terminum's own store under
  `~/.terminum`.
- **Open source, AGPL-3.0:** released almost daily, every release signed.

## Install

### On a Mac

```bash
brew install tmux        # tmux 3.8+ (older ones can crash); macOS 14+
curl -fsSL https://github.com/extractumio/terminum/releases/latest/download/install.sh | sh
```

macOS then asks on the screen whether terminum's terminals may read Desktop, Documents,
Downloads and your cloud folders: Allow, once. Then:

```bash
echo 'export PATH="$HOME/.terminum/bin:$PATH"' >> ~/.zshrc   # the install's tip
export PATH="$HOME/.terminum/bin:$PATH"
terminum password        # set the password every browser signs in with
terminum open            # open it: http://127.0.0.1:48080/
terminum hooks install   # for Claude Code and Codex: exact states, plans, the board's tools
```

`terminum upgrade` installs the latest signed release (`terminum rollback` goes back); the page
shows a chip when one is out.

### On a Ubuntu/Debian server

```bash
sudo apt install tmux curl openssh-client
curl -fsSL https://github.com/extractumio/terminum/releases/latest/download/install.sh | sh
echo 'export PATH="$HOME/.terminum/bin:$PATH"' >> ~/.bashrc
export PATH="$HOME/.terminum/bin:$PATH"
terminum password
terminum hooks install   # for Claude Code and Codex: exact states, plans, the board's tools
```

terminumd runs as a systemd user service (`sudo loginctl enable-linger $USER` keeps it, and the
terminals, after you log out). Open the page from your computer through
`ssh -L 48080:127.0.0.1:48080 <server>` at `http://127.0.0.1:48080/`, or with `tailscale serve` on
the server.

### A remote host's tmux

Install tmux on the host (Linux or macOS) and make sure the machine running terminum reaches it
with `ssh`; then, there:

```bash
terminum hosts add devbox 'alex@devbox.example'
```

Its tmux sessions, its files and its coding agents' progress show on the page; terminum copies its
helper there, puts its hooks into Claude Code and Codex there, and keeps both at terminum's version.

### Over Tailscale

To open terminum on your phone or another computer, serve it on your tailnet and sign in at
`https://<machine>.<tailnet>.ts.net/`:

```bash
tailscale serve --bg --https=443 http://127.0.0.1:48080
```

This takes the machine's tailnet name. To keep that name for another service, give terminum a name of
its own with a Tailscale Service: see [Over Tailscale](docs/USAGE.md#over-tailscale) in the guide.

## Learn more

- [The guide](docs/USAGE.md): every command, feature, remote host and setting.
- [Architecture](docs/ARCHITECTURE.md): every part on one diagram, a Mac, a Linux server and the browsers.
- [Set it up with a coding agent](skills/terminum-setup/SKILL.md): a skill Claude Code, Codex and the like follow to install, check and repair terminum for you.
- [SECURITY.md](SECURITY.md): the threat model.
- [All the docs](docs/README.md): specs, contributing, releases, and which fact lives where.

Licensed under AGPL-3.0 or a commercial license (`LICENSE`,
`LICENSE-COMMERCIAL.md`); contributions come under the agreement in `CONTRIBUTING.md`. The coding
agents' marks in `web/static/agents/` are their owners' trademarks, outside both licenses
(`NOTICE`).
