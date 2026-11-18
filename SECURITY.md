# Security

terminumd gives a browser page your terminals (a shell as you) and your files, so it is built to be
reachable only by its own page. This is the threat model: who can reach it, what stops them, and
what is **left**. How each defense works, with its numbers, is in the specs:
`docs/specs/terminum.md` AC-09 and AC-10, and §9 Security of each spec.

Three things hold everywhere below:

- **The password is the boundary.** Whoever signs in, from any device, has your shell, your files,
  your hosts and every attached session.
- **A process running as you is not stopped.** It can read the store, drive tmux or set the
  password. Roles, scopes and guards on agents' tools stop an agent that errs, never a program of
  yours.
- **What an agent reads is data, never instructions**, and every answer of terminum's tools says
  so. An agent
  started without permission prompts is still one step from running what it reads.

## Reaching terminumd

| Threat | Defense |
|---|---|
| Other machines | terminumd listens on `127.0.0.1` only. Other devices come through `tailscale serve` (HTTPS on your tailnet), which runs on this Mac; `tailscale funnel` (the public internet) is refused (AC-10). |
| Other devices | HTTPS with Tailscale's certificate up to this Mac, then loopback. A forwarded request cannot use Finder or the default app, read a drop's paths, type a path into a terminal or choose about updates (AC-10; `terminum-terminal.md` AC-36): a guard against accidents, no boundary, since a signed-in device has a shell anyway. On iOS, Safari may drop a page's storage after 7 days without a visit; the page then asks for the password again. |
| Web pages in any browser (CSRF, DNS rebinding) | Every request that reads or changes terminals or files needs the sign-in, sent as a header and never in an address. The `Host` must be a name terminum serves; writes need a JSON `Content-Type` and the page's exact `Origin`; WebSockets and uploads always need the exact `Origin`; no CORS headers; `/internal/*` answers 404 over TCP (AC-09, AC-10). |
| Other local web servers | A cookie for `127.0.0.1` reaches every local port, so the sign-in is never a cookie: it stays in the page's own origin's storage and goes as a header (AC-09). |
| Getting a sign-in | The password is set only by the `terminum` command over its Unix socket; until then no browser signs in. Of a sign-in terminumd keeps only a hash and a proof key. Sign out ends that browser's; `terminum signout` and a new password end every one and close every page (AC-09). |
| Local processes of other users | Everything under `~/.terminum` is private (folders 0700, files 0600); the CLI talks to terminumd only through the Unix socket, never over TCP (AC-09). |

### Guessing the password

- It is kept only as a salted, slow hash (0600). Wrong ones are braked per address and for all,
  counted apart for this Mac's own requests and forwarded ones (AC-09).
- **Left**: any program on this Mac can try passwords on loopback, and can name any address with a
  forwarded `Host`, so for it only the count of all holds. A sign-in proves the password alone,
  never that its user sits at the Mac.

### A program on terminumd's port

While terminumd is not running (a login, an upgrade, a crash):

- launchd holds the port from the moment the agent is loaded, so no other program can bind it;
  `terminum status` reports a port another program held at load, `terminum restart` takes it back
  (`terminum-lifecycle.md` AC-08).
- A page that reconnects proves the server first, and its requests carry a credential of that
  terminumd's boot, never the secret: a request that reached another program gave it nothing the
  next terminumd accepts (AC-09). Refusing the secret from a browser that sends Sec-Fetch-Site is
  a tripwire, not a barrier. Whoever can read `signins.json` (0600) is this user, who can read the
  browser's storage too.
- **Left**: before the agent is loaded, after an uninstall, when the port was taken at load, and
  on Linux or with `TM_NO_LAUNCHD` (terminumd binds the port itself) while it is down, another
  program can serve a page on the port. That page runs its own script in the page's origin, can
  read the sign-in stored there (the per-boot credentials do not help here) and can ask for the
  password, and `tailscale serve` forwards to it as well: **do not open the page then**.

## What the page shows

