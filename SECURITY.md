# Security

terminumd gives a browser page your terminals (a shell as you) and your files, so it is built to be
reachable only by its own page. This is the threat model: who can reach it, what stops them, and
what is **left**. Each defense and risk in full is in the spec a line names (`docs/specs/`): its
cases and its §9 Security.

Three things hold everywhere below:

- **The password is the boundary.** Whoever signs in, from any device, has your shell, your files,
  your hosts and every attached session.
- **A process running as you is not stopped.** It can read the store, drive tmux or set the
  password. Roles, scopes and guards on agents' tools stop an agent that errs, never a program of
  yours.
- **What an agent reads is data, never instructions**, and every answer of terminum's tools says
  so. An agent started without permission prompts is still one step from running what it reads.

## Reaching terminumd

`terminum.md` AC-09, AC-10 and §9 Security.

| Threat | Defense |
|---|---|
| Other machines | terminumd listens on `127.0.0.1` only. Other devices come through `tailscale serve` (HTTPS on your tailnet); `tailscale funnel` is refused. |
| Other devices | A forwarded request cannot use Finder, type a path into a terminal or choose about updates: a guard against accidents, no boundary. |
| Web pages (CSRF, DNS rebinding) | Every request that reads or changes terminals or files needs the sign-in, sent as a header, never a cookie or an address. The `Host` must be a name terminum serves; writes need a JSON `Content-Type` and the page's exact `Origin`; no CORS headers. |
| Getting a sign-in | Only the `terminum` command sets the password, over its Unix socket; terminumd keeps a salted, slow hash and brakes wrong ones. `terminum signout` or a new password ends every sign-in. |
| Other users of this Mac | `~/.terminum` is private (0700, 0600); the CLI uses the Unix socket only. |

- **Guessing the password. Left**: any program on this Mac can try passwords on loopback, where
  only the brake on all attempts holds; a sign-in never proves its user sits at the Mac.
- **A program on terminumd's port.** launchd holds the port while terminumd is down
  (`terminum-lifecycle.md` AC-08 and §9 Security). **Left**: before the agent is loaded, after an
  uninstall, when the port was taken at load (`terminum status` says so), and on Linux or with
  `TM_NO_LAUNCHD`, another program can serve a page there that reads the stored sign-in and asks
  for the password, through `tailscale serve` too: **do not open the page then**.

## What the page shows

- **Terminal output and files are text to the page**: CSP `script-src 'self'`, no eval; Markdown
  with raw HTML off, Mermaid in strict mode; links never navigate the page; SVG only as an image;
  HTML files in a sandboxed frame without scripts (`terminum-terminal.md` and `terminum-files.md`,
  §9 Security).
- **A program's clipboard writes (OSC 52)** reach a page only just after your mouse release in
  that full-screen pane, and only as Copy, [+] and Ask: nothing is copied or typed until you press
  one (`terminum.md` AC-06 and §9 Security). **Left**: a program that writes the clipboard over
  and over can have its own text offered after your release, in place of what you dragged over,
  and Copy shows only its line count and first line: **check what you paste** (Ask and [+] show
  it whole).

## Terminals and files

| Threat | Defense |
|---|---|
| Typing into tmux | User text reaches tmux only as one quoted argument, so a folder named `#(cmd)` runs nothing; a paste holding control characters other than tab and line ends is refused (`terminum.md` §9). |
| Typing into the wrong terminal | The Files panel's "insert path" and "cd" name their pane; `cd` goes only into an idle shell; nothing sends Enter (`terminum-files.md` AC-60). |
| Writes outside your files | The Files API writes only under `TM_WRITABLE_ROOTS`, judged on resolved paths; delete moves to the Trash (`terminum-files.md` §9 Security). |

- **Your own tmux server**: terminumd attaches only the sessions you named and never starts that
  server, makes or closes a session there (`terminum.md` AC-89). **Left**: your `~/.tmux.conf`
  hooks and plugins see its clients; whoever signs in can type into every attached session.
