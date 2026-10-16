# Parity map: iterm-enhancer 0.42 → terminum

Every acceptance case of iterm-enhancer's `docs/specs/iterm-enhancer.md` and where it lives in terminum's specs (`terminum.md` AC-01…20, `terminum-terminal.md` AC-31…40, `terminum-files.md` AC-51…73, `terminum-remote.md` AC-81…87), or why it is dropped. IDs are written with a space here so that the spec lint does not read them as this file's own.

| iterm-enhancer | terminum | Why |
|---|---|---|
| AC 01 tree follows cwd | AC 51 | `#{pane_current_path}`, one mode instead of three |
| AC 02, 03, 04, 05, 08–13, 15–19, 21–24, 29, 31, 32 (files) | AC 52…AC 69 | kept as they are |
| AC 06 install, AC 33, 34, 35, 40 | AC 08, AC 16 | launchd instead of AutoLaunch; versioned builds, rollback, signed releases kept |
| AC 07 token, AC 28 outdated link | AC 09 | a one-time code gives the page a sign-in it keeps per origin and sends as a header (no token in the URL, no cookie); the proof and the checks kept; an expired sign-in says so |
| AC 14 theme from the profile | AC 12 | theme files of our own; the defaults are the owner's profiles |
| AC 20 (dropped) | — | — |
| AC 25, 27, 36, 41 panel per iTerm2 window | AC 70 | a browser tab shows a pane and its Files panel; no window binding problem; 100 pages (AC 20) |
| AC 26 viewer window | AC 70 | the View window of the page, or a second browser tab at /view |
| AC 30 bridge lifecycle | AC 14 | no bridge; launchd restarts terminumd; tmux keeps the terminals |
| AC 37, 38, 42, 39 remote files, hosts, helpers, builds | AC 82, 83, 84, 86 | kept; a host is the ssh destination the user gave terminum, not one sniffed from a process |
| AC 43 iTerm2 settings, AC 45 Open Quickly | dropped | no iTerm2; the list's filter and ⌘K (AC 01) find a session |
| AC 44 live inventory | AC 71 | tmux's own events and `list-panes` |
| AC 46, 47, 48, 49, 50 checkpoints and reconstruction | AC 15 | tmux holds the sessions; only a reboot needs a restore, from a layout file |
| AC 51 update notice | AC 17 | kept |
| AC 52, 53 web access, the terminal, files in the web app | AC 02, 03, 04, 07, 10, 13, 18 and `terminum-terminal.md` | split by feature; "Resize iTerm" becomes sizing (AC 13); "Show in iTerm" dropped (every client sees every pane); merge windows becomes "Move window to session" (AC 07) |
| AC 54 dropped -CC integration | dropped | terminumd is the control client; a lost ssh is AC 81 |
| AC 55 latency trace | AC 38 | kept, stages renamed |
| AC 56, 57, 58 widgets, bookmarks, search | AC 31–34 | kept |