| Threat | Defense |
|---|---|
| The page itself (terminal output, a malicious README or SVG) | CSP `script-src 'self'` with no eval; the hardening headers on every answer; `/custom.css` only with the sign-in (AC-09). Terminal output is text: it reaches only the panel's Markdown rules and Mermaid in strict mode, and a diagram guessed from unfenced text is drawn only on your press (`terminum-terminal.md` §9 Security). Links never navigate the page; SVG only through an image; HTML files in a sandboxed frame without scripts; only regular files are read (`terminum-files.md` §9 Security). |
| Bookmarks and traces | Bookmarks hold selected terminal text and your note (0600), sent only over a signed-in socket and shown as text (`terminum-agents.md` AC-98). `?trace=1` records no typed text, screen content, title or host name, in `~/.terminum/logs` (0600); `?trace=2` stays in the page (`terminum-terminal.md` AC-38). |

### A program's clipboard writes (OSC 52)

- A pane's clipboard write reaches a page only just after that page's left mouse release to that
  full-screen pane, and only as Copy, [+] and Ask: nothing is put on the clipboard or typed until
  you press one; what could send keys, or make a pasted line read otherwise than it runs, is taken
  out; a read is never answered (AC-06; `terminum-terminal.md` AC-37). terminum's own tmux keeps
  `set-clipboard off`; a host's or your own server's setting is theirs.
- **Left**: a program that writes the clipboard over and over (a loop, a hostile file shown in a
  pager with the mouse) can have its text offered, or put in place of what the program copied,
  after one of your releases, whatever you dragged over and whatever is on the screen. Copy's
  tooltip (none on a phone) and its toast say only its line count and first line, and Copy does
  not show the text whole: **check what you paste** (Ask and [+] show it whole).

## Terminals and files

| Threat | Defense |
|---|---|
| Typing into tmux | User text reaches tmux only as one quoted argument, `#` doubled where tmux expands formats, so a folder named `#(cmd)` runs nothing. Control characters never go on tmux's command line: keys go by tmux's names, a paste through a tmux buffer; `send-keys -H` only for mouse reports and CSI u keys terminumd builds itself. A paste holding control characters other than tab and line ends is refused (`terminum.md` §9). |
| Typing into the wrong terminal | The Files panel's "insert path" and "cd" name the pane they act on; `cd` goes only into an idle shell; nothing sends Enter; names are quoted for the pane's own shell, or refused (`terminum-files.md` AC-60). |
| Writes outside your files | The Files API writes only under `TM_WRITABLE_ROOTS`, judged on resolved paths; delete moves to the Trash (`terminum-files.md` §9 Security). |
| Quick shell | A tmux window like any other, made in the shown pane's folder (checked as for New window), shown by the page itself in a same-origin frame (`frame-ancestors 'self'`). |

### Your own tmux server

- terminumd attaches only the sessions you named, as a control client of the server on that socket
  (`$TMUX_TMPDIR`, else `/tmp`, `/tmux-<uid>/<socket>`, which tmux keeps 0700 to you). It never
  starts that server, never makes or closes a session there. A socket name is letters, digits,
  `-`, `_`, `.` only; terminum's own socket is refused there.
- On a host the same holds for the tmux its `terminum` session is on: the other sessions there are
  only counted and named, to offer them, never typed into or sized until you attach one (AC-89).
- **Left**: your `~/.tmux.conf` hooks and plugins see its clients. Whoever signs in, also from
  another device, can type into every attached session.

### Remote hosts

- A host is the ssh string you gave, checked against an allowlist: no option or config file can
  hold a ProxyCommand, and no command runs but terminumd's own, without prompts
  (`terminum-remote.md` AC-81).
- **Adding a host is the consent to set it up.** Once it attaches, the files agent is copied there
  and terminum's hooks and MCP server are put into its Claude Code and Codex, with no further
  question; both follow this Mac's build. What you turned off for a host is not set up again
  (AC-83, AC-99).
- A page signed in from another device can add a host too, so **the password also decides which
  machines terminum writes to**.
- The files agent writes only under that user's `$HOME` and `/tmp` and exits when its ssh ends; its
  answers are untrusted, capped and timed. On a host terminumd ends only the user's orphaned tmux
  control clients that are terminum's (`terminum-remote.md` AC-81 and §9 Security, with the two
  accepted cases of a client taken for one).
- **Left**: an agent on a host reaches its project's tasks through the host's link, so a broken
  host can read them and write comments its project's other agents read, which they are told are
  data (`terminum-kanban.md` AC-120).

