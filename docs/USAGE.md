# Using terminum

The full guide: commands, the page, your own tmux, other devices, remote hosts and
configuration. The overview and the install commands are in the [README](../README.md); which
fact lives where is in [docs/README.md](README.md).

## What it is

- **terminumd**, one server on `127.0.0.1:48080`: it drives a private tmux server (`tmux -L
  terminum`) and each remote host's tmux, and serves the page and the Files panel
  ([how the parts fit](ARCHITECTURE.md)).
- **The page** (`web/static`): the session list (tmux sessions, windows, panes; coding agents'
  states), the terminal (see Sizing below; keys and the mouse reach a full-screen program), widgets for
  code, Markdown, diagrams and tables in the output, folds, bookmarks, search, hot keys for
  phones, paste and upload of files.
- **The Files panel** (`ui/`): the tree of the shown pane's folder, a viewer and editor with
  highlighting and rendered Markdown (its ```mermaid fences drawn as diagrams, "Source" shows
  their text), JSON and YAML as a tree, CSV and TSV as a table (Source shows the text),
  pictures, PDFs in the browser's own viewer, videos and sounds in its player (AU and AIFF
  decoded where the browser does not play them; MIDI rendered by terminumd with the General
  MIDI SoundFont that comes with terminum, GeneralUser GS, or yours: `~/.terminum/soundfont.sf2`
  or `midi.soundfont` in settings.json), create, rename, Trash, live refresh; Esc in View hides
  it and goes back to the terminal, Esc in the tree hides it and leaves the keys there (Enter,
  Esc, ↓, Enter… looks at one file after another); on a wide window it is docked beside the
  terminal, open or closed for each session, its width set by dragging its border with the
  terminal; on a phone it is a view in its place.
- **Sizing**: the pane follows the page you use: the one that opens it, comes to the front or
  types there sizes it to its view (45×14 to 240×70 cells); in a window of several panes
  (Claude Code's teammates split it) the pane you look at is zoomed to fill it, the layout kept, and
  only typing makes the window smaller; your own tmux, or a tmux client with that window on its
  screen, keeps its layout unzoomed. Text is never scaled: a pane narrower or wider than
  your view re-flows to it, lines tmux or the program wrapped joined again (a full-screen program
  such as mc is drawn as its grid, scaled when wider). Switching sessions shows the new one at once
  at your text size.
- **Sessions outlive everything**: terminals live in tmux; terminumd runs as a launchd agent
  and comes back after a crash; after a reboot, or when a tmux server was killed, the sessions,
  windows, splits and folders are restored: terminum's own, each host's and the sessions of your
  own tmux you attached (`"restore": true`). Each session's last 300 lines are kept as well
  (`"lastLines"`, up to 2000, 0 for none): after the restart, **Show Last 300 Lines** in its row's
  menu shows how it ended (the terminal's lines; a full-screen program's own screen is not kept).
  They are kept on disk in `~/.terminum/state/lastlines/` (private, out of Time Machine) until the
  pane closes; `"lastLines": 0` deletes them.

## Install

The commands and what they need are in the [README](../README.md#install). This is what they do.

The installer checks the release's signature, installs into `~/.terminum/` (`builds/<build>`, `current`, the command
`~/.terminum/bin/terminum`, which is terminumd itself, and `daemon/terminumd`, the copy launchd
runs), loads the launchd agent `io.terminum.terminumd`, which holds the page's port from then on ([SECURITY.md](../SECURITY.md)), and says how to set the password.

During the install macOS asks on the Mac's screen, once per folder, whether programs in
terminum's terminals may read Desktop, Documents, Downloads and each cloud folder
(`~/Library/CloudStorage`: Dropbox, Google Drive, OneDrive…): terminumd opens each one itself, so the prompts come while you are
at the Mac rather than when an agent first reads a folder while you are away. Allow covers
every shell and agent in terminum's tmux and stays across upgrades, since launchd always runs
`daemon/terminumd` (signed as `io.terminum.terminumd`). The install waits 5 s for the answers
and says which folders macOS still asks about; `terminum folders` asks again and waits. A
refused folder can be allowed again in System Settings › Privacy & Security › Files and Folders.

iCloud Drive (`~/Library/Mobile Documents`) stays closed to programs in terminum's terminals, as
to any program without Full Disk Access: macOS refuses it ("Operation not permitted") and never
asks, so terminum does not ask for it, and does not need Full Disk Access for anything else.
Keep work in Documents or Desktop (also when "Desktop & Documents Folders" sync to iCloud: their
own Allow covers them) or in a cloud folder of `~/Library/CloudStorage`.

macOS judges a terminal's program by the program that started terminum's tmux server, and that
server outlives upgrades. `terminum status`, the install and `terminum folders` say when it was
started by another program (a build from before this one, a build run by hand, `tmux -L
terminum` typed in Terminal): restart it once at a quiet moment, by a reboot or `tmux -L
terminum kill-server`, which ends the programs in it; terminum makes the sessions again with
their windows, splits, folders and projects, and the programs are started again by
hand (`claude --continue`, `codex resume --last`).

On a Linux server terminumd is the systemd user service `io.terminum.terminumd`
(`~/.config/systemd/user/`), started at once and at every login; a restart, an upgrade or an
uninstall ends terminumd alone, never the terminals. Without linger the user's services end at
logout, terminals too: `terminum status` says so. The [README](../README.md#on-a-ubuntudebian-server)
has the command that keeps them and says how to reach its page. Finder and default apps are a Mac's only.

From a checkout: `make install` (Rust, Node 26 and the Xcode Command Line Tools needed;
[CONTRIBUTING](../CONTRIBUTING.md#worktrees) says when it refuses).

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
terminum hooks install        # Claude Code's and Codex's own hooks: exact states, prompts, reminders
terminum import               # workspaces, bookmarks and hosts of iterm-enhancer
terminum help                 # every command
```

Every browser, on the Mac or another device, signs in with the password; until one is set the
page says how; how the sign-in is kept is in [SECURITY.md](../SECURITY.md). `tmux -L terminum attach` from any terminal shows the same sessions.

- **Under the list**: New session, Bookmarks / Reminders, Add host and Attach tmux sessions
  fold into one row of their icons with ▸ on a computer; click it and they unfold (▾) until you
  click it again; the page remembers which.
- **Quick shell**: the button in the header (or "Quick shell below" / "on the left" in the menu)
  opens a shell of its own under the shown pane, in its folder, for quick commands; drag its edge
  to size it, ⇄ moves it, × closes it. It is a tmux window marked with its pane, listed under it.
- **Coding agents**: with `terminum hooks install` Claude Code (a plugin of terminum's, added
  with `claude plugin`) and Codex (entries in `~/.codex/hooks.json`; trust them once in Codex's
  `/hooks`) tell terminum when they work, wait for an approval or an answer, stop, fail or run
  subagents; without them terminum reads their screens. `terminum hooks` says what is set up,
  `terminum hooks remove` takes terminum's hooks out. An install or upgrade of terminum refreshes them (and the
  MCP server) where you installed them, never elsewhere; `terminum uninstall` takes them out.