- **Remote hosts**: a host is the ssh string you gave, checked against an allowlist: no
  ProxyCommand, no command but terminumd's own (`terminum-remote.md` AC-81 and §9 Security).
  **Adding a host is the consent to set it up**: the files agent, terminum's hooks and its MCP
  server are put there with no further question. A page signed in from another device can add a host
  too, so **the password also decides which machines terminum writes to**. **Left**: a broken host
  can read its project's tasks and write comments the project's other agents read, as data.

## Coding agents

`terminum-agents.md`, `terminum-kanban.md` and `terminum-dispatcher.md`: §9 Security of each.

- **Coding agents' hooks** send a few fields of each event, never a tool's output, a file's text or
  the agent's answer, only to terminumd's socket, or on a host to a Mac's link. **Left**: any
  process of yours can post a made-up event: it changes what a pane or a card shows and can move the
  task of a session Dispatcher started; none of its words runs or reaches tmux.
- **Codex's trust of terminum's hooks** is by the entry's hash, not the program's: **an upgrade of
  terminum changes what the hook does without a new trust prompt.** Its MCP tools are approved
  without a prompt.
- **Plans and tasks of coding agents** are model-written text: cleaned of control and hidden
  characters, redacted, shown as plain text or as Markdown with raw HTML off, never run.
  `terminum mcp` listens on no port; a worker's tools write only the plan and the store, never
  tmux. **Left**: an agent does all on its own tasks, and reads and comments on its project's
  others: a scope, no boundary, since a process of yours can name another pane.
- **The tasks' store** (0600, on the Mac alone) is reached by pages only with the sign-in. Agents'
  text is redacted, yours is not: you can purge a text or a file for good. Its input log holds what
  was typed into panes, but no password at a recognized prompt and no line begun with a space
  (`terminum.md` AC-147). **Left**: a password typed at a prompt it does not recognize is kept.
- **The pools of agents**: an agent's command is one line you wrote, run in your login shell, kept
  and shown as written: **keep a secret in your shell's environment, never in the command.**
  **Left**: a process of yours can change the store, so RUN's dialog shows the command in full
  before it runs.
- **A run's agent's permissions are its command's.** A bypassing command
  (`claude --dangerously-skip-permissions`, `codex --dangerously-bypass-approvals-and-sandbox`)
  runs any tool with no question, so whatever it reads (a task an agent wrote, a comment, a file,
  a web page) is one step from running; Codex then has no sandbox and no hook that refuses.

### Dispatcher's typing and closing

- **Typing**: only in a pane it started and marked, at that agent's empty box, and only what the
  typing gate of `terminum-dispatcher.md` §9 lists: task IDs with fixed words; the prompt of a
  human's RUN, including recent human comments on a continuation (AC-122); a schedule's; a project's manager's; the integration
  queue's own words; your first prompt in + Session's box, once, into the pane that request
  started (`terminum-dispatcher.md` AC-204). A task's words never reach the shell an agent leaves. **Left**: in one
  session of several tasks an agent-written task can still sway the others once read.
- **Closing and ending**: it closes only such a pane, its agent idle, and at a task's time budget
  signals what runs under it by pid and start time, its shell spared, never by name or pattern.
- **Auto-Dispatch** (AC-170), off until you turn it on for a project, starts tasks no human looked
  at, an agent's own too: **an agent that writes tasks can have them run by another, with that
  one's permissions, and those can write more.** What bounds it: its starts in 24 hours (as made:
  10), the severities you tick and a wait (as made: P0 and P1, after 10 minutes), the agents'
  slots and limits, a breaker. **Set the agents' week and month limits**: the cap counts tasks,
  not what they spend. **Left**: a process of yours writing the store could turn it on.
- **A resume after tmux ends** (`terminum-dispatcher.md` AC-205): a run Dispatcher started, at work on a
  task in progress, goes on by itself in a new window once its tmux server is back, with no new press:
  the press that started it stands. It resumes its own conversation with the pool's command as it is
  then, types only the task IDs with fixed words, and never resumes a window closed by hand, + Session's
  or your own sessions; a second end of that server within the hour of a resume resumes nothing.
  **Left**: an agent whose work kills tmux runs once more before the hour stops it; a process in that
  tmux can change the server's identity option and close its own window to have its run resumed, at
  most once an hour, with the same words.