## Coding agents

### Coding agents' hooks

- `terminum hooks install` writes only terminum's Claude Code plugin folder (never Claude Code's
  settings) and its entries in Codex's `hooks.json` and block in `config.toml`, backed up first;
  Codex runs the hooks only once you trust them in `/hooks`.
  A hook sends a few fields of each event, never a tool's output, a file's text or the agent's
  answer, only to terminumd's socket or, on a host, a Mac's link (`terminum-agents.md` AC-39).
- **Left**: any process of yours can post there. A made-up event can change a listed pane's shown
  state and its tool and wait texts, add a sent entry, show three integers and shape-checked
  subagent IDs on a task's card and, for a session Dispatcher started, move its task as that event
  would. None of its words runs or reaches tmux; only a session ID of a shape that is no flag and
  no shell word reaches a command line.

### Codex's trust of terminum's hooks

Codex trusts a hook entry by the hash of the entry in `hooks.json`, not of the `terminum` program
it names: **an upgrade of terminum changes what the hook does without a new trust prompt**
(terminum keeps its entries byte for byte, so a trusted entry stays trusted). Its MCP tools in
`config.toml` are approved without a prompt; install and remove change only terminum's own lines
(`terminum-agents.md` AC-97).

### Plans and tasks of coding agents

- An agent's plan and what it writes on tasks are model-written text: kept 0600; control
  characters, and what hides what a line reads (bidi controls, zero-width characters, the soft
  hyphen, word joiners, line separators, tag characters), dropped; redacted as sent history is;
  limited in length and number; sent only to signed-in pages; shown as plain text (a plan) or as
  Markdown with raw HTML off (a task) (`terminum-agents.md` AC-97, `terminum-kanban.md` AC-115).
- `terminum mcp` listens on no port: it speaks MCP over the agent's stdin and stdout and forwards
  each call to terminumd's private socket, or on a host to a Mac's link. A worker's tools write
  only terminumd's plan and the store, never tmux or the pane.
- What terminumd tells a session (a stale plan, a human's move of its task) is fixed words with
  numbers, a column and a user name, given through the hook's answer, never typed.
- **Left**: an agent does all on its own tasks, and reads and comments on its project's others: a
  scope, no boundary. `terminum mcp` names the pane from the agent's environment (`TMUX_PANE`),
  and a process of the same user can name another.

### The tasks' store

- `kanban.sqlite` and the files in `attachments/` are 0600 in 0700, on the Mac alone (an Agent on
  a host never opens one), reached only with the sign-in; a file's bytes only with its header,
  never by an address. A file shows inline only as a raster picture checked by its bytes; all else
  downloads, so nothing runs on the page's origin (`terminum-kanban.md` §9 Security).
- Agents' text is redacted, a human's is not: a human can purge a file or a text for good, for a
  secret pasted by mistake (`terminum-kanban.md` AC-109).
- **The input log** holds what was typed into panes, but not a password at a recognized prompt
  (sudo and the like) nor a line begun with a space. **Left**: a password typed at a prompt it does
  not recognize is kept (`terminum.md` AC-147).
- **The interaction log** holds how many presses, keys and scrolls each page counted a minute and
  the ids of the pages' own pressed controls; never a key, a text, a position or anything of a
  viewed file (`terminum.md` AC-175).

### The pools of agents

- An agent's command is what you wrote: one line, run in your login shell as if you typed it, kept
  and shown as written (`terminum-dispatcher.md` AC-137). **Keep a secret in your shell's
  environment, never in the command.**
- Anyone signed in can write one on the board, which is no more than typing in a terminal there;
  so can a project's manager by its tools; a worker's `terminum mcp` has no tool for them.
- **Left**: a process of yours can change the store, so RUN's dialog shows the command in full
  before it runs.

### A run's agent's permissions

- Dispatcher starts an agent's command as the operator wrote it: **the permissions are the
  command's.** It adds only the agent's inline TUI, its effort and, going on, the resume of its
  conversation (`terminum-dispatcher.md` AC-156).
- **A bypassing command** (`claude --dangerously-skip-permissions`, `codex
  --dangerously-bypass-approvals-and-sandbox`) runs any tool with no question, so whatever it
  reads (a task an agent wrote, a comment, a file, a web page) is one step from running. Codex
  then has no sandbox and no hook that refuses, so this repository's guard of the primary checkout
  does not hold for it.
- What reaches it as a prompt: a human's RUN, the task's text shown in RUN's window with an
  agent's part marked, and after its questions only the humans' comments (AC-157); or a manager's
  `run_task`, with no window, its message said to be the manager's.

### Dispatcher's typing and closing

- **Typing**: only in a pane it started and marked, with the screen an empty box of that agent,
  and only what the typing gate of `terminum-dispatcher.md` §9 lists: task IDs with fixed words;
  the prompt of a human's RUN, whose window shows it first; a schedule's; a project's manager's;
  the integration queue's own words; your first prompt in + Session's box, once, into the pane
  that request started (`terminum-dispatcher.md` AC-204). Several tasks run together go by their numbers and your
  message alone, each text shown to you first. A human's keys through the page wait behind it (a
  tmux client you attach yourself does not). A task's words never reach the shell an agent leaves.