- **MCP server**: `terminum hooks install` also adds `terminum mcp`, terminum's MCP server, so
  the agent can keep the session's plan, progress and notes for you: in Claude
  Code through the same plugin (its `.mcp.json`), in Codex as one block in
  `~/.codex/config.toml` between `# >>> terminum: MCP server …` and `# <<< terminum` (the file
  backed up first; nothing else in it changes, lines of others inside the markers are kept, a
  symlinked file is written where it points). An `[mcp_servers.terminum]` of your own there, or a
  file changed while the install ran, is refused, not overwritten. Restart running Codex sessions
  to give them the server. `terminum hooks remove` takes the block out. The server starts in every Claude Code and Codex
  session on the Mac, but outside a terminum pane it offers no tools and no instructions, so it
  costs those sessions nothing.
- **Sent history**: the speech bubble under the split shows what you sent in the pane's folder
  (its prompts and commands, kept in the message history below), newest first: ↑ older, ↓ newer;
  Jump to message shows where it was sent, Insert into chat types it again (without Enter). On a
  wide window small bars on the terminal's right edge mark where your messages (white), the
  agent's answers ended (pale blue), bookmarks (orange) and reminders (pink) stand in the
  history: press one to go there. Settings' Side marks turns them off.
- **Input log**: everything terminum writes into a pane, a row each in the table `inputs` of
  `~/.terminum/state/kanban.sqlite`: what you type (a row per Enter), paste or the Files panel
  types, from which browser and address, and what Dispatcher pastes and launches, with its run,
  tasks and agent; kept 180 days. What is typed at a password prompt, or a line you begin with a
  space, keeps only why it is not kept. Read it with `sqlite3`; the columns and an example query
  are in `docs/specs/terminum.md` §9 Data formats (AC-147). What an agent was actually given is
  its own record, in the sent history above.
- **Message history** (`/history/`, Messages among the sections in the page's top
  left corner): every prompt and command sent, yours and Dispatcher's (tagged), kept in the
  table `messages` beside the input log. Type words to find them, the best matches first (each word
  also as the start of a longer one, `"a phrase"`, `-word` to leave out), or browse newest first by
  day; narrow by days (the strip of a column a day), project, sender and agents or shells. A row
  opens its session at that message, shows what was sent around it, or copies it. Every choice is in
  the address, so Back and links work.