- **Schedules**: a schedule's window shows its prompt and command before Save. **Left**: a
  process of yours writing the store could add one, a recurring run with every tool, so the board
  shows each schedule's author and last change (`terminum-schedules.md` §9).

### A project's manager

The session + Session starts for a project, and a session you open in a project, is its manager
(`terminum-agents.md` AC-174, AC-179, AC-180 and §9 Security). By its tools alone, with no window
shown to you first, it starts tasks on the pool's agents, changes those agents and their commands,
makes schedules, and pastes into and closes the project's other sessions.

- **So** what a manager's agent reads (a task, a comment, a file, a web page, a session's screen)
  is one tool call from a command of its choosing, run with an agent's permissions, a bypassing
  one's too; its client asks about `execute_tool` once, not for each tool behind it. **Start a
  manager only on an agent you would give your own shell, and keep the agents' week and month
  limits set.**
- **Tokens** (`terminum tokens`, AC-181) give a program outside a pane a role in one project:
  keep one in the program's environment, never on a command line.
- **Left**: all of it guards against an agent that errs, never against a process of yours.

### The integration queue

Off until you turn it on in the Integration tab of a project's window. Its stages' lines (Prepare,
Verify, Check, Publish, After, Cleanup) are then **a standing permission to run commands and to
push as you** (`terminum-integration.md` AC-186, AC-191, AC-198 and §9 Security). Only a signed-in
page writes them; an agent chooses only which commit is queued, and no task's text reaches a
commit, a command or a shell. terminumd's own push is one commit whose check was green, never
forced, never a tag. It moves your project's folder only by a fast-forward, and only while `git status --porcelain`
is empty there. **Left**: a check runs the queued code as you, and the check is the
candidate's own, so a task can change what checks it: **read the batches and the published
history.**

### Project Supervisor

The Supervisor is off per project until the owner enables it. Discovery and each call use
the project's current level. terminumd binds the role to its own pane record and checks
local process ancestry both when a caller names that pane and when it names a different
pane; while a Supervisor pane is marked, a call whose process the system does not name is refused. A disabled or replaced generation keeps the pane's role with its tools inactive.
Only the signed-in owner's settings can change the level. Project imports preserve it.
Tokens keep their own roles.

Observer can list and read its project's tasks. Advisor can also create a task, add
comments and ask on a task linked to its pane. Operator can also list runs, read the
project dashboard, promote a queued task, give up a run and start a free task on a
named project agent. Dispatcher supplies the start prompt from the task IDs and fixed
words. Give up frees the slot and leaves the window. Every level refuses terminal
input, session closure, transcript and attachment access, pool and schedule management,
token operations and protected settings. The critical line delivers incidents at every
level. Round and Talk registration and the evidence, budget and hold tools follow their
own acceptance cases (`terminum-supervisor.md` AC-231, AC-232, AC-234, AC-235, AC-237–239,
AC-244, AC-245, AC-258).

- **Rounds and Talk**: Dispatcher writes a fixed case-file ID prompt to the marked round
  pane, through its existing typing gate. Talk requires the human's press and pastes its
  fixed ID prompt once into the pane the press started. Task text stays in bounded data
  fields in the case file. The round reports through typed tools. Outside prose goes
  only to a tool-free reader, whose closed categories and counts can inform a marked
  task. Auto-Dispatcher excludes that task until a human moves it
  (`terminum-supervisor.md` AC-234, AC-244, AC-262).