- **Closing and ending**: it closes only such a pane, its agent idle. At a task's time budget it
  signals what runs under the pane it started, by pid and start time read just before, its shell
  spared; never by name or pattern. Its `ps` reads no arguments.
- **What a hook sends for the board**: integers and percents only (`terminum-dispatcher.md`
  AC-168, §9 Security); a made-up one changes the board's stripes, nothing else. `terminum
  statusline` runs your own status line command, as Claude Code would, with the input it gave.
- **Left**: in one session of several tasks an agent-written task can still sway the others once
  read.

**Auto-Dispatch** (`terminum-dispatcher.md` AC-170), off until you turn it on for a project:

- On, tasks start that no human looked at, an agent's own too. Dispatcher types only its own words
  and task IDs, and a task's text reaches its agent as the assignment it reads: **an agent that
  writes tasks can have them run by another, with that one's permissions, and those can write
  more.** Set to the board's order it starts them itself, top down, and no agent reads them first.
- **What bounds it**: the tasks it starts in a project in 24 hours (as made: 10); the agents'
  slots; the week and month limits of the agents you gave one (they stop its starts, the picking
  agent's too, never yours); the severities you tick and the wait before a task is taken (as made:
  P0 and P1, after 10 minutes); one round a project at a time, of at most 5 starts; no task
  started twice unless you moved or edited it; half an hour's pause after a round that started
  nothing; its breaker, which turns it off after a failed round or two failed tasks and says so on
  the board.
- **Set the agents' week and month limits**: the cap counts tasks, not what they spend.
- Only a signed-in page writes the setting and a task's agent; `terminum mcp` has neither.
  **Left**: a process of yours writing the store could turn it on.

**Schedules**: a schedule's window shows its prompt and command before Save, and its tasks carry
only its own text. A signed-in page writes one and so does a manager; a worker's `terminum mcp`
has no tool for one. **Left**: a process of yours writing the store could add one, a recurring run
with every tool, so the board shows each schedule's author and last change
(`terminum-schedules.md` §9).

### A project's manager

The session + Session starts for a project, and a session you open in a project, is its manager
(`terminum-agents.md` AC-174): by its tools alone it does in that project what you do on the
board, with no window shown to you first.

- **What it does** (AC-179, AC-180): starts tasks on the pool's agents through Dispatcher's own
  gate, its words said to be the manager's, never yours; adds and changes the pool's agents, whose
  command runs in your login shell; makes schedules, which run again and again with no one
  watching; pastes text and Enter into the project's other sessions and closes them.
- **So** what a manager's agent reads (a task, a comment, a file, a web page, a session's screen)
  is one tool call from a command of its choosing, run by an agent with that agent's permissions,
  a bypassing one's too; and its client asks about `execute_tool` once, not for each tool behind
  it. **Start a manager only on an agent you would give your own shell, and keep the agents' week
  and month limits set.**
- **The record**: what it makes carries its name on the card and in the task's history ("Opus ·
  manager"); each call is a line of the log (`manager.tool`); what it typed is in the input log.
- **Who is a manager** is terminumd's own record, read at each call; a worker neither finds nor
  runs these tools. On this Mac a manager's call must come from a process under the manager's own
  pane; a host's manager is known by the pane its call names alone (AC-178).
- **Tokens** (`terminum tokens`, AC-181) give a program outside a pane a role in one project.
  terminumd keeps their SHA-256 alone and refuses to make, remove or take one from inside a pane.
  A token is no secret from this user's processes: keep it in the program's environment, never on
  a command line.
- **Left**: all of it guards against an agent that errs. A process of yours can leave its
  ancestry, write the store, drive tmux, or set the page's password, sign in and press + Session.

### The integration queue

Off until you turn it on in the Integration tab of a project's window. Its stages' lines (Prepare,
Verify, Check, Publish, After, Cleanup) are then **a standing permission to run commands and to
push as you** (`terminum-integration.md` AC-186, AC-198, §9 Security):

- Only a signed-in page writes them; no agent's tool changes them. An agent chooses only which
  commit is queued, and no task's text reaches a commit, a command or a shell.
- terminumd's own push names one remote, one branch and one commit, never forced, never a tag,
  and only for a candidate whose own check was green on the target it was checked on; the
  project's folder is only fast-forwarded, and only when it stands clean on that branch (AC-191).
- **Left**: a check runs the queued code as you, as the agents that wrote it already do, and the
  check is the candidate's own, so a task can change what checks it: **read the batches and the
  published history.**

### Idle agents' exit

Off until you set its days in Settings. Then terminumd types only Ctrl+C twice and `/exit` with
Enter, fixed words of its own, into a pane whose Claude Code or Codex is idle at an empty box (no
draft, no question: a permission is never answered), that no page and no tmux client of yours
showed for those days, and that is not Dispatcher's. What the agent ran in the background ends with
it. Its summary is output to the pane's tty, never input (`terminum-agents.md` AC-153 and §9
Security).

## What is kept on disk

| What | How it is kept, and what is left |
|---|---|
| Sent history | What you send (commands at a shell prompt, prompts to an agent) is kept on the Mac (0600, folders 0700) and sent only to signed-in pages. A line starting with a space is never kept; values of words named like secrets, and words of secrets' shapes, are written as `[redacted]`, a quoted value hidden whole. **Left**: a quote that opens inside a value's word (`TOKEN=abc'd e'`) is not followed; other secrets typed on a command line are kept as typed (`terminum-terminal.md` AC-90). |
| Last lines (`lastLines`) | What a terminal printed (a token, a `.env` you displayed) is kept on disk for a shown pane's last lines, so it survives a reboot there, where tmux alone keeps it in memory only. Where, who reads them, when they go and what turns them off: `terminum-lifecycle.md` AC-96. |
| Reboot restore | `layout.json` (0600) holds the layout; never output, history, environment or a command line other than a checked ssh recipe; only shells are started again (`terminum-lifecycle.md` AC-15). |

## Updates and releases

### The update check

The only request terminumd makes by itself: one `HEAD` to the project's GitHub releases page a
day. Only a tag `vMAJOR.MINOR.PATCH` is kept and shown as text; nothing is downloaded or
installed; `state/no-update-check` turns it off (`terminum-lifecycle.md` AC-17).

### The installer and releases

- `install.sh` and `terminum upgrade` verify the signature of `SHA256SUMS` with the release key
  they carry, before the checksum, and refuse root. An upgrade installs only what the key signed.
- Releases are built and signed only on the maintainer's Mac, with locked dependencies
  (`docs/RELEASING.md`); the key exists only there (and in its backups).
- Each macOS terminumd is also signed with the maintainer's Developer ID as
  `io.terminum.terminumd` (no hardened runtime, not notarized), so the folder access macOS grants
  it, which every program in terminum's tmux inherits, is kept across upgrades and cannot pass to
  a binary signed by anyone else.
- **Left**: whoever controls the maintainer's GitHub account can replace a release's `install.sh`,
  which a *first* `curl | sh` trusts; a program running as the maintainer while the Keychain is
  unlocked can sign.

## Reporting

Report a vulnerability privately through the repository's security advisories ("Report a
vulnerability"), not in a public issue. Include the version (`terminumd --version`), the macOS and
tmux versions, and the smallest reproduction.
