# Using terminum

Commands, the page, tasks, your own tmux, other devices, remote hosts, configuration. Overview and
install commands: [README](../README.md). Exact behavior and limits: the specs
([docs/README.md](README.md) says which).

## What it is

| Part | What it does |
|---|---|
| **terminumd** | One server on `127.0.0.1:48080`: drives a private tmux server (`tmux -L terminum`) and each remote host's tmux, serves the page ([how the parts fit](ARCHITECTURE.md)). |
| **The page** | Session list with coding agents' states; terminal (keys and mouse reach full-screen programs); code, Markdown, diagrams and tables rendered in the output; folds, bookmarks, search, hot keys for phones, paste and upload. |
| **The Files panel** | Tree of the shown pane's folder; viewer and editor with highlighting; create, rename, Trash, live refresh. |

**Files**

- Rendered: Markdown with Mermaid, JSON and YAML as a tree, CSV and TSV as a table ("Source" shows
  the text); pictures; PDF, video and sound in the browser's own viewer and player (AU and AIFF
  decoded where it cannot play them).
- MIDI plays with the bundled General MIDI SoundFont, or yours: `~/.terminum/soundfont.sf2` or
  `midi.soundfont` in settings.json.
- Esc in View returns to the terminal; Esc in the tree hides it and keeps the keys there (Enter,
  Esc, ↓, Enter… walks the files).
- Wide window: docked beside the terminal, open or closed per session, width by dragging its
  border. Phone: a
  view of its own.

**Sizing**

- The page that opens a pane, comes to the front or types sizes it to its view (45×14 to 240×70
  cells). Text is never scaled: it re-flows; a full-screen program is drawn as its grid, scaled
  when wider.
- Where a coding agent (Claude Code in its default TUI) draws on the normal screen, looking
  changes nothing: only typing sizes its window. Such an agent prints its answer again at every
  new size, and tmux's history would hold it twice. Until you type, the pane is re-flowed to
  your view.
- In a window of several panes the one you look at is zoomed, the layout kept; only typing
  shrinks the window. Your own tmux, or a tmux client showing that window, keeps its layout
  unzoomed.

**Sessions outlive everything**

- Terminals live in tmux; terminumd comes back after a crash (launchd).
- After a reboot or a killed tmux server, sessions, windows, splits and folders are restored:
  terminum's, each host's, and attached sessions of your own tmux (`"restore": true`).
- A task Dispatcher was running, in progress, goes on by itself in a new window, its agent told
  the session was resumed; one waiting for you, or a window you closed, does not. A second end of
  that tmux server within the hour resumes nothing: those tasks are FAILED, and RUN resumes them.
- Each session's last 300 lines are kept (`"lastLines"`: up to 2000, 0 for none and to delete
  them) in `~/.terminum/state/lastlines/` (private, not in Time Machine) until the pane closes: **Show Last 300 Lines** in the
  row's menu. A full-screen program's screen is not kept.

## Install

Commands: [README](../README.md#install). What they do:

- **Installer**: checks the release's signature; installs into `~/.terminum/` (`builds/<build>`,
  `current`, `bin/terminum`, `daemon/terminumd`); loads the launchd agent `io.terminum.terminumd`,
  which holds the page's port ([SECURITY.md](../SECURITY.md)).
- **Folder prompts (macOS)**: during the install, on the Mac's screen, once per folder: may
  terminum's terminals read
  Desktop, Documents, Downloads and each cloud folder (`~/Library/CloudStorage`). Allow covers
  every shell and agent and stays across upgrades. The install waits 5 s and names the folders
  still asked about; `terminum folders` asks again and waits. Refused: System Settings › Privacy & Security ›
  Files and Folders.
- **iCloud Drive** (`~/Library/Mobile Documents`) stays closed: macOS refuses it without Full Disk
  Access and never asks. Keep work in Documents, Desktop (also when they sync to iCloud) or
  `~/Library/CloudStorage`.
- **tmux server started by another program** (an older build, a build run by hand, `tmux -L
  terminum` typed in Terminal): macOS judges the terminals by it, and `terminum status`, the
  install and `terminum folders` say so. Restart it at a quiet moment (reboot, or `tmux -L terminum
  kill-server`): sessions, windows, splits, folders and projects come back; programs do not
  (`claude --continue`, `codex resume --last`).