- **Command channels**: saving in the signed-in Supervisor tab authorizes the ordered
  executables to send fixed incident notices. The owner-wide settings stay out of
  project imports and exports. terminumd starts the absolute executable directly.
  Fixed templates on standard input contain incident IDs, levels, counts and an optional
  link built from the owner's HTTPS origin. Task or comment text, home paths and
  credentials stay out of the template. The child starts in `/` with `PATH=/usr/bin:/bin`
  and `LANG=C`. Its output is discarded. Each attempt has a five-second deadline and
  terminumd ends the process group it started. Incident actions require the signed-in
  page. A manual test sends fixed test text. A weekly check passes `--check` and fixed
  check text, with the wrapper responsible for the silent check. A claimed send can be
  lost if terminumd exits before the command runs. A failed delivery produces a local
  incident and tries the next command. Quiet hours defer ordinary automatic notices
  (`terminum-supervisor.md` AC-227–229, AC-263).
- **Probe commands**: the owner confirms the displayed command. Probe metric output is
  bounded and checked. Import leaves command candidates inactive until confirmation.
  Export includes commands and secrets only when the owner selects them. The export
  holds those values in plain text (`terminum-supervisor.md` AC-220, AC-260).
- **Left**: a round's agent keeps its pool command's permissions, with AC-156's risk of a
  bypassing command (`terminum-dispatcher.md` §9), so choose agents without bypass for the
  Supervisor's tiers. An Operator can start a task whose text or comments it wrote, so the
  worker it starts reads the Supervisor's words with its own pool command's permissions:
  set Advisor to keep every start a human's. A process running as the same user can change the local store, tmux
  or the commands the owner configured. The scoped tools bound model actions through terminumd.
  An owner-configured command runs with that user's OS access and can launch processes that leave its group. Keep the private charter,
  register, case files, ledger and baselines out of tracked files.

### Idle agents' exit

Off until you set its days in Settings. Then terminumd types only Ctrl+C twice and `/exit` with
Enter into a pane whose Claude Code or Codex is idle at an empty box, that nobody showed for those
days and that is not Dispatcher's. What the agent ran in the background ends with it
(`terminum-agents.md` AC-153).

## What is kept on disk

| What | How it is kept, and what is left |
|---|---|
| Sent history | What you send to a shell or an agent is kept on the Mac (0600), for signed-in pages only. A line starting with a space is never kept; values of words named like secrets, and words of secrets' shapes, are written as `[redacted]`. **Left**: a quote that opens inside a value's word (`TOKEN=abc'd e'`) is not followed; other secrets typed on a command line are kept as typed (`terminum-terminal.md` AC-90). |
| Last lines (`lastLines`) | What a terminal printed (a token, a `.env` you displayed) is kept on disk for a shown pane's last lines, so it survives a reboot; what turns that off: `terminum-lifecycle.md` AC-96. |
| Bookmarks | Selected terminal text and your note, which can hold what the terminal showed, are kept 0600 and sent only to signed-in pages (`terminum-terminal.md` AC-33). |
| Reboot restore | `layout.json` (0600) holds the layout, never output, history or environment; only shells are started again (`terminum-lifecycle.md` AC-15). |

## Updates and releases

- **The update check** is the only request terminumd makes by itself: one `HEAD` a day to the
  project's GitHub releases page. Nothing is downloaded or installed; `state/no-update-check`
  turns it off (`terminum-lifecycle.md` AC-17).
- **The installer and releases**: `install.sh` and `terminum upgrade` verify the signature of
  `SHA256SUMS` with the release key they carry, before the checksum, and refuse root. Releases are
  built and signed only on the maintainer's Mac (`docs/RELEASING.md`; `terminum-lifecycle.md` §9
  Security). The folder access macOS grants terminumd passes to every program in terminum's tmux,
  agents too. The repository's rulesets refuse everyone a force-push to `main` or its deletion,
  and a moved or deleted `v*` tag; fast-forward pushes and new tags pass. **Left**:
  whoever controls the maintainer's GitHub account can replace a release's `install.sh`, which a
  *first* `curl | sh` trusts, or remove those rulesets; a program running as the maintainer while
  the Keychain is unlocked can sign.

## Reporting

Report a vulnerability privately through the repository's security advisories ("Report a
vulnerability"), not in a public issue. Include the version (`terminumd --version`), the macOS and
tmux versions, and the smallest reproduction.
