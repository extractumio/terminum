# terminum

**The self-hosted, tmux-native browser IDE where coding agents take a task from the board to a
checked, published commit, and ask you when a decision is yours.**

terminum automates the work on a project: a task board, a Dispatcher that runs tasks on Claude
Code and Codex, and an integration queue, beside real tmux terminals and a file editor on your
Mac, Linux server or any ssh host. No app, no account, no vendor relay.

[![terminum on a monitor, a laptop and a phone: one Claude Code session on all three](docs/images/terminum-devices.jpg)](https://terminum.app/)

1. **You put tasks on the board.** Typed, dropped, scheduled, or added by an agent.
2. **Agents run and integrate them.** Dispatcher starts an agent on each task; the queue checks and
   publishes its commit. Auto-Dispatch and the queue are off until you turn them on.
3. **You answer what only you can.** An agent that needs you asks and waits; the board says so.

terminum is built this way: every `Integrate TERM-nnn` commit here went through its own queue.

## Features

### From task to published commit

- **Board:** tasks with a type, severity, owner, comments, files and subtasks.
- **Dispatcher:** press Run or drop a task on an agent: it opens a tmux window, starts the agent
  with your command, closes the session when done.
- **Budgets:** a time budget per task, memory limits per project, subscription use on the board.
- **Integration queue** (opt-in): commits merge in batches, your check runs, a green batch is
  pushed, a red one goes to an agent.
- **Auto-Dispatch** (opt-in): starts TODO tasks by priority or an agent's scoring; turns itself off
  after failures.
- **Schedules:** tasks by the clock or a cron line, each with an agent and a time budget.
- **Managers:** a project's own session runs, answers and stops its tasks: one agent leads the
  others.
- **[Project Supervisor](docs/USAGE.md#project-supervisor)** (opt-in) measures project health,
  raises incidents through one critical line and brings decisions to a weekly review.

### Asked for decisions, shown everything

- **Questions:** an agent says why it needs you and recommends answers; reply and press Run.
- **Alerts:** a waiting or failed task shows in every session and on the board.
- **Plan and state:** each agent's percent done, ETA and state in the list.
- **Dashboard:** your share of the work beside the agents', failures and costs.
- **Message history:** every prompt and command sent, searchable, with its sender.

These come with `terminum hooks install` (see [the guide](docs/USAGE.md)).

### A full IDE in the browser

- **Real terminal:** tmux emulates; vim, htop and agents look native.
- **Files:** a tree that follows the terminal's folder, an editor with highlighting.
- **Viewer:** Markdown with Mermaid, JSON and YAML as trees, CSV as tables; PDF, audio and video
  play in place.
- **Quote & Ask:** select text, add a question, send it as a numbered quote.
- **Rendered chat:** Markdown, tables and Mermaid drawn in the output.
- **Bookmarks and reminders:** mark a line, come back later, or make it a task.

### Local or remote: no difference

- **Same agents:** Claude Code and Codex on any ssh host, in one list; tasks run there too.
- **Same tmux:** attach sessions of your own tmux server by name.
- **Same files:** browse and edit on any host, no sftp client.
- **Same clipboard:** pasted screenshots and dropped files land on the session's host.
- **Zero server setup:** the host needs only tmux; reconnects by itself.

### Any screen, nothing lost

- **Phone, iPad, desktop:** the full IDE, touch-ready.
- **Survives reboots:** sessions, windows and splits come back with their last 300 lines.

### Self-hosted, open source

- **Your data on your machine:** tasks, messages, the log of typed input (no passwords) and the
  Dashboard's counts (no key or text), kept 180 days under `~/.terminum`.
- **Open source, AGPL-3.0:** released almost daily, every release signed.

## Install

### On a Mac

```bash
brew install tmux        # tmux 3.8+ (older ones can crash); macOS 14+
curl -fsSL https://github.com/extractumio/terminum/releases/latest/download/install.sh | sh
```

macOS asks once whether terminum's terminals may read Desktop, Documents, Downloads and your
cloud folders: Allow. Then:

```bash
echo 'export PATH="$HOME/.terminum/bin:$PATH"' >> ~/.zshrc   # the install's tip
export PATH="$HOME/.terminum/bin:$PATH"
terminum password        # set the password every browser signs in with
terminum open            # open it: http://127.0.0.1:48080/
terminum hooks install   # for Claude Code and Codex: exact states, plans, the board's tools
```

`terminum upgrade` installs the latest signed release, `terminum rollback` goes back; the page
says when one is out.

### On a Ubuntu/Debian server

```bash
sudo apt install tmux curl openssh-client
curl -fsSL https://github.com/extractumio/terminum/releases/latest/download/install.sh | sh
echo 'export PATH="$HOME/.terminum/bin:$PATH"' >> ~/.bashrc
export PATH="$HOME/.terminum/bin:$PATH"
terminum password
terminum hooks install   # for Claude Code and Codex: exact states, plans, the board's tools
```

terminumd runs as a systemd user service; `sudo loginctl enable-linger $USER` keeps it and the
terminals after you log out. Open the page through `ssh -L 48080:127.0.0.1:48080 <server>` at
`http://127.0.0.1:48080/`, or with `tailscale serve` on the server.

### A remote host's tmux

The host (Linux or macOS) needs tmux and `ssh` access from the machine running terminum. There:

```bash
terminum hosts add devbox 'alex@devbox.example'
```

The host's tmux sessions, files and agents' progress show on the page; terminum installs its
helper and hooks there and keeps them at its own version.

### Over Tailscale

For a phone or another computer, serve it on your tailnet and sign in at
`https://<machine>.<tailnet>.ts.net/`:

```bash
tailscale serve --bg --https=443 http://127.0.0.1:48080
```

This takes the machine's tailnet name; to give terminum a name of its own, see
[Over Tailscale](docs/USAGE.md#over-tailscale).

## Learn more

- [The guide](docs/USAGE.md): every command, feature, remote host and setting.
- [Architecture](docs/ARCHITECTURE.md): every part on one diagram.
- [Set it up with a coding agent](skills/terminum-setup/SKILL.md): a skill that installs, checks and repairs terminum for you.
- [Onboard a project](docs/USAGE.md#onboard-a-project): "onboard my project" in a session configures its agents, routines and Supervisor.
- [SECURITY.md](SECURITY.md): the threat model.
- [All the docs](docs/README.md): specs, contributing, releases.

Licensed under AGPL-3.0 or a commercial license (`LICENSE`, `LICENSE-COMMERCIAL.md`);
contributions under the agreement in `CONTRIBUTING.md`, each commit signed off
(`git commit -s`). The coding agents' marks in
`web/static/agents/` are their owners' trademarks, outside both licenses (`NOTICE`).