- **Scheduled runs** (`/scheduled/`, Scheduled beside History in the Message history): every firing of
  your schedules, newest first: when, the schedule, its task (a press opens it on the board), its
  agent and status, when it went IN PROGRESS and ended; a row opens to the prompt that went in and
  the result that came out, or says why nothing ran (skipped, missed, did not start). Schedule picks one.
- **Dashboard** (`/dashboard/`): four tabs over one period (Day, 2 weeks, Month, Quarter, Year) and
  one project or all. **You**: how much you work here, how engaged and how loaded, never counting
  Dispatcher: active time (what you send, type, press, scroll and do on the board; a pause over
  5 minutes ends a stretch, or over 15 with a page in front through it), active days, engagement
  (the share of active minutes you acted or read in; the rest you watched), your longest stretch,
  switches between sessions an hour, board actions, how long tasks waited for you, off hours, the
  phone's share, each against the span before; active time by project, your working day, the
  week's busy hours, sessions at once. The pages count what you do by the minute (how many presses,
  keys and scrolls and where, never which key or what text) into the table `user_interaction` of
  `~/.terminum/state/kanban.sqlite`, kept 180 days; engagement starts on the day they first did.
  **Tasks**: who does the work on tasks: the operations you decided (a task made, edited, moved,
  commented on, run, archived) beside the agents' and Dispatcher's, over time, by operation, and
  each task's split. **System**: the sessions Dispatcher started
  and how their tasks ended, failures by reason, the agents' load, schedules' firings. **Costs**:
  dollars at API list prices and tokens of Dispatcher's sessions by agent, project and model, and
  each agent's use against its limits. A column of Active time opens its messages in the history.
- **The agent's folder**: when Claude Code works in another folder than where it started (a
  worktree), its row in the list, the header's folder and the Files tree follow it, and paths
  it prints open from there. Settings' Follow the agent's folder turns that off for this browser (`page.followAgent`
  in settings.json for every browser that did not choose).
