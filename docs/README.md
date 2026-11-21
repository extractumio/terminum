# terminum's docs

Each fact lives in one place; elsewhere, point to it. Read from the top down: this page says which document holds a question, a spec's section 4 lists its cases in a line each, and section 5 and section 9 hold the detail. `scripts/check_docs.py` reports on the shape and `/docs-review` looks after the rest (spec AC-103).

| Looking for | It is in |
|---|---|
| What terminum is and how to install it | [README](../README.md) |
| How to use it, every feature and command | [USAGE](USAGE.md) |
| How the parts fit, one picture | [ARCHITECTURE](ARCHITECTURE.md) |
| What it does exactly: cases, scenarios, limits, the test that proves each | [Specs](#specs) |
| The threat model | [SECURITY](../SECURITY.md) |
| The UI's look and its change checklist | [UI-GUIDELINES](UI-GUIDELINES.md) |
| How to contribute, worktrees | [CONTRIBUTING](../CONTRIBUTING.md) |
| The rules for humans and agents | [CLAUDE.md](../CLAUDE.md) |
| How a release is built, signed and published; CI | [RELEASING](RELEASING.md) |
| What changed in each release | [releases/](releases/) |

## Specs

A spec is a snapshot of what terminum must do: never a chronicle or a change log (git holds those), and it may hold what is not built yet. Acceptance cases `AC-nn` are unique across the specs and never renumbered. A requirement is a BDD scenario under its case.

| Spec | Holds | Cases |
|---|---|---|
| [terminum.md](specs/terminum.md) | The core: session list, screen, typing, sizing, managing sessions, sign-in, themes, keys, budgets, the parts kept apart under load, projects, the user's own tmux, the input log and message history, the interaction log, the Message history and Dashboard pages, one dialog at a time and its size, the sections' switcher, text boxes | AC-01…07, 09, 10, 12, 13, 18…20, 88, 89, 92, 100, 101, 147, 151, 152, 165, 167, 171…173, 175, 176, 184, 195 |
| [terminum-lifecycle.md](specs/terminum-lifecycle.md) | Install, run, restart, reboot, upgrade, uninstall, update notice; landing a worktree, the release's gate, a tidy machine and docs, a quiet machine for the gate, bounded diagnostic logs, a flaky check's one retry, a release of a pinned commit, the gate's passes and record, the nightly | AC-08, 11, 14…17, 96, 102…104, 146, 196, 197, 200…202 |
| [terminum-terminal.md](specs/terminum-terminal.md) | Terminal features: widgets, folds, bookmarks, reminders, search, links, paste, selection, traces, sent history | AC-31…38, 40, 90, 91 |
| [terminum-agents.md](specs/terminum-agents.md) | Coding agents: states, unread, plans and progress, the records they kept, idle agents' exit, terminum's tools and `terminum tool`, managers and workers | AC-39, 93, 97, 98, 153, 155, 174, 178…181 |
| [terminum-files.md](specs/terminum-files.md) | The Files panel | AC-51…73, 94, 95 |
| [terminum-remote.md](specs/terminum-remote.md) | Remote hosts: tmux over ssh, files through an agent, helpers, builds and releases, agents on a host, one server's hang kept from the others | AC-81…87, 99, 132 |
| [terminum-kanban.md](specs/terminum-kanban.md) | Tasks, the board at `/kanban/`, projects shared with the session list, agents' task tools, the store, task IDs, folded cards, grooming, a task's details (type, link, target, Blocked by, Follow-ups) | AC-105…120, 133…136, 139, 145, 148…150, 154, 159, 199 |
| [terminum-dispatcher.md](specs/terminum-dispatcher.md) | Dispatcher: RUN, a task's status as its session lives, time budgets, watching and closing sessions, an agent's queue, Auto-Dispatcher, each project's pool of agents and its limits, + Session, subscription stripes, who started a run, a main task run with its subtasks | AC-121…131, 137, 138, 140…144, 156…158, 166, 168…170, 183, 185 |
| [terminum-schedules.md](specs/terminum-schedules.md) | Scheduled tasks: a schedule's window and cadence, its kept slot, its firings, the board's section, the Scheduled page, Run now | AC-160…164, 182 |
| [terminum-integration.md](specs/terminum-integration.md) | The integration queue: a project's check, publish and after commands, batches of finished tasks merged and checked together, conflicts, notices to agents, the board's INTEGRATION column, a batch's integration task, a landing's seven stages | AC-186…194, 198 |
| [terminum-supervisor.md](specs/terminum-supervisor.md) | Project Supervisor: signals, guarded indicators, rounds, the problem register, incidents, holds, budgets, review and reflection | AC-211…263 |

Each spec has the same sections: 0 Metadata (status only), 1 Overview, 2 Scope and non-goals, 3 Assumptions, dependencies, open questions, 4 Acceptance cases, 5 BDD scenarios, 6 Flow and sequence diagrams, 7 Current behavior and gaps (open gaps only), 8 Recommendation and ownership, 9 Technical details and specifications, 10 Delivery stages (only what is not shipped), 11 Checklist with Definition of Done (one line per case: what proves it).