- **Linux**: the systemd user service `io.terminum.terminumd` (`~/.config/systemd/user/`), started
  at every login; restart, upgrade and uninstall end
  terminumd alone, never the terminals. Without linger everything ends at logout (`terminum
  status` says so; the command is in the [README](../README.md#on-a-ubuntudebian-server)). No
  Finder, no default apps.
- **From a checkout**: `make install` (Rust, Node 26, Xcode Command Line Tools;
  [CONTRIBUTING](../CONTRIBUTING.md#worktrees)).

## Use

```bash
terminum password             # the password every browser signs in with (8+ characters)
terminum open                 # the page in this Mac's browser: http://127.0.0.1:48080/
terminum status               # build, pid, port, password set, tmux servers and sessions, hosts, pages
terminum folders              # macOS asks now for the protected folders the terminals read
terminum signout              # every browser signs in again
terminum upgrade              # the latest signed release (--to vX.Y.Z); rollback; uninstall
terminum restart              # load the launchd agent again (a new port in settings.json; a port taken back)
terminum logs                 # terminumd's log (-n N, -f)
terminum hooks install        # Claude Code's and Codex's hooks and MCP server: states, plans, the board's tools
terminum import               # workspaces, bookmarks and hosts of iterm-enhancer
terminum help                 # every command
```

Every browser signs in with the password ([SECURITY.md](../SECURITY.md)). `tmux -L terminum attach`
in any terminal shows the same sessions.

## The page

### Session list

| Thing | What you can do |
|---|---|
| A row | Hatched = unread (its agent ended a turn nobody saw). Clone opens a window beside it in its folder. |
| Reorder | Drag a row, or a group's or a project's header. |
| A row's menu (right click; touch: rest a finger, lift) | Clone, Add to Project, Mark as Unread or Read, Diagnostics…, Split Below, Right, Left, moves, Move to Session, Close. |
| Diagnostics… | What terminumd and tmux see of the pane, for a bug report. It holds the pane's folder, host and title: check before sharing. |
| A tmux session's group | Header buttons: copy, import, [+], ⋯. Menu: New Window, moves, Close Group. |
| A project | A named, colored group of sessions, the same on every browser; [+] opens a window in its folder. Menu: New Window, Rename, moves, Delete Project (its sessions and tasks go to Default), Close Project (closes its sessions, keeps it); both ask. |
| Under the list | New session, Bookmarks / Reminders, Add host, Attach tmux sessions. |
| The header's ⋯ | Settings. |
| Tip of the day | At the list's foot on a computer: Next, × (until tomorrow), Turn off. Settings brings it back. |

Claude Code in a worktree: its row, the header's folder, the Files tree and the paths it prints
follow it. Off: Settings' Follow the agent's folder (this browser); `page.followAgent` in
settings.json (browsers that did not choose).

### Terminal

- **Header**: the pane's folder (the copy button copies its path), tmux's `session:window`, what
  runs there.
- **Quick shell**: the header's button (or the menu's Quick shell below / on the left) opens a
  shell beside the shown pane, in its folder: drag its edge to size it, ⇄ moves it, × closes it. It is a tmux window, listed under its
  pane.
- **The view's menu**: 20 color schemes (one for all browsers, in settings.json), the Markdown
  text size, the history lines of new
  terminals.
- **Sent history** (the speech bubble under the split): what you sent in the pane's folder, newest
  first (↑ older, ↓ newer); Jump to message, Insert into chat (no Enter). The blue number under it: the tasks made in
  this session.
- **Side marks** (wide window, the right edge): your messages (white), ends of the agent's answers
  (pale blue), bookmarks (orange), reminders (pink); press one to go there. Off in Settings.
- **Ask**: select text, press the blue speech bubble with a question mark, type a question: Add to
  Chat puts the selection as a numbered Markdown quote (`> Quote #0007`) and your question into the
  input, without Enter. The asked text keeps a light blue wavy line.
- **Full-screen programs with the mouse** (Claude Code's fullscreen mode, vim with `set mouse=a`,
  mc): drags and clicks are the program's. When it copies its selection to the clipboard (OSC 52),
  the page shows Copy, [+] and Ask by it. Hold **Shift** while dragging to select in the browser
  instead.
- **Idle agents' exit** (Settings: "Exit idle agents after, days"; off until set): a Claude Code or
  Codex idle at an empty box, in a pane nothing of yours showed for that many days, is ended
  (Ctrl+C twice, then `/exit`); never one that works, asks, holds a draft or is Dispatcher's. Its
  pane then shows a `Resume:` line with the options it had (`specs/terminum-agents.md` AC-153).

| Key | Does |
|---|---|
| ⌘[ ⌘] | Back and forward through the last 20 sessions shown |
| ⌘F | Search the history |
| ⌘B | Open or close Files |
| Option or ⌘ with ↑/↓ | Page up and down |
| ⌘Z, ⌘⇧Z | The program's undo (Claude Code, zsh, bash, fish) and redo (fish) |
| ⌘X; Backspace, Delete | Cut or delete the text selected in a coding agent's input |

**On a phone**

- The floating buttons but the keyboard's two fold into ⟨…⟩ at the top right: a tap shows them
  for 3 s.
- A selection shows the page's Copy, [+] and Ask instead of iOS's menu.
- On the input: hold a finger half a second and lift for Cut, Clear, Paste, Copy, Undo, Redo; a
  quick swipe right is →, left is Undo; two taps select a word.

### Paste, drop and upload

| You do | You get |
|---|---|
| Paste a screenshot into the terminal (⌘V) | Saved in `~/.terminum/tmp` (on a host: the host's), its path pasted; Claude Code takes it as an image. |
| Drop files or folders from Finder on a session of this Mac, in this Mac's browser | Their own paths at the input, quoted, no Enter. Nothing is uploaded, at any size. |
| Drop a file anywhere else (a host's session, a phone, from a browser or Mail), or Upload | Uploaded as a screenshot is, its path pasted; up to 100 MB (larger: give its path; in Finder ⌥⌘C copies it). |
| Drag files or folders from Files onto the terminal | Their paths at the input, quoted, no Enter; a selected row brings every selected row's. |
| Drag files from Finder onto the Files tree | Uploaded into the folder under the pointer (below the rows: the root), on this Mac or the pane's host. A name already there asks Keep Both or Skip. |

### Bookmarks and reminders

- **A reminder** is a bookmark with Reminder picked in its dialog. Its bell comes up when the
  session's agent ends a turn; the terminal's top says so for 4 seconds ("api: 16 reminders, 3
  new").
- **They show** only in the session they were made in. Bookmarks / Reminders under the list holds
  them all; after their session closes, only there ("orphaned").
- **The window** (the bookmarks' button; the bell opens it with open reminders only): the list on
  top, the chosen one's content under it. ↑ ↓ choose, Enter shows it in the terminal, Resolve or
  Delete settles one (Undo for 5 seconds), Esc closes.
- **Quote in chat** pastes a bookmark's or a reminder's note and text into the session's input, no
  Enter. **Convert to task** makes a task in TODO.
- **Limits**: 300 bookmarks and 100 open reminders (a save past them says so). Coding agents'
  records: 50 bookmarks and 20 open reminders a session, their oldest going first (a banner says
  so). A resolved reminder is deleted after 30 days.

## Coding agents

`terminum hooks install` sets both up (refreshed at each upgrade, where you installed them);
`terminum hooks` says what is set up; `terminum hooks remove` or `terminum uninstall` takes them
out.

- **Hooks**: Claude Code (terminum's plugin, added with `claude plugin`) and Codex (entries in
  `~/.codex/hooks.json`: trust them once in Codex's `/hooks`) tell terminum when they work, wait
  for an approval or an answer, stop, fail or run subagents. Without hooks terminum reads their
  screens.
- **MCP server** (`terminum mcp`): the agent keeps its plan and works the board. Claude Code gets
  it through the plugin; Codex as one block in `~/.codex/config.toml` between `# >>> terminum: MCP
  server …` and `# <<< terminum` (the file backed up first, nothing else changed). An
  `[mcp_servers.terminum]` of your own, or a file changed during the install, is refused. Restart
  running Codex sessions.
- **Its tools**: the list holds only `search_tools`, `get_tool_info`, `execute_tool` and
  `update_session_plan`; every other tool is found by its action and run through `execute_tool`
  (`execute_tool("finish_task", {id, result})`). Outside a terminum pane the server offers no
  tools and costs nothing.
- **Notices when an agent starts**: when Claude Code or Codex starts where terminum does less, the
  first page that shows its pane opens a window once for that run, with the commands to copy:
  - *full screen* (Claude Code's `"tui": "fullscreen"`, Codex on the alternate screen): no history to
    scroll back, no widgets, folds, find or copy of earlier output. Switch Claude Code with `/tui
    classic` (or `"tui": "default"` in `~/.claude/settings.json`); start Codex with `codex -c
    tui.alternate_screen=never`.
  - *Codex without terminum* (plain `codex` attached to a shared `codex app-server`): no state,
    plan, board or tools; restart it as the window says (also under "When the Plan pane is empty or
    wrong").

  Tick **Do not show again** to turn that kind off on every browser; **Agent notices** in Settings
  turns both back on (settings.json's `agentNoticesOff`). vim, mc and other programs never get one;
  nor do a host's panes.

## Tasks

The board: `/kanban/` (Kanban among the sections, top left). A task has an ID (TERM-7), a type, a
severity, an owner, a project, Markdown text and comments, files, subtasks, and stands in TODO, IN
PROGRESS, WAIT FOR HUMAN, FAILED or DONE.

### The board

- **Columns**: TODO is always shown; the others share the rest under tabs with counts (a phone:
  one column). WAIT FOR HUMAN and FAILED also show in a line on both pages.
- **A card** carries its session's row (a press opens the session), its CPU, memory, running time
  and tokens, and its subtasks as boxes. TODO and DONE cards are one line: a press unfolds one, a
  second press opens its window.
- **Order**: per column, recently updated first, or priority then recently updated (the default);
  kept per project in this browser. A drop inside a column changes no order; a drop on an agent
  runs the task.
- **Projects**: the key in the head chooses one; the pencil on its row, or New project…, opens its
  window.
- **Agents** stand over the columns. Each project has its own pool (the Agents tab of its window).
  An agent's window: its subscription stripes and reset times, its tokens of 7 and 30 days, its
  live sessions with their tasks.
- **Sections**: Agent Pool, Routines and Task Management, each named on a rail at its left. The
  pool's and Routines' rails fold their section to one strip that keeps its +; each section's +
  tile follows its last card.
- **+ Session** on an agent's card opens a box under the pool: type a prompt, paste or add files,
  drop task cards to name their IDs; Start opens that agent's session and types the prompt once.
  The list head's + Session starts one in the project in view, with nothing typed.
- **New task**: + New task in TODO (drop or paste files in), a file dropped on the board, a
  session's menu, a terminal's selection (New task from selection), Convert to task on a bookmark.
  ⌘Enter in a text keeps it.
- **A task linked to a session** shows on its row; when you move it, its agent hears of it at its
  next tool call.

### A task's window

- **Layout**: the task, its subtasks (+, Add existing, Make it a subtask, ×), then comments and
  history as a conversation, the comment field last. What waits for you (a question, a result, a
  failure) comes first.
- **Keys**: Save keeps your comment; Run starts the agent with it; Run from scratch is on the
  Status line.
- **Comments reach the working agent**: a comment you save, or one another session's agent adds
  (the project's manager's too), reaches the agent of the task's open session at its next tool
  call, through its hooks: a short notice with the task's ID and who wrote it, never the
  comment's words, which it reads with `get_task`. Nothing is typed: an idle agent hears it at its
  next turn, and Run has it act now. Each comment says "told the session", "the session hears of
  it at its next tool call" or "not told the session" (the history line says why). A session's
  own agent hears nothing of its own comments.
- **A fresh start** (the first Run, Run from scratch, a manager's `run_task`) carries the humans'
  and the project's manager's comments in its words; agents' comments only by a count, to read
  with `get_task`.
- **Details**: Type (Feature, Bug, Debt, Story; also set by the title's first word), an External
  link (http or https), a Target, Blocked by and Follow-ups (tasks of the same project, each with
  a reason; kept and shown, nothing waits on them).
- **ID**: the project's letters and a number. The letters come from the project's folder and can
  be changed in its window; an old ID (T-12) and the window's 8-character id still find the task.

### Running a task

A task moves only by running.

- **Start**: drop it on a free agent (⌘+press selects several), add a message, press Run; or Run
  in the new-task window.
- **Dispatcher** opens a window, types the task when the agent's box is empty, moves the card,
  closes the session 5 minutes after it ends.
- **The agent's command** runs as you wrote it: put the permissions it needs there (`claude
  --dangerously-skip-permissions`). Dispatcher adds only the inline TUI, Codex's own process, and
  the effort; Run's window shows the whole line. A Claude Code command with its own `--settings`
  gets none added: set `"tui": "default"` there. Without permission prompts, allow `execute_tool`
  (Claude Code: `--allowedTools mcp__plugin_terminum_terminum__execute_tool`): that allows every
  task tool. Codex needs
  terminum's hooks trusted in `/hooks`.
- **Budget**: the agent gets the task's Budget in minutes, is told at the start and the end, and
  is ended at the limit.
- **Memory**: a session over the project's limits is warned, then ended (4 and 8 GiB unless set:
  `dispatcher` in settings.json, or the project's Memory limits tab).
- **A question** comes with `ask_human` (why you are needed, the recommended answers, the whole
  issue): the task waits in WAIT FOR HUMAN and its session closes. Answer in a comment and press
  Run: the same conversation goes on (on another agent of its kind, if dropped there).
- **The result** comes in fixed sections (Outcome, Done, Checked, Not done, Needs a human, New
  tasks, Tasks changed, Issues and discoveries, Further recommendations), in the project's
  "Language of agents' writing on the board" (empty: English), also used for its tasks and
  comments. What the agent noticed becomes tasks.
- **Subtasks** not DONE run in the same session, in the agent's order, the main task last; one
  that cannot be done fails with the main task, and Run again runs what is left.

**Queue**

- Run on a busy agent offers Add to queue: the task starts when a slot frees. New waits join last,
  whatever their severity.
- The agent's "queued" key opens the queue: drag or use the arrows to reorder, the cross to take
  one out (also Stop waiting in the task's window).
- A manager moves a waiting run to the front with `promote_queued_task {id}` (the primary task
  `list_runs` shows): the same agent and conversation; slot limits still apply.
- Sessions waiting in INTEGRATION do not count toward the at-once limits; a returned run counts
  again.

### Agents on the board

- **Tools**: `list_tasks`, `get_task`, `create_task`, `link_task`, `take_task`, `move_task`,
  `update_task`, `ask_human`, `add_comment`, `attach_file`, `get_file`, `finish_task`, on every
  task of their project, except one a human's RUN holds in another session, until that session
  closes.
- **`create_task`** follows the project's template (its window: Template for agents' tasks).
- **An agent's bug** (type Bug, or a title starting `Bug:`) is never below P2 (its tools refuse
  P3): P1 or above with impact (lost or blocked work, a wrong result, a red main), else P2.
- **`update_task`** grooms the board: severity and parent of any task (a human's with a reason,
  kept as a comment); title, text and archiving only of tasks agents wrote. Your words and tasks
  are never reworded or archived by an agent.

### Auto-Dispatch

A switch in the board's bar, off by default: it starts a project's TODO tasks by itself. With
several projects shown the switch only turns it off; each project turns it on in its own window.

| Setting (the project's window, Auto-Dispatch tab; the bar's cog with one project shown) | As made |
|---|---|
| The agent that picks | none: choose one |
| An agent for tasks that name none | None: such tasks are left to you |
| Severities it takes | P0 and P1 |
| Minutes a task stands in TODO first | 10 |
| Tasks started in 24 hours at most | 10 |
| How tasks are picked: an agent's scoring, or priority and saved order | none: choose one |

- **An agent's scoring**: when slots are free, one more session of the picking agent scores the
  first tasks by RICE and starts the best, each on the task's Agent, else the default.
- **Priority and saved order**: P0 first, then saved rank, started by terminum itself.
- What a round leaves is passed over for a day. A task it started is started again only after you
  move or edit it.
- A failed round or two failed tasks turn it off; the switch says why.

### Schedules

Routines, the board's word for them, are a strip over the columns; its + opens one.

- **Fields**: title, text, project, severity, agent, When, a time budget (required), Paused. What
  the agent gets shows the prompt, the command and where it runs.
- **When**: every 1, 5, 10, 15, 20, 30 or 45 minutes, every N hours, every day, weekdays, chosen
  days of the week, a day of the month, once, or five cron fields; the Mac's local time; the next
  three runs are shown.
- **A firing** makes a task S_TERM-8 (S_ and the project's next ID), IN PROGRESS, never in TODO, in
  the project's folder, and runs it on
  the schedule's agent; its result has Outcome, Summary, Details and For the human, or it asks you
  with `ask_human`. Its card says SCH-1.
- **No run**: a firing while a task of it still runs is skipped (counted on the card); one while
  a task of it waits for room replaces that wait, unless you ran that task yourself; one missed while the Mac slept is not run on wake; one that cannot start (no folder, no hooks,
  agent gone) fails its task and says why, and the next firing tries again.
- **Run now** starts one at once; the cadence stays.
- A schedule in Default runs in no project: its agent reaches only its own task. An agent keeps
  one slot for every two of its schedules not paused, shared by them; with no room, a schedule's
  task waits ahead of the agent's and project's other waits.
- You and a project's manager make schedules; a worker cannot.
- `/scheduled/` (Scheduled beside History): every firing, newest first, with the prompt that went
  in and the result that came out, or why nothing ran.

### Integration

Off until turned on in the Integration tab of a project's window: the target, the Check, Publish
and After commands, the batch's size, and the pool's agent that reads a red check's log, with its
budget and prompt. The whole design: `docs/specs/terminum-integration.md`.

- A task's agent queues its commit with `integrate` (from a shell: `terminum integration queue
  <ID> --result-file <path> --wait`).
- terminumd builds batches of the commits, runs the Check on each, publishes a green one, and
  gives a red one to that agent, which returns the task that broke it, checks again or adds a fix.
- The INTEGRATION column shows the batch in work, then the queue, each card with its place or what
  it waits for; Pause and Batches… in its head; Integrate and Take out in a task's window.
- When terminum's tmux server ends (a crash, a reboot), queued tasks stay queued; a Check, Verify
  or Publish runs again; an After does not, and a P1 task "After's outcome is not known for batch
  N", yours, asks you to check its work and do it by hand if it is not done.

### Managers and workers

- **Manager**: the session + Session starts for a project, or one you open in a project from the
  session list (a crown on its row). **Worker**: every other session.
- **A manager's tools**: run the project's tasks on its pool (`run_task`), follow them
  (`list_runs`, `get_task`), answer and run again (`run_task` with a comment), stop them; add,
  change and remove the pool's agents and the project's schedules, run a schedule now; list, read,
  type into and close the project's other sessions; read its numbers. A worker neither finds nor
  runs these.
- What a manager makes is in force at once and carries its name ("Opus · manager"); what it tells
  a worker is said to be the manager's, never yours.
- **From a shell**: `terminum tools` lists what the pane's session may run; `terminum tool <name>
  '<json>'` runs one.
- **From a program outside terminum**: a token in `TERMINUM_TOKEN`. `terminum tokens add <name>
  --role manager|worker --project <project> [--days N]` (in a terminal outside terminum) prints it
  once; `terminum tokens` lists them; `terminum tokens remove <name>` ends one.
- Roles guide agents; they are no wall against a program of yours ([SECURITY.md](../SECURITY.md)).

### Subscription limits on the board

Each agent shows two stripes: the used share of its subscription's five-hour window and week, one
reading for every agent of its kind. Its window says when each resets.

| A stripe | Means |
|---|---|
| Green, yellow, red | Under 40%, 40% to 90%, over 90% |
| "not known" | Nothing said it yet |
| "reset" | The window started anew since |
| "none reported" | The account has no such window (a Codex plan with a week alone) |
| Faded | Read over an hour ago |

- **The usage command** reads them with no model call, so no tokens: when terminumd starts, every
  10 minutes while a run of that kind works, when one of its tasks ends, hourly otherwise. A
  failure shows in the agent's window; the last reading stays, and it asks again every 10 minutes.
- **Defaults**: `claude -p /usage --output-format json --no-session-persistence
  --strict-mcp-config --setting-sources local` (none of your hooks run) and `codex app-server`
  (asked `account/rateLimits/read`). Change one, or turn it off with `""`, in
  `~/.terminum/settings.json`, for example when the program is not on terminumd's PATH:

  ```json
  "dispatcher": { "usageCommands": { "claude": "/Users/alex/.local/bin/claude -p /usage --output-format json", "codex": "" } }
  ```

- **Codex** also says its limits in its session log; terminum's hook reads them at each turn's end.
- **Claude Code** also says them to its status line. terminum never edits Claude Code's settings:
  add it yourself in `~/.claude/settings.json`, your own status line command after it as one
  argument:

  ```json
  "statusLine": { "type": "command", "command": "terminum statusline 'your own status line command'" }
  ```

  Alone, `terminum statusline` prints "5h 34% · week 21%". It sends terminumd only percents, window
  lengths and reset times, only from a pane terminum lists; with an API key, Bedrock or Vertex
  nothing is sent.

## History and dashboard

Both live in `~/.terminum/state/kanban.sqlite`, kept 180 days.

- **Input log** (table `inputs`): everything terminum writes into a pane: what you type (a row per
  Enter), paste or the Files panel types, from which browser and address, and what Dispatcher pastes and launches, with
  its run, tasks and agent. Input at a password prompt, or a line begun with a space, keeps only
  why it is not kept. Read it with `sqlite3` (columns and a query: `docs/specs/terminum.md` §9,
  AC-147).
- **Message history** (`/history/`, Messages among the sections; table `messages`): every prompt
  and command sent, each row saying who sent it (you, Dispatcher, a manager), to which agent or
  shell, in which session, project and tasks. The rail narrows by kind, sender, day and project,
  with counts. Search by words (`"a phrase"`, `-word` to leave out; a word also matches as a
  start). A row opens to its text, its session at that message, its tasks and what was sent around
  it. Every filter is in the address, so Back and links work.
- **Dashboard** (`/dashboard/`): a period (Day, 2 weeks, Month, Quarter, Year), one project or all.

| Tab | Shows |
|---|---|
| **You** | Active time and days, engagement, longest stretch, switches between sessions an hour, board actions, how long tasks waited for you, off hours, the phone's share, each against the span before; by project, by hour and day. Never Dispatcher's work. |
| **Tasks** | The operations on tasks you decided (made, edited, moved, commented on, run, archived) beside the agents' and Dispatcher's, over time and per task. |
| **System** | Dispatcher's sessions and how their tasks ended, failures by reason, the agents' load, schedules' firings. |
| **Costs** | Tokens and dollars at API list prices of Dispatcher's sessions by agent, project and model; each agent's use against its limits. |

- **Active time**: what you send, type, press, scroll and do on the board; a pause over 5 minutes
  ends a stretch (15 with a page in front). **Engagement**: the share of active minutes you acted
  or read in, counted from the day the pages first counted.
- **Counted** by the minute into the table `user_interaction`: how many presses, keys and scrolls
  and where, never which key or what text.

## Plans, progress and notes

With terminum's hooks installed, a coding agent's plan shows where it is.

- **The percent** is the items completed of all, never the agent's guess; it dims when the plan is
  stale. It shows on the session's row, in a circle among the floating buttons, and on the
  header's Plan button.
- **The Plan pane** (press any of the three; Esc closes): the plan's title, the current step, the
  items, the agent's ETA and when it was updated.
- **Marks**: ✓ completed, ▶ in progress, ○ pending, ! blocked. "partial: some updates were
  missed": an update named an item terminum never saw; the next whole list heals it.
- **"No plan: the agent has not sent one for this session."**: Claude Code makes tasks only for
  longer work, or when asked ("make a plan with tasks"). A session started before the hooks were
  installed reports nothing until restarted; a new agent session in the pane starts with no plan.
- **How agents feed it**: their own plan tool, read by the hooks (Claude Code's tasks and
  TodoWrite, Codex's update_plan where it has one), or `update_session_plan` of `terminum mcp`:
  the current step, an ETA in minutes, and the plan itself for an agent without a plan tool.
- **The reminder**: a working agent that has not updated its plan for 10–30 minutes is asked once,
  through its hook's answer; nothing is typed into the pane.

### When the Plan pane is empty or wrong

| What you see | Why, and what to do |
|---|---|
| No percent at all; "terminum's hooks are not running in this session" | The hooks are not installed (`terminum hooks` says), or Codex does not trust them: open `/hooks` in Codex and trust terminum's, the changed SessionEnd one too. |
| A session started before the install or an upgrade | Restart it: agents read their plugins and `config.toml` only at start. |
| "This session does not run in a terminum pane" | An agent in another terminal, or under sudo, su, a container or a nested ssh, is not seen. |
| Codex lists one terminum tool, `why_no_terminum_tools`, and shows no state or plan; `terminum hooks` says "a shared codex app-server runs" | Plain `codex` attached to a shared app-server a Codex app or editor started, which runs terminum's MCP server and hooks outside your pane. Start Codex with `-c tui.alternate_screen=never` or `--no-daemon`; or run `codex features disable daemon_auto_start`, then `codex app-server daemon stop` (it ends that server's conversations). terminum stops neither. |
| "terminum is not running" | `terminum status`, then `terminum restart`. |
| After `terminum rollback` | Hooks and MCP keep working (they run `~/.terminum/bin/terminum`, which a rollback never moves); a tool the older terminumd does not know fails as a tool error. |
| "MCP server failed" after `terminum uninstall`, or `terminum upgrade --to` a release older than 0.23.0 (an upgrade moves `bin/terminum`, a rollback does not) | That build has no `terminum mcp`. Run `terminum hooks remove` with the newer build first; after the fact, delete terminum's block in `~/.codex/config.toml` and run `terminum hooks install` with the build you kept. |
| A host's agents do not show | Remote hosts, below. |

Plans are stored in `~/.terminum/state/plans.json`, agents' records with your bookmarks in
`web-bookmarks.json` there. To clear the plans: stop terminumd (`launchctl
bootout gui/$(id -u)/io.terminum.terminumd`; Linux: `systemctl --user stop io.terminum.terminumd`),
remove the files, `terminum restart`. Terminals are not touched.

## Your own tmux sessions

```bash
terminum tmux attach work                 # or "Attach tmux sessions" under the list: a session of plain `tmux`
terminum tmux attach work --socket dev    # one of `tmux -L dev`
terminum tmux attach work --host devbox   # one of a host's tmux (the import icon on its group)
terminum tmux                             # what is attached and whether it runs
terminum tmux detach work                 # terminum forgets it; it goes on running
```

- terminum shows the session as its own and never closes a session there.
- Its windows are sized to the page that shows them; with no page showing one, it gets tmux's own
  size back.
- When that whole tmux server is gone (a reboot, a kill), terminum makes the attached sessions
  again with their windows and folders; a session you close while the server runs stays closed.
- A new host whose tmux runs other sessions offers them; later the import icon on its group does.
  Each attached session on a host is one more ssh connection.

## A phone or another computer

terminumd listens only on `127.0.0.1:48080`. Other devices reach it through `tailscale serve`
(HTTPS on your tailnet) and sign in with the same password.

### Over Tailscale

Tailscale gives one name per machine, `https://<mac>.<tailnet>.ts.net`, with one service on each
port. Pick where terminum goes:

| Setup | terminum at | Steps |
| --- | --- | --- |
| Nothing else on the Mac's name | `https://<mac>.<tailnet>.ts.net/` | 1–3 below |
| Another service keeps the Mac's name | `https://terminum.<tailnet>.ts.net/` | 1 below, then the Tailscale Service |

1. Set the password, if you have not yet: `terminum password`.
2. Serve it over HTTPS. This replaces what port 443 served: move another service first (below).

   ```bash
   tailscale serve --bg --https=443 http://127.0.0.1:48080
   ```

3. Open `https://<mac>.<tailnet>.ts.net/` on a device in your tailnet and sign in.

Without a tagged machine for a Service, another service on the Mac's name can move to a port of
its own instead, here one on 8765:

```bash
tailscale serve --https=443 off                            # if it had the address without a port
tailscale serve --bg --https=8443 http://127.0.0.1:8765    # it moves to https://<mac>.<tailnet>.ts.net:8443
tailscale serve status                                     # :443 → 48080 (terminum), :8443 → 8765
```

Notes:

- Give `--https` every time: `tailscale serve --bg 48080` also takes port 443, silently.
- `http://…` does not open: `tailscale serve` cannot redirect port 80. Type `https://`.
- terminum answers to any `*.ts.net` name; another name a proxy forwards goes in `allow_hosts` in
  `~/.terminum/state/web.json`.
- Through a proxy, Finder and typing a path from Files into a terminal are off; the rest works.
- `tailscale funnel` (the public internet) is refused with 403.
- Stop serving: `tailscale serve --https=443 off`.
- tailscaled keeps `serve --bg` and `--service` across a restart of the Mac; terminumd starts at
  login.

#### A name of its own: a Tailscale Service

`https://terminum.<tailnet>.ts.net` is a Tailscale Service: it leaves the Mac's name to another
service, and only a tagged machine can serve it (`tag:personal` below).

1. In the tailnet policy (admin console → Access controls), let your devices reach the service and
   let the Mac host it without a manual approval:

   ```json
   "grants": [{ "src": ["tag:personal"], "dst": ["svc:terminum"], "ip": ["tcp:443"] }],
   "autoApprovers": { "services": { "svc:terminum": ["tag:personal"] } }
   ```

2. In admin console → Services, define `terminum` with the endpoint `tcp:443`.
3. On the Mac:

   ```bash
   tailscale serve --service=svc:terminum --https=443 http://127.0.0.1:48080
   ```

   `autoApprovers` approves only a host that advertises after the service is defined: approve the
   Mac on the service's page when it advertised earlier, or without `autoApprovers`.
4. Open `https://terminum.<tailnet>.ts.net/` and sign in.
5. Give the Mac's name back to the other service if step 2 of the first list took it, and check:

   ```bash
   tailscale serve --bg --https=443 http://127.0.0.1:8765   # the Mac's name → the other service
   tailscale serve status   # <mac>…:443 → 8765; terminum…ts.net (svc:terminum) → 48080
   ```

Remove it: `tailscale serve clear svc:terminum`.

## Remote hosts

```bash
terminum hosts add devbox 'alex@devbox.example -p 2222'   # or "Add host" under the list
terminum hosts                                             # each host, its state, files and agents
terminum hosts enable devbox                               # its files again, after files-remove or a failed setup
terminum hosts agents devbox on|off                        # coding agents' progress from devbox, off or on again
terminum hosts off|on|retry|remove|files-remove devbox
```

- **A host** is its ssh string (options `-p -l -i -J -4 -6 -C`). terminumd holds its tmux session
  `terminum` (and the sessions you attach) over ssh, reconnecting after a drop.
- **Set up by itself** once the host attaches: the Files helper (6 MB in `~/.terminum/bin`), then
  terminum's hooks and MCP server in Claude Code and Codex there; both follow this Mac's terminum
  at each upgrade. Codex asks once to trust the hooks in `/hooks`; restart agent sessions already
  running there.
- **Off**: `hosts files-remove devbox` (Files' menu: "Remove Helper from devbox…") and `hosts
  agents devbox off` (removing the host takes the hooks out too); what you turned off stays off.
- **`hosts agents devbox on`** (the host menu: "Coding agents' progress on devbox…") prints a line
  per agent: installed, not found, needs Codex trust (/hooks), uses devbox's own terminum, refused
  (a terminum entry of another command), or failed and why. A host with no coding agent
  is looked at again when terminum next starts; an agent installed later gets the hooks when
  terminum next starts or upgrades, or at once with this command.
- Only agents in the host's default tmux server are seen.
- **tmux 3.8 or newer**, on the Mac and on hosts: before 3.6 tmux leaks memory with every screen
  read; before 3.8 its server can crash when a session ends, ending every terminal in it. The
  install says when terminum's own server is older. After `brew upgrade tmux` the running server
  keeps its version until restarted (`tmux -L terminum kill-server` at a quiet moment).

| Trouble | Why, and what to do |
|---|---|
| ssh works, the host's tmux does not answer | After three tries the host is left alone: its group says why and has **Reconnect** (`terminum hosts retry devbox`). Usually a new tmux binary with the old server still running: restart that server, or bring the old binary back. |
| "reconnecting" | A lost network shows within about 30 s. |
| A long wait | Copying the helper may take 300 s: `hosts enable` waits up to 6 minutes, `hosts agents … on` up to 10½. |
| "forwarding refused by devbox's sshd" for agents; Files work | Agents' events need a reverse Unix-socket forward, which `AllowStreamLocalForwarding local` refuses: set `AllowStreamLocalForwarding yes` (the default) and, on Alpine, `AllowTcpForwarding yes`. Tried again after 10 minutes, then less often (up to every 2 h 40 min). |
| "Files cannot reach the helper" | `DisableForwarding yes` or `AllowStreamLocalForwarding no` refuse the Files tunnel too. Dropbear forwards no Unix socket: neither Files nor agents work there. |
| The host menu shows ssh's last lines | A ForceCommand or a login file ends the session. |
| ssh refuses to connect | A `RemoteCommand` for the host in `~/.ssh/config`: give terminum a `Host` alias without one. |

## Configure

Everything is in `~/.terminum/` and is read again within a second of a change; a wrong value keeps
its default and is said once in the page.

| File | What |
|---|---|
| `settings.json` | `port`, `tmux.socket`, `tmux.history`, `restore`, `lastLines`, `agentExitDays` (idle agents' exit, also in Settings), `agentNoticesOff` (`codexShared`, `fullScreen`: the notices turned off, also in Settings), `agentOpenTasks` (open tasks by agents a project may hold before create_task is refused: 10000, 1–1000000), `themes` rules (by host and session), `page` defaults (`textSize`, `markdownSize`, `colorScheme`, `widgets`, `folds`, `followAgent`), `midi.soundfont` (the .sf2 MIDI files play with), `dispatcher` (memory limits, `usageCommands`), `hosts` (each with `attach`: sessions of its tmux), `attach` (sessions of your own tmux by socket) |
| `themes/*.json` | colors, ANSI palette, font; `local.json` and `remote.json` ship as defaults |
| `hotkeys.json` | the hot keys panel's rows (`"F5"`, `"C-r"`, `"text:git status"`, `"Paste"`) |
| `custom.css` | your stylesheet, loaded after the page's own |
| `tmux.conf` | the private tmux server's configuration (written once; yours to edit) |