- **Tip of the day**: on a computer the foot of the session list shows a tip, another each day;
  Next shows the next one, × closes it until tomorrow (Settings' Show today's tip brings it back), Turn off (or Settings' Tip of the day) turns
  tips off for this browser.
- **Ask**: select text in the terminal and press the blue speech bubble with a question mark (under
  the selection's orange [+]); type a question and Add to Chat puts the selection as a Markdown
  quote (`> Quote #0007`, then `> ` and each of its lines), a blank line, your question and a blank
  line into the input, without Enter; the view stays where you were reading. In a coding agent's
  input it is typed, so the agent shows it whole, unless the agent's box would then be taller
  than the pane: then it is pasted and the agent folds it.
  The number counts up per session. The text you asked about keeps a light blue wavy line down its left in
  the terminal, as a reminder's quote, on every page of that session.
- **Full-screen programs with the mouse** (Claude Code's fullscreen mode, vim with `set mouse=a`,
  mc): a drag, a double or a triple click is the program's, and it selects in its own way. When
  it copies what it selected to the terminal's clipboard (OSC 52: Claude Code does on the
  release, vim with an OSC 52 yank does on `y`), the page shows Copy, [+] and Ask by the
  selection with its text, as for a selection of yours; they go at the next click or key. Hold
  **Shift** while dragging to select in the browser instead (Safari, Chrome, Firefox): nothing
  reaches the program.
- **Reminders**: pick Reminder in a bookmark's dialog; the pale fuchsia bell comes up when its
  session's agent ends a turn, its count filled, and the terminal's top says so for 4 seconds
  ("api: 16 reminders, 3 new"; another session's when you open it next). It opens the same window as the bookmarks' button,
  with the open reminders only, those up first (Going through them, below), and Quote in chat. Quote in chat, on a reminder or a bookmark, and Insert into chat on a sent message
  paste it (a bookmark's note and its text) into the session's input, without Enter.
  Bookmarks and reminders show only in the session they were made in, in a project or not.
  Bookmarks / Reminders below the sessions lists them all, each with its project or session
  (Open all); once their session closes they are only there, "orphaned".
- **Limits**: you keep up to 300 bookmarks and 100 open reminders (a save past them says so). Coding
  agents' records have their own: 50 bookmarks and 20 open reminders a session; past
  them their oldest go, and a banner over the terminal says so at the end of that agent's turn, with
  a link to review them. A resolved reminder is deleted 30 days after you resolve it.
- **Going through them**: the bookmarks' button (its count: bookmarks and open reminders) opens a
  window with the session's bookmarks and reminders on top, top to bottom ("1 / 52" at each one's
  right), each its note and its
  text's start on one line, and the chosen one's content under them; drag the line between them to
  share the height (a double click: half). Press one, or ↑ ↓ on a computer, to show it; Show in
  terminal (Enter) goes there; Resolve (or Delete) settles one, a reminder resolved and a bookmark
  deleted, with Undo for 5 seconds; Esc closes the window. The bar over the terminal has the same
  Resolve.
- **Tasks** (Kanban among the sections, `/kanban/`; ⌘Enter in a text keeps it, a file dropped on the board starts a task; a card carries its session's row, a press opens the session, and says what became of its agent; the project key in the head chooses the project, a pencil on its row or its menu's New project… opens its window; the session page's list stands left of the columns, as wide as there, and its agents over them, each with two stripes of its subscription's use (below, "Subscription limits on the board"); a press on an agent opens its window: its stripes with their reset times, its tokens of 7 and 30 days, and each live session with its tasks as links, those it works on and those finished while the session stays open; drop a task on a free agent, or ⌘+press several and drop them, write a short message and press Run, or press Run in the new-task window: a task moves only by running; Dispatcher opens the window, types the task once the agent's box is empty, moves the card as the session lives and closes the session 5 minutes after it ended; a task's Budget gives the agent that many minutes, told at the start and at the end, and it is ended at the limit; a card shows its session's CPU, memory, running time and tokens, and a project's memory limits warn and end it (4 and 8 GiB unless set, `dispatcher` in settings.json or Memory limits… in its project's window); WAIT FOR HUMAN and FAILED say so in a line on both pages and in a count on Kanban; Codex needs terminum's hooks trusted in its /hooks): tasks in TODO, IN PROGRESS, WAIT FOR HUMAN,
  FAILED and DONE, with a severity, an owner, a project, Markdown comments and files; + Task
  takes files dropped or pasted in, also from a session's menu or a terminal's selection (the
  bookmark dialog's New task from selection). A task can be linked to a session: its row shows the
  task, and when you move the task its agent hears of it at its next tool call. Agents use
  `list_tasks`, `get_task`, `create_task`, `take_task`, `move_task`, `ask_human`, `add_comment`, `attach_file`,
  `get_file` and `finish_task`: on every task of their project, whatever session made or holds it, but one a human's RUN holds in another session, until that session is closed;
  an agent Dispatcher starts runs its command as you wrote it, so write the permissions it needs there (`claude --dangerously-skip-permissions`); when it
  needs you it asks with `ask_human`: its questions, with its recommended answers, stand on the task
  in WAIT FOR HUMAN and its session closes; answer in comments and press Run, and the same
  conversation goes on (dropped on another agent of its kind, there). Run on a busy agent offers
  its queue: Add to queue, and the task starts by itself when a slot frees; the agent's card shows a pale dot for
  each queued task, and its "queued" key opens the queue, where you drag tasks (on a phone: rest a finger on one
  first) or use the arrows to change their order and the cross to take one out; Stop waiting in the task's window
  does the same; Auto-Dispatch, a switch and a cog in the board's bar (off by default), starts a
  project's TODO tasks by itself: the cog's window holds the agent that picks, a default agent to run them, the
  severities it takes (as made: P0 and P1), the minutes a task stands in TODO before it is taken (10) and the
  tasks it starts in 24 hours at most (10); the switch
  then turns it on or off in one press (with several projects shown a press turns it off for them, and turning on is
  each project's own in the window); "How tasks are picked" chooses between an agent's scoring and the board's order (P0 first, then the cards'
  order, started by terminum itself with no agent); with an agent, when slots are free it starts one more session of the picking agent, which
  scores the first such tasks by RICE and starts the best, each on the agent named in the task's window (Agent),
  else the default; what a round leaves is passed over for a day and the next takes the tasks below; a task it
  started is started again only after you move or edit it; a failed round or two failed tasks turn it off, and its
  switch says why;
  `create_task`'s text follows the project's template (its window: Template for agents' tasks).
  With `update_task` an agent grooms its project's board: severity and parent of any task (a
  human's with a reason, kept as a comment), title and text of a task agents wrote, and such a task
  archived with its reason; your own words and tasks are never reworded or archived by an agent.
  An agent's tool list carries only `search_tools`, `get_tool_info`, `execute_tool` and
  `update_session_plan`; it finds the task tools by the action ("change a task's priority") and
  runs them through `execute_tool`, as terminum's instructions and Dispatcher's prompts write
  each call (`execute_tool("finish_task", {id, result})`). An agent that Dispatcher starts
  without permission prompts needs `execute_tool` allowed (Claude Code:
  `--allowedTools mcp__plugin_terminum_terminum__execute_tool`), and that allows every task tool.
  A RUN's prompt also asks the agent to close the task out: what it noticed goes to tasks, it
  reviews the tasks it made, and its result comes in fixed sections (Outcome, Done, Checked, Not
  done, Needs a human, New tasks, Tasks changed, Issues and discoveries, Further
  recommendations), written in the project's language: its window's "Language of agents'
  writing on the board" (empty: English), used for its tasks and comments on the board too.
  TODO and DONE show each card as one line, its ID, severity and title: a press shows one whole
  (and folds the one before), a press on it then opens its window, Esc folds it. Agents stand a
  column wide over the columns, four a row.
  **Scheduled tasks**: the line Scheduled N between the agents and the columns opens (and stays
  open in that browser) to the schedules of the projects shown; + Schedule opens a schedule's
  window: title, text, project, severity, the agent that runs it, When (every 1–45 minutes, every
  N hours, every day, weekdays, chosen days of the week, a day of the month, once, or five cron
  fields, in the Mac's local time, the next three runs said under it),
  a time budget (required: no one watches the run) and Paused; What the agent gets shows the
  prompt and the agent's command. Each schedule keeps one slot of its agent while not paused. When
  it is due, Dispatcher makes a task from it as it is then, IN PROGRESS and named S_TERM-8 (S_
  before its project's next ID), in the project's folder, and runs it on that agent, telling it it
  is a scheduled run and to give its result as Outcome, Summary, Details and For the human, or ask
  you with `ask_human`; the task then waits for you, is done or fails as any task, never standing
  in TODO, its card saying SCH-1. A firing while a task of it is still IN PROGRESS is skipped and
  counted on the schedule's card; a run missed while the Mac slept is not run on wake: the next
  comes by the cadence; one that cannot start (no folder, no hooks, agent gone) fails its task,
  the schedule's card says why, and the next firing tries again. Run now on a card makes its task
  and starts it at once, whatever its cadence, which stays. You make schedules, and so does a
  project's manager (below); a worker's agent has no tool for them.
  **Managers and workers.** The session + Session starts for a project is that project's
  manager: beside every agent's tools, its `terminum mcp` finds tools to run the project's tasks
  on the pool's agents (`run_task`: a worker's session does each, by Dispatcher's rules), follow
  them (`list_runs`, `get_task`), answer their questions and run them again (`run_task` with a
  comment), stop them, add, change and remove the pool's agents and the project's schedules, run
  a schedule now, list, read, type into and close the project's other sessions, and read its
  numbers. Every other session is a worker and neither finds nor runs these: one Dispatcher
  started for a task, one you opened by hand. What a manager makes is in force at once and
  carries its name ("Opus · manager"); what it tells a worker is said to be the manager's,
  never yours. From a shell: `terminum tools` lists what the pane's session may run, `terminum
  tool <name> '<json>'` runs one. A program outside terminum calls the same with a token in
  `TERMINUM_TOKEN`: `terminum tokens add <name> --role manager|worker --project <project>
  [--days N]`, run in a terminal outside terminum, prints it once; `terminum tokens` lists them,
  `terminum tokens remove <name>` ends one. Roles guide agents; they are no wall against a
  program of yours (SECURITY.md).
  A task's ID is its project's letters and number, TERM-7: the letters come from the project's
  folder and can be changed in its window; an old ID (T-12) and the 8 characters its window shows
  beside its dates still find it. A task's window shows the task, then its subtasks (New subtask,
  Add existing, Make it a subtask, × to take one out), then its comments and history, newest first
  (press Newest first to turn them); a card shows its subtasks as boxes in their status's color.
  The round button with a blue number under the sent history's counts the tasks made in the shown
  session, and opens them in the same window; Convert to task on a bookmark or reminder (its pane,
  its bar, the list below the sessions) makes one in TODO without leaving the session.
  What agents kept for you before tasks stays with your bookmarks and reminders.
- **The view's menu** has 20 color schemes (one for every browser, kept in settings.json),
  the Markdown text size and the history lines of new terminals. Option or ⌘ with ↑/↓ pages.
- **Keys of an IDE**: ⌘[ and ⌘] go back and forward through the last 20 sessions you showed,
  ⌘F searches the history, ⌘B opens or closes Files. In a coding agent's input, Backspace or
  Delete over text you selected deletes it, as in a text field.
- **The header** names the shown pane under its title: its folder (the copy button copies its full
  path), tmux's `session:window`, then what runs there.
- **The list**: a group's buttons (copy, import, [+], ⋯) show while the pointer is over its header,
  in the place of its count; each row's Clone (shown while the pointer is over the row; on a phone always)
  opens a window beside it in its folder; a row hatched with pale lines is unread (its agent finished
  a turn nobody saw); a right click, or a finger
  rested on a row and lifted, opens its menu (Clone, Add to Project, Mark as Unread or Read, Diagnostics… (what terminumd and tmux see of the pane, to copy into a bug report: check it first, it holds the pane's folder, host and title), Split Below, Right, Left,
  moves, Move to Session, Close); the header's ⋯ holds the settings. Drag a row, a
  group's or a project's header to reorder: a dashed outline shows where it lands (a finger rests
  a moment first; moving at once scrolls). Projects are named groups
  of sessions in a color of their own, with [+] for a new window in the project's folder, the same
  on every browser; a right click on a project's header, or its ⋯, opens its menu (New Window,
  Rename, moves in the list, past tmux groups too, Delete Project, which moves its sessions and tasks to Default, Close Project, which closes its sessions and keeps it; both ask first); a tmux session's group
  has the same kind of menu (New Window, a window in that session; moves, Close Group).
  ⌘Z sends the program's undo (Claude Code, zsh, bash, fish); ⌘⇧Z redoes where it can (fish);
  ⌘X cuts the text selected in a coding agent's input box.
- **On a phone**: the floating buttons but the keyboard's two fold into ⟨…⟩ at the top right; a tap
  shows them for 3 s. They hide while your finger scrolls. A selection shows the page's Copy, [+]
  and Ask there instead of iOS's menu. On the input: a finger held still half a second and lifted
  opens Cut, Clear, Paste, Copy, Undo, Redo (what applies); a quick swipe right is →, left is Undo;
  two taps select a word. Dialogs and menus open centered.
- **A screenshot** pasted into the terminal (⌘V) is saved in `~/.terminum/tmp` (on a host, in the
  host's `~/.terminum/tmp`) and its path is pasted: Claude Code takes it as an image.
- **Files and folders dropped from Finder** on a session of this Mac, in this Mac's browser, put
  their own paths at the input, quoted, one per line in a coding agent's box, without Enter;
  nothing is uploaded, at any size (terminumd reads the drag's paths from macOS's drag pasteboard).
- **A file dropped** anywhere else (a host's session, a phone, a drag from a browser or Mail), or
  picked with Upload, is uploaded as a screenshot is and its path pasted, up to 100 MB; give a
  larger file by its path (in Finder ⌥⌘C copies it).
- **Files and folders dragged from Files** onto the terminal put their paths at the input, quoted,
  without Enter; nothing is uploaded. Drag a selected row to put every selected row's path.
- **Files dragged from Finder onto the Files tree** upload into the folder under the pointer (a
  file's row: its folder; below the rows: the root), on this Mac or the pane's host; the folder
  shows while you drag. A name already there asks Keep Both or Skip: nothing is written over.

## Plans, progress and notes

With terminum's hooks installed (`terminum hooks install`, above), a coding agent's plan shows
where it is:

- **Where**: the session's row shows "43%" right of its title over a thin track (hover: "3 of 7 ·
  updated 2 min ago"); the shown session's progress is a circle "43%" with a ring, first in the floating buttons'
  column (on a phone, tap ⟨…⟩ to see it with the others); the header has a Plan button right of ✎ with the percent ("43%"; the icon alone at 0%)
  for a session with a coding agent. The percent is
  the items completed of all, never the agent's own guess; it dims when the plan is stale.
- **The Plan pane**: press the percent on a row (it shows that session), the progress circle
  or the header's Plan button; by keyboard, Tab to the Plan button and press Enter or Space; Esc or × closes
  it. It shows the plan's title, the current step, the items, the agent's ETA ("~17:40 (agent's
  estimate, given 16:55)") and when it was updated, then how many records the agent kept for you
  (a press shows them: the bookmarks' list). The plan's text is shown as plain text.
- **"No plan: the agent has not sent one for this session."**: the agent made no plan yet. Claude
  Code makes tasks only for longer, multi-step work (or when asked: "make a plan with tasks");
  terminum's MCP tools report one too (update_session_plan). A session started before terminum's
  hooks were installed reports nothing until it is restarted.
- **Marks**: ✓ completed, ▶ in progress, ○ pending, ! blocked. "partial: some updates were
  missed" means an update named an item terminum never saw; the next whole list heals it.
- **How agents feed it**: their own plan tool, read from their hooks (Claude Code's tasks and
  TodoWrite, Codex's update_plan where it has one), and terminum's MCP server `terminum mcp`
  (update_session_plan, get_session_plan, and the task tools below): the
  current step, an ETA in minutes, the plan itself for an agent without a plan tool (Codex
  0.164 has none). A new agent session in the pane starts with no plan.
- **The reminder**: when a working agent has not updated its plan for 10–30 minutes (by the
  turn's length), terminum asks it once through the hook's answer (Claude Code: a synchronous
  PostToolBatch hook; Codex: its PostToolUse); nothing is typed into the pane, and an idle,
  waiting or finished plan is never reminded.

### Subscription limits on the board

Each agent over the board's columns shows two stripes: how much of its subscription's five-hour
window and week is used, as the last session of its kind said it, one reading for every agent of
that kind. "not known": nothing said it yet; "reset": the window started anew
since; "none reported": the account has no such window (a Codex plan with a week alone); faded:
read over an hour ago. 90% or more is drawn in the danger color. Its window says when each resets.

- **Its usage command** reads them for each kind in the pool, without a model call (no tokens spent):
  when terminumd starts, every 10 minutes while a run of that kind works on a task, as soon as one of
  its tasks ends, and hourly otherwise. The last reading is kept across restarts. By default Claude
  Code's is `claude -p /usage --output-format json --no-session-persistence --strict-mcp-config` and
  Codex's is `codex app-server` (terminum asks it `account/rateLimits/read`). Change or turn one off
  (`""`) in `~/.terminum/settings.json`, for example when the program is not on terminumd's PATH:

  ```json
  "dispatcher": { "usageCommands": { "claude": "/Users/alex/.local/bin/claude -p /usage --output-format json", "codex": "" } }
  ```

  When one fails, the agent's window says why, and the stripes keep the last reading.
- **Codex** also says its limits in its session log: terminum's hook reads them at each turn's end.
- **Claude Code** also says them to its status line, between usage commands. terminum never edits Claude Code's settings,
  so add it yourself in `~/.claude/settings.json`; your own status line command, if you have one,
  goes after it as one argument and prints as before, with the same input:

  ```json
  "statusLine": { "type": "command", "command": "terminum statusline 'your own status line command'" }
  ```

  Alone, `terminum statusline` prints "5h 34% · week 21%". It sends terminumd the percents, window
  lengths and reset times only, and only from a pane terminum lists. An API key, Bedrock or Vertex
  has no plan limits: nothing is sent.

### When the Plan pane is empty or wrong

- **No percent at all**: terminum's hooks are not installed (`terminum hooks` says), or Codex
  does not trust them yet (open `/hooks` in Codex and trust terminum's, the changed SessionEnd
  one too); the pane then says "terminum's hooks are not running in this session".
- **The session started before the install or an upgrade**: restart it; agents read their
  plugins and `config.toml` only when they start.
- **Not a terminum pane**: an agent in another terminal, or under sudo, su, a container or a
  nested ssh, is not seen; its MCP tools answer that the session does not run in a terminum pane.
- **terminumd is down**: the tools answer "terminum is not running"; `terminum status`, then
  `terminum restart`.
- **After `terminum rollback`** the hooks and the MCP server keep working: they run
  `~/.terminum/bin/terminum`, which a rollback never moves (it stays the newest build's); an MCP
  tool the older terminumd does not know fails as a tool error, never the agent.
- **"MCP server failed" in Claude Code or Codex** after `terminum uninstall` or `terminum
  upgrade --to` a release older than 0.23.0 (an upgrade does move `~/.terminum/bin/terminum`): the
  older build has no `terminum mcp`. Run `terminum hooks remove` with the newer build first;
  after the fact, delete Codex's block (MCP server, above) in `~/.codex/config.toml` and run `terminum hooks install` with the build you kept.
- **A host's agents do not show**: see Remote hosts below.
- **Stored where**: plans in `~/.terminum/state/plans.json`, the agent's records with your
  bookmarks in `~/.terminum/state/web-bookmarks.json`. To clear the plans, stop terminumd first (`launchctl bootout gui/$(id -u)/io.terminum.terminumd`; on Linux
  `systemctl --user stop io.terminum.terminumd`), remove the files, then `terminum restart`;
  terminals live in tmux and are not touched.

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
| Another service keeps the Mac's name | `https://terminum.<tailnet>.ts.net/` | 1 below, then the Tailscale Service |

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
of its own instead, here one on 8765:

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
  of the Mac. The program behind each address must be running too: terminumd starts at login.

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
   tailscale serve --bg --https=443 http://127.0.0.1:8765   # the Mac's name → the other service
   tailscale serve status   # <mac>…:443 → 8765; terminum…ts.net (svc:terminum) → 48080
   ```

To remove it: `tailscale serve clear svc:terminum`.

## Remote hosts

```bash
terminum hosts add devbox 'alex@devbox.example -p 2222'   # or "Add host" under the list
terminum hosts                                             # each host, its state, files and agents
terminum hosts enable devbox                               # its files again, after files-remove or a failed setup
terminum hosts agents devbox on|off                        # coding agents' progress from devbox, off or on again
terminum hosts off|on|retry|remove|files-remove devbox
```

A host whose ssh works but whose tmux does not answer stops being tried after three tries: its group says why and has a **Reconnect** button (`terminum hosts retry devbox`). The usual cause is a new tmux binary with the old server still running: restart that server, or bring the old binary back. Use tmux 3.6 or newer on a host: older ones leak memory with every screen terminum reads.

Once a host attaches, terminum sets it up by itself, with no question: its helper for Files
(6 MB in `~/.terminum/bin`), then terminum's hooks and MCP server in Claude Code and Codex there.
Both follow this Mac's terminum at each upgrade. `hosts files-remove devbox` (Files' menu: "Remove
Helper from devbox…", or Not now) and `hosts agents devbox off` turn one off for that host; what
you turned off stays off until you turn it on. A host with no coding agent is looked at again
when terminum next starts; a coding agent installed on a host after its agents were turned on gets
terminum's hooks and MCP server when terminum next starts or upgrades (or at once with `hosts
agents devbox on`).

`hosts agents devbox on` (or the host menu's "Coding agents' progress on devbox…") puts terminum's
hooks and MCP server into Claude Code and Codex on devbox (`~/.claude`, `~/.codex`) and prints one
line per agent: installed, not found, needs Codex trust (/hooks), uses devbox's own terminum,
refused (a terminum entry of another command) or failed and why. Codex asks once to trust them in
`/hooks`; restart agent sessions already running there. devbox's rows and Plan pane then show its
agents as local ones (Plans, progress and notes, above). `off`, or removing the host, takes them
out again.

The events ride on the Files helper's ssh as a reverse Unix-socket forward, so devbox's sshd must
allow it: `AllowStreamLocalForwarding yes` (the default) and, on Alpine, `AllowTcpForwarding yes`
(its stock value is `no`). When only this reverse forward is refused (`AllowStreamLocalForwarding
local`), the host menu and `terminum hosts` say "forwarding refused by devbox's sshd", Files keep
working, and the forward is tried again after 10 minutes, then less and less often (up to every 2 h 40 min) while it stays refused. `DisableForwarding yes` or
`AllowStreamLocalForwarding no` refuse the Files tunnel too (it forwards a socket the other way);
Dropbear forwards no Unix socket in either direction, so neither Files nor agents work there:
Files say "forwarding refused by devbox's sshd (…)" and "Files cannot reach the helper". Only agents in devbox's default tmux server are
seen; an account with a ForceCommand or a login file that ends the session shows the ssh's last
lines in the host menu instead.

A host is its ssh string (options `-p -l -i -J -4 -6 -C`); terminumd holds its tmux session
`terminum` over ssh (and the host's other sessions you attach, above), reconnects after a drop, and browses its files through a helper it copies
only after you enable it. A lost network shows as "reconnecting" within about 30 s (ssh's keepalive:
15 s × 2); copying the helper may take up to 300 s, so `terminum hosts enable` waits up to 6 minutes
and `hosts agents … on` up to 10½. The remote command is always on ssh's command line: a
`RemoteCommand` for the host in your `~/.ssh/config` makes ssh refuse to connect: give terminum a
`Host` alias without one.

## Configure

Everything is in `~/.terminum/` and is read again within a second of a change:

| File | What |
|---|---|
| `settings.json` | `port`, `tmux.socket`, `tmux.history`, `restore`, `lastLines`, `agentExitDays` (idle agents' exit, also in Settings), `themes` rules (by host and session), `page` defaults (`textSize`, `markdownSize`, `colorScheme`, `widgets`, `folds`, `followAgent`), `midi.soundfont` (the .sf2 MIDI files play with), `hosts` (each with `attach`: sessions of its tmux), `attach` (sessions of your own tmux by socket) |
| `themes/*.json` | colors, ANSI palette, font; `local.json` and `remote.json` ship as defaults |
| `hotkeys.json` | the hot keys panel's rows (`"F5"`, `"C-r"`, `"text:git status"`, `"Paste"`) |
| `custom.css` | your stylesheet, loaded after the page's own |
| `tmux.conf` | the private tmux server's configuration (written once; yours to edit) |

A wrong value keeps its default and is said once in the page.
