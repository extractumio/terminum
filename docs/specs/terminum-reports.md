# terminum — Specification (Reports and the Inbox)

Sibling of `terminum-kanban.md` (tasks and the board), `terminum-dispatcher.md` (who runs a task), `terminum-schedules.md` (routines) and `terminum-supervisor.md`. This spec holds reports: what an agent, a routine or the Supervisor gives a person to read, kept apart from tasks and read in the Inbox like mail. Cases are numbered across the specs.

## 0. Metadata

| Field | Value |
|---|---|
| Status | approved by the owner (TERM-822's requirement); built |

A snapshot of what terminum must do, not a record of how it came to be: git holds that.

## 1. Overview

**Problem.** A result for a person to read (a schedule's result, a review, an audit) waited in WAIT FOR HUMAN beside the questions agents ask, and reminded like one. The human could not tell at a glance what needs an answer from what can be read later.

**Who it affects.** One person running many coding agents and routines, who reads their results on the board in their own time and answers their questions at once.

**Expected outcome.** WAIT FOR HUMAN holds only tasks that need a human's answer or action. Every report goes to the **Inbox**, a section above the Sessions in the session list, on both pages and on a phone: unread or read, deleted with Undo, read in a pane over the board's right part. Agents make reports by their tools; a routine's run and the Supervisor make them by themselves. A report raises no alert unless it is critical. The human can reply to a report: the reply goes back into its agent's conversation, and the answer comes back below the report, or as a new report it links to.

**Scenario.** At 03:00 routine "Nightly deps check" runs S_TERM-41, whose agent ends with its result; terminumd makes it report R_TERM-7, and S_TERM-41 is DONE. Alex opens the board at 09:00: the Inbox says 3 unread, R_TERM-7 in bold. Alex presses it; the reading pane covers the columns, and after 2 seconds R_TERM-7 is read. Alex replies "Which of them break the build?" and presses Send: S_TERM-41's conversation goes on, the agent answers below the report as a comment, and the report shows "Opus is answering" until it does. Alex deletes R_TERM-5 by Backspace and presses Undo in the toast. At 14:00 the Supervisor opens a critical incident; its critical report R_TERM-8 stands red at the top of the Inbox, alerting once through the incident.

## 2. Scope and non-goals

In scope:
- The report record, its store and routes; agents' report tools and roles; the Inbox on the session page, the board and a phone; the reading pane; read, unread, delete, restore; replies; alerts; where reports come from (agents, routines, the Supervisor); links from tasks, sessions and Find; the move of waiting reports at upgrade.

Out of scope (explicit):
- Reports shared with other people, sent by mail or to a channel: the Inbox is the signed-in page's. A critical incident still leaves through the Supervisor's line (`terminum-supervisor.md` AC-227).
- Several humans' own read state: one sign-in, one human (`terminum-kanban.md` AC-119); `read_at` is that human's.
- A reply to a report made in a session terminumd did not start (no task, or a task not run by Dispatcher): refused with where to answer; terminumd never types into a session it did not start (`CLAUDE.md` §2).
- Editing a report by the human: they reply or delete; a manager or the Supervisor edits by its tools.
- A report's owner, status, budget or column: a report is read, not worked; its severity is critical or not.
- The requirement's "the reply goes back to the agent, in the same session" holds for reports made in a task's run Dispatcher started; for one made in a session the human opened, a manager's, or by the Supervisor, the reply field says where to answer instead (AC-270): typing into a session terminumd did not start is outside `CLAUDE.md` §2. The owner confirms this narrowing.
- Folders, labels, search operators in the Inbox: Find and the project filter cover the need.

## 3. Assumptions, dependencies, open questions

Assumptions:

| ID | Assumption | Owner | Status |
|---|---|---|---|
| AS-265 | A report is read in full on the board, on a computer or a phone; nobody needs it as a file elsewhere. | Owner | stated |
| AS-266 | Reports come at most a few hundred a week per project; a list paged by 100 rows and a soft delete kept 30 days hold them. | Claude | stated |

Dependencies:

| ID | Dependency | Owner | Status |
|---|---|---|---|
| DEP-265 | The store and its files (`terminum-kanban.md` AC-109, AC-119), task comments (AC-108), AC-117's lines, the session list (AC-135). | Claude | present |
| DEP-266 | RUN's continuation and the §9 gate (`terminum-dispatcher.md` AC-122, §9): a reply goes through them. | Claude | present |
| DEP-267 | The Panel's Markdown rules and Mermaid in strict mode (`CLAUDE.md` §2): a report's body is shown by them. | Claude | present |

Open questions: none.

## 4. Acceptance cases

| ID | Priority | Case |
|---|---|---|
| AC-265 | [MUST / P0] | A report is a record of its own in the store, never a task: an ID `R_<prefix>-<n>` in its project's count of task IDs, as `S_` and `I_` are, a title, a summary, a Markdown body, critical or not, unread or read, its author with role and session, its source (a task, a schedule, a Supervisor digest or incident, or none), files, comments, links to the report it follows and to its follow-ups, when made and changed, deleted or not; limits said in words. |
| AC-266 | [MUST / P0] | Agents make reports by create_report, add to them by append_to_report and attach_report_file, read them by get_report and list_reports; a worker writes only to reports of its own tasks or session, at most 10 an hour, a manager or the Supervisor of the project to any of it, and only they update_report or delete_report; a report made for a task links it by a comment on the task; finish_task has no report argument, and RUN's prompt and the tools' instructions say when to report. |
| AC-267 | [MUST / P0] | The Inbox stands above the Sessions in the session list, on the session page, the board and a phone's sheet: its head with the unread count, folding like a group; rows newest first, unread in bold, critical marked red and first while unread, each with title, summary, source and age; it follows the board's project choice; keys, a right-click menu and buttons for every action; an empty Inbox says so. |
| AC-268 | [MUST / P0] | Pressing a report opens the reading pane over the board's right part (its columns, agents and routines), on the session page by opening the board with it: header, body by the Panel's Markdown rules, files, comments, reply field; a link `/kanban/?report=<ID>` opens it; Esc or × gives the columns back; on a phone it covers the board. |
| AC-269 | [MUST / P0] | A report open 2 seconds becomes read; the human marks it unread again; Delete (a button, Backspace or Delete, or the menu) hides it at once with Undo in a toast; deleted reports stand under the Inbox's Deleted view for 30 days, restorable, then go with their files. |
| AC-270 | [MUST / P0] | A reply to a report made in a task's run goes on in that task's conversation as a human's RUN does (`terminum-dispatcher.md` AC-122), through the §9 gate, in fixed words with the report's ID and the human's text; the report says "<agent> is answering" while it runs; the answer comes as comments below the report, or as a new report that follows it when the human asked for one; refused in words when the report has no such task. |
| AC-271 | [MUST / P0] | A report raises no alert, no toast and no line: only the Inbox's count; a critical one raises `terminum-kanban.md` AC-117's line in red until it is read or the line closed, once: a report of a Supervisor incident alerts through the incident alone. |
| AC-272 | [MUST / P0] | A routine's run that ends with a result and made no report of its own makes one from its result, its task DONE; its card shows its last report and how many of its reports are unread; every report stays (none supersedes another); a routine's question still waits in WAIT FOR HUMAN. The Supervisor's daily digest is a report, and an incident it opens makes a critical report. |
| AC-273 | [SHOULD / P1] | A task's window lists its reports; a report's ID in a comment or result opens it; a session row marks the unread reports its session made; Find finds reports by ID, title and summary. |
| AC-274 | [MUST / P0] | At upgrade every task waiting in WAIT FOR HUMAN with its report becomes a report in the Inbox, unread, with that result as its body, and its task DONE with a comment linking it: nothing lost, done once. |

## 5. BDD scenarios

### AC-265 — A report [MUST / P0]

```gherkin
Scenario: AC-265 happy path — made
  Given project Terminum, prefix TERM, its last report R_TERM-6
  When an agent of TERM-41 makes a report "Nightly deps check", summary "2 outdated packages", a body of Markdown, not critical
  Then R_TERM-7 is kept: the project, title, summary, body, critical no, unread, its author (the agent's name, its role worker, its session's key), its source task TERM-41, made and changed now, no files, no comments
  And its number comes from the project's count of task IDs (terminum-kanban.md AC-148), as a schedule's task's does: no task is TERM-7, S_TERM-7 or I_TERM-7, and R_ is no project prefix
  And a severity is "critical or not": a report has no owner, status, budget or column; it is read, not worked

Scenario: AC-265 failure — a report's ID is no task's
  When an agent calls get_task, add_comment or any task tool with R_TERM-7
  Then it is refused: "R_TERM-7 is a report: use get_report."
  And a page or a link that names R_TERM-7 opens the report, never a task whose ID it holds

Scenario: AC-265 edge — follows another
  When a report is made with follows R_TERM-7
  Then R_TERM-9 says "Follows R_TERM-7", and R_TERM-7 says "Follow-up: R_TERM-9", both links opening the other
  And follows names a report of the same project, not deleted; else it is refused: "R_TERM-70 is no report of Terminum."

Scenario: AC-265 failure — limits
  Then a title holds 1 to 200 characters, a summary at most 500, a body at most 1 MiB, a comment at most 256 KiB, a file at most 100 MB (`terminum-kanban.md` AC-109), and 50 files per report
  And each over its limit is refused in words naming it ("Body: at most 1 MiB"), nothing kept
  And control characters but tab and newline are dropped from each text, as a task's are
```

### AC-266 — Agents' report tools [MUST / P0]

```gherkin
Scenario: AC-266 happy path — a worker reports on its task
  Given worker session %8 running TERM-41 of Terminum (terminum-agents.md AC-174)
  When it calls create_report {title, summary, body, task: "TERM-41"}
  Then R_TERM-7 is made as AC-265 says, and TERM-41 gets a comment by that agent "Report R_TERM-7: <title>", its ID a link to the report (AC-273)
  And the tool answers "R_TERM-7 is in the human's Inbox. Finish TERM-41 with finish_task as usual: the report is not its result."
  When it calls finish_task on TERM-41 with its result
  Then TERM-41 is DONE as any task (terminum-dispatcher.md AC-123)

Scenario: AC-266 happy path — no task named
  Given a session running one task, TERM-41
  When it calls create_report without task
  Then the report's source is TERM-41
  Given a session running none or several of its tasks
  Then the report has no source task, its author's session named; several tasks: refused "Name the task this report is for: TERM-41 or TERM-42."

Scenario: AC-266 happy path — append and attach
  When the worker calls append_to_report {id: "R_TERM-7", text}
  Then the text is a comment below the report, by that agent, and the report's changed time moves; it stays read or unread as it was
  When it calls attach_report_file {id, path} or {id, name, text}
  Then the file stays with the report as a task's does (terminum-kanban.md AC-109): a path in its session's folder, a text from a host

Scenario: AC-266 failure — a worker writes only to its own
  When a worker calls append_to_report or attach_report_file on a report whose task is not its session's and whose session is not its own
  Then it is refused: "R_TERM-5 is not of your task or session: you can read it with get_report."
  When a worker calls create_report with a task its session does not run, or follows a report not of its task or session
  Then it is refused: "TERM-40 is not a task of this session: report on your own task, or name none." or "R_TERM-5 is not of your task or session: follow only your own reports."

Scenario: AC-266 failure — a worker's reports are bounded
  Given worker session %8 made 10 reports in the last hour
  When it calls create_report
  Then it is refused: "This session made 10 reports in the last hour: put more in one report, or append to it."
  Given a critical report of session %8 raised its AC-117 line in the last hour
  When it makes another critical report
  Then the report is kept critical, red in the Inbox, and raises no line: one line an hour per session (AC-271)
  When a worker calls update_report or delete_report
  Then the tool is not found for it, as a manager's tool (terminum-agents.md AC-174)

Scenario: AC-266 happy path — a manager or the Supervisor
  Given Terminum's manager, or its Supervisor caller (terminum-supervisor.md AC-231)
  When it calls update_report {id, title?, summary?, body?, critical?} on a report of Terminum
  Then those fields change, the report's history saying who changed them, and its read state stays
  When it calls delete_report {id}
  Then it is deleted as the human's Delete is (AC-269), restorable from the Inbox's Deleted view
  And a report of another project is refused: "R_API-3 is of project API, not yours."

Scenario: AC-266 happy path — read
  When any caller calls get_report {id}
  Then it gets the report, its comments oldest first, its files' names and sizes, its links, whether it is read
  When it calls list_reports {unread?, limit?}
  Then it gets its project's reports newest first, at most limit (default 20, at most 100), not deleted, each with ID, title, summary, critical, read, source, made

Scenario: AC-266 edge — what the agent is told
  Then RUN's prompt says after its finish_task line "When the deliverable is a report for a person to read (a review, an audit, an answer to a question), give it by execute_tool(\"create_report\", {task, title, summary, body}) before finish_task: it goes to the human's Inbox, and the task is DONE as any."
  And create_report's description says the same, and finish_task's: "A report for the human to read goes first by create_report; a routine's result also goes to their Inbox as one."

Scenario: AC-266 failure — finish_task's old report argument
  When an agent calls finish_task with report: "yes"
  Then it is refused, nothing moved: "finish_task takes no report: give the report by execute_tool(\"create_report\", {task, title, summary, body}), then call finish_task again without it."
```

### AC-267 — The Inbox [MUST / P0]

```gherkin
Scenario: AC-267 happy path — above the Sessions
  Given reports R_TERM-7 (unread), R_TERM-6 (read), R_TERM-8 (critical, unread)
  When the user opens the session page or the board
  Then the session list starts with the Inbox: its head "Inbox" with "2" unread, above the Sessions' first group
  And its rows, newest first but critical unread ones first: R_TERM-8 red-marked, R_TERM-7 bold, R_TERM-6 plain; each with its title, summary on a second line cut by an ellipsis, its source (task ID, routine name, "Supervisor", or its author's name) and age ("3 h")
  And at most 8 rows show, then "Show all (23)" opens the rest in the same list, paged by 100 from terminumd

Scenario: AC-267 happy path — the board's project choice
  Given the board shows project Terminum
  Then the Inbox lists only Terminum's reports and counts only them; with all projects shown it lists them all, each row with its project's color in a dot as a session row's
  And the session page's Inbox follows the board's last choice, kept in the browser as the board keeps it; with none it shows all projects

Scenario: AC-267 happy path — folds like a group
  When the user presses the Inbox's head
  Then the Inbox folds to its head with its count, and stays folded on both pages and after a reload, as a group of sessions does

Scenario: AC-267 happy path — keys
  Given the focus on an Inbox row
  Then ↑ and ↓ move it, Enter opens the report (AC-268), Backspace or Delete deletes it (AC-269), U toggles read and unread
  And the keys act only while the focus is in the Inbox, never in a text field or the terminal

Scenario: AC-267 happy path — menu and buttons
  When the user right-clicks a row, or long-presses it on a touch screen
  Then a menu offers Open, Mark as read or Mark as unread, Open source task (when it has one), Copy link, Delete
  And hovering a row shows its unread toggle and its delete button (on a touch screen they stand at its right), so every action has a button

Scenario: AC-267 edge — empty
  Given no report in the projects shown
  Then the Inbox's head shows no count and its body says "No reports", one line

Scenario: AC-267 edge — a phone
  Given a window narrower than 861 px, where the list is a sheet (terminum.md AC-18)
  Then the Inbox stands at the top of that sheet, as above, and the list's button wears a dot while any report is unread
```

### AC-268 — Reading a report [MUST / P0]

```gherkin
Scenario: AC-268 happy path — the reading pane
  When the user presses R_TERM-7 in the Inbox on the board
  Then a pane covers the board's right part (the agents, routines and columns), the list staying beside it
  And its head shows the title, critical when it is, the source as a link (a task opens its window, a routine its card, an incident its card), its author with role, when made and changed, its project's name, and the buttons Mark as unread, Delete, Copy link and Close
  And under it the summary, the body rendered by the Panel's Markdown rules (raw HTML off, `javascript:` links dropped, remote images not loaded, Mermaid in strict mode), its files (images shown, others downloaded, nothing run), "Follows" and "Follow-up" links, its comments oldest first, and the reply field with Send last when it can be answered (AC-270), else why not

Scenario: AC-268 happy path — from the session page
  When the user presses a report in the Inbox on the session page
  Then the board opens with that report's pane, in the same tab; ⌘-click or the menu's Open in new tab opens it in another
  When the user presses the browser's Back
  Then the session page stands as it was, its session shown

Scenario: AC-268 happy path — a link
  When the browser opens /kanban/?report=R_TERM-7
  Then the board opens with R_TERM-7's pane; Copy link copies that address
  And a link to a deleted report opens its pane with "Deleted: restore it to keep it." and Restore; an unknown ID says "No report R_TERM-70." where the pane would stand

Scenario: AC-268 happy path — closing
  When the user presses ×, or Esc with the focus outside a text field
  Then the pane goes and the columns stand as they were, scrolled where they were; the address loses its report

Scenario: AC-268 edge — changes while open
  Given R_TERM-7 open
  When an agent appends a comment or a manager updates it
  Then the pane shows it within 2 s, keeping the reply being written and its scroll

Scenario: AC-268 edge — a phone
  Given a window narrower than 861 px
  Then the pane covers the whole board under its head, × at its top, and the list's sheet closes when a report opens
```

### AC-269 — Read, unread, deleted [MUST / P0]

```gherkin
Scenario: AC-269 happy path — read by reading
  When R_TERM-7 stays open in the pane for 2 seconds, its page visible
  Then it is read, in every page within 2 s; opened and closed sooner it stays unread
  When the user presses Mark as unread, U, or the row's toggle
  Then it is unread again, and opening it again reads it again after 2 seconds

Scenario: AC-269 happy path — delete and undo
  When the user deletes R_TERM-5 by its button, Backspace or Delete on its row or in its pane, or the menu
  Then it leaves the Inbox at once, its pane closing, and a toast says "R_TERM-5 deleted" with Undo for 8 seconds
  When the user presses Undo
  Then R_TERM-5 stands again where it stood, read or unread as it was

Scenario: AC-269 happy path — restore later
  When the user presses Deleted reports in the Inbox's head
  Then the Inbox lists the deleted reports, newest deletion first, each with Restore and the day it goes for good
  When the user presses Restore
  Then it stands in the Inbox again
  And a report deleted 30 days ago goes with its files, comments and links at the next hourly sweep; a report that follows it says "Follows a deleted report"

Scenario: AC-269 failure — terminumd refuses
  When a read, unread, delete, undo or restore is refused or terminumd cannot be reached
  Then the row stands as before and the refusal shows in a toast with terminumd's words
```

### AC-270 — Replying [MUST / P0]

```gherkin
Scenario: AC-270 happy path — a reply goes on in the conversation
  Given R_TERM-7, made in the run of S_TERM-41, now DONE, its session closed
  When the user writes "Which of them break the build?" under R_TERM-7 and presses Send
  Then S_TERM-41 goes on as a human's RUN of it does (terminum-dispatcher.md AC-122): in its live session, else from its pane's shell, else its conversation resumed in a new window, on its agent's slot or waiting for one (AC-158)
  And the words it gets are fixed but for the reply: "S_TERM-41: the human replied to your report R_TERM-7. Answer below it by execute_tool(\"append_to_report\", {id: \"R_TERM-7\", text}), or, when they ask for a new report, by execute_tool(\"create_report\", {task: \"S_TERM-41\", follows: \"R_TERM-7\", title, summary, body}); then call finish_task on S_TERM-41 as before.\n\n<reply>"
  And only once the press is accepted is the reply a comment by the user on R_TERM-7, in the transaction that keeps the press, and S_TERM-41's history says "<the user> replied to R_TERM-7"

Scenario: AC-270 failure — refused, nothing kept
  When Send is refused (the agent works, a draft stands in its box, a RUN of it waits, the gate refuses: terminum-dispatcher.md AC-122)
  Then no comment is kept on R_TERM-7, the reply stays in its field, and RUN's refusal shows under it in its words
  When the cause clears and the user presses Send again
  Then the reply is kept once

Scenario: AC-270 happy path — answering
  Given the reply accepted
  Then R_TERM-7's pane and row say "Opus is answering" (its agent's name) while its task is IN PROGRESS or waits for a slot from that reply
  When the agent appends a comment to R_TERM-7, or makes R_TERM-9 that follows it
  Then the answer stands below the reply, or R_TERM-9 stands unread in the Inbox and R_TERM-7 says "Follow-up: R_TERM-9"; an appended answer makes R_TERM-7 unread again
  When the task ends with no answer
  Then "is answering" goes, and R_TERM-7 says "No answer from Opus: S_TERM-41 is <status>."

Scenario: AC-270 edge — a reply's run makes no second report
  Given a run a reply started
  When its agent calls finish_task with its result
  Then the task is DONE with that result as a comment, and no report is made from it (AC-272): its answer went by append_to_report or create_report

Scenario: AC-270 edge — a routine's next firing during a reply
  Given S_TERM-41 of SCH-1 runs from a reply when SCH-1 is due
  Then that firing is skipped as AC-162 skips one while a task of it runs, the Scheduled page saying why

Scenario: AC-270 failure — no one to answer
  Given a report whose source is no task Dispatcher ran: a session the human opened, a manager's, the Supervisor's digest or incident
  Then its pane has no reply field: GET /api/reports/{id} says "replyable": false with why, shown where the field would stand: "R_TERM-7 was made in a session terminum did not start: answer in that session." with a link to it while it lives; "Answer through Talk on incident 12." with Talk (terminum-supervisor.md AC-244); "A digest: nobody to answer."
  And a reply sent anyway is refused with the same words, nothing kept
  And a report of a task that is archived says "S_TERM-41 is archived: bring it back to reply."

Scenario: AC-270 edge — only fixed words carry the report
  Then the words name the report by ID only; the report's body, summary and agents' comments are never typed: the agent reads them by get_report
  And the reply goes through the same checks of the §9 gate as a RUN's saved comments do: a character the gate refuses refuses the reply with its words, nothing typed
```

### AC-271 — Alerts [MUST / P0]

```gherkin
Scenario: AC-271 happy path — no alert
  When a report that is not critical comes
  Then no AC-117 line, no toast, no sound, no counter on a touch screen: only the Inbox's count rises, and the Kanban section's own count stays the board's cards

Scenario: AC-271 happy path — critical
  When a critical report R_TERM-8 of Terminum comes
  Then terminum-kanban.md AC-117's line reads "Terminum: R_TERM-8 critical: <title>" in FAILED's red on both pages, counted in the red counter on a touch screen, and opens its pane
  And its row stands first in the Inbox, red, while unread
  When it becomes read, or the user closes its line
  Then the line goes in every browser at once (AC-117)
  When a manager updates a report to critical while it is unread
  Then its line comes then
  And a worker's session raises at most one such line an hour; its later critical reports stand red in the Inbox without one (AC-266)

Scenario: AC-271 edge — an incident alerts once
  Given the Supervisor opened incident 12 and made its critical report R_TERM-8 (AC-272)
  Then the incident's own alert stands (terminum-supervisor.md AC-225) and R_TERM-8 raises no AC-117 line of its own; it stands red and first in the Inbox while unread
```

### AC-272 — Where reports come from [MUST / P0]

```gherkin
Scenario: AC-272 happy path — a routine's run
  Given S_TERM-41 of routine SCH-1 (terminum-schedules.md AC-162)
  When its agent calls finish_task with its result, and its run made no report for S_TERM-41 and was not started by a reply (AC-270)
  Then the result is its comment, S_TERM-41 is DONE, and terminumd makes R_TERM-7: title S_TERM-41's title, summary the Outcome's first line, body the result, source S_TERM-41 and SCH-1, its author the agent
  And the tool answers "S_TERM-41 is DONE; its result went to the human's Inbox as R_TERM-7."
  When the agent made a report for S_TERM-41 itself in that run before finish_task
  Then the result stays a comment and no second report is made

Scenario: AC-272 happy path — the routine's card
  Then SCH-1's card shows its last report's ID and age as a link to it, and "3 unread" when its reports unread number 3
  And no report supersedes another: each stays until the human deletes it

Scenario: AC-272 edge — a routine's question
  When S_TERM-44's agent calls ask_human
  Then S_TERM-44 waits in WAIT FOR HUMAN with its questions, as any task (terminum-dispatcher.md AC-157), and its card says "waits for you"

Scenario: AC-272 happy path — the Supervisor
  When the Supervisor's daily digest has news (terminum-supervisor.md AC-243)
  Then it is also a report: title "Supervisor digest, <date>", its lines as the body, source "Supervisor", not critical
  When the Supervisor opens an incident (terminum-supervisor.md AC-224)
  Then a critical report "Incident: <the incident's title>" with its first evidence and a link to the incident's card is made, alerting as AC-271's edge says
  And a round may report to the owner by create_report as the Supervisor caller (AC-266)
```

### AC-273 — Links [SHOULD / P1]

```gherkin
Scenario: AC-273 happy path — from a task
  Given TERM-41 with reports R_TERM-7 and R_TERM-9
  Then its window lists "Reports: R_TERM-7, R_TERM-9" under its details, each a link that opens the report's pane over the board
  And "R_TERM-7" in any comment, result or text on the board is such a link; a deleted one's link opens it as AC-268 says

Scenario: AC-273 happy path — from a session
  Given session %8 made R_TERM-7, unread
  Then its row in the session list shows an envelope with 1; a press on it opens R_TERM-7, several open the Inbox filtered to them

Scenario: AC-273 happy path — Find
  When the user types "R_TERM-7" or a word of a report's title or summary in the board's Find
  Then the Inbox shows only the reports it matches, as the columns show only the tasks it matches; a press opens its pane
```

### AC-274 — Waiting reports move to the Inbox [MUST / P0]

```gherkin
Scenario: AC-274 happy path — at upgrade
  Given TERM-41 waits in WAIT FOR HUMAN with its report (its last move marked report), its result the agent's comment
  When terminumd starts on a store of before reports
  Then R_TERM-<n> is made, unread, its title TERM-41's title, its summary the result's Outcome line, its body that result, its author that agent, its source TERM-41 (and its routine, when a routine's), made at the result's time
  And TERM-41 is DONE, its history "Dispatcher moved it from WAIT FOR HUMAN to DONE: its report went to the Inbox as R_TERM-<n>", with that comment
  And it is done in one transaction, once: a restart makes no second report
  And a task in WAIT FOR HUMAN with questions stays as it is
```

## 6. Flow and sequence diagrams

```mermaid
sequenceDiagram
  participant A as Agent (task S_TERM-41)
  participant T as terminumd
  participant P as Page (Inbox)
  A->>T: create_report / finish_task
  T->>T: reports row, task comment, DONE
  T-->>P: {"t":"reports"}
  P->>T: GET /api/reports
  P->>T: POST /api/reports/R_TERM-7/read (after 2 s open)
  P->>T: POST /api/reports/R_TERM-7/reply {text}
  T->>T: reply comment; RUN of S_TERM-41 (AC-122, §9 gate)
  T->>A: fixed words + reply (typed through the gate)
  A->>T: append_to_report / create_report {follows}
  T-->>P: {"t":"reports"}
```

## 7. Current behavior and gaps

| Case | Gap |
|---|---|
| AC-270 | Replying is not built: a report's pane offers "Continue in <task>", its task's window, where a comment and RUN go on in the same conversation (`terminum-dispatcher.md` AC-122). |
| AC-273 | A session row shows no envelope of its unread reports, and a report's ID in a comment or result is not a link; a task's window lists its reports and Find finds them. |

## 8. Recommendation and ownership

The Inbox is the session list's first section, on both pages, because the list is one widget the board embeds (`terminum-kanban.md` AC-135); the reading pane lives on the board, whose right part it covers, so the session page opens the board with the report rather than holding a second reader. A reply reuses RUN's continuation and its gate rather than a new way of typing. Owner: Claude, for the maintainer.

## 9. Technical details and specifications

**Store** (`terminumd/src/reports/`), in `kanban.sqlite` of `terminum-kanban.md`:

```sql
CREATE TABLE reports (id TEXT PRIMARY KEY, project TEXT NOT NULL REFERENCES projects, number INTEGER NOT NULL, slug TEXT NOT NULL UNIQUE,
  title TEXT NOT NULL, summary TEXT NOT NULL, body TEXT NOT NULL, critical INTEGER NOT NULL DEFAULT 0,
  author TEXT NOT NULL REFERENCES users, role TEXT NOT NULL, session_key TEXT, task TEXT REFERENCES tasks, schedule TEXT REFERENCES schedules,
  run TEXT, incident INTEGER, follows TEXT REFERENCES reports, read_at INTEGER, line INTEGER NOT NULL DEFAULT 0, line_seen INTEGER,
  deleted_at INTEGER, replied_at INTEGER, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE report_notes (id TEXT PRIMARY KEY, report TEXT NOT NULL REFERENCES reports ON DELETE CASCADE, at INTEGER NOT NULL,
  author TEXT NOT NULL REFERENCES users, kind TEXT NOT NULL, text TEXT NOT NULL);   -- kind: comment, reply, change
ALTER TABLE files ADD COLUMN report TEXT REFERENCES reports;
```

The ID is `R_<prefix>-<number>`, the number taken from the project's `next_seq` as a task's is (`slugs.rs`), so no task's ID holds it; a project's new prefix leaves older reports' IDs as they were (they are looked up by `slug`), and the count stays above every report's number too. `run`: the Dispatcher run the report was made in, so AC-272 knows a run's own report and AC-270 a reply's run. `line`: 1 when it raises AC-117's line (critical, not an incident's, within its session's one an hour); `line_seen` the time a human closed it. `role`: `human`, `worker`, `manager`, `supervisor`, `terminum`. A report's files are task files (`terminum-kanban.md` AC-109) whose row names the report instead of a task. `replied_at` is the last reply's time: "is answering" while the report's task is IN PROGRESS or waits for room and no agent note or follow-up came after it. The hourly sweep deletes reports with `deleted_at` 30 days past, their notes, and their files through `files.rs`'s own deletion (bytes set aside, then the row); a follow-up's `follows` is set to NULL. The 7-day sweep of files joined to nothing skips a report's. A file row names a task or a report, never both: the code that writes them keeps it so (a `CHECK` would rebuild `files`, whose rows task history points at).

**Routes**, each write with the exact Host, Origin and Content-Type checks of every write (`SECURITY.md`):

```text
GET    /api/reports?project=<id>&deleted=1&from=<n>&session=<key>&limit=<n> → {"reports": [{"id": "R_TERM-7", "number", "project", "project_name", "project_color", "title", "summary", "critical", "read", "source": {"task": "S_TERM-41", "schedule": "SCH-1", "incident": null, "name": "Nightly deps check"}, "author_name", "role", "session", "created_at", "updated_at", "deleted_at"}], "unread": {"projects": {"<project>": n}, "schedules": {"SCH-1": n}, "sessions": {"<key>": n}}, "more": true, "next": 100}
GET    /api/reports/{id} → the same, with "body", "notes": [{"id", "at", "author_name", "author_kind", "kind", "text"}], "files": [{"id", "name", "bytes", "mime"}], "follows", "follow_ups": []
POST   /api/reports/{id}/read      POST /api/reports/{id}/unread
DELETE /api/reports/{id}           POST /api/reports/{id}/restore
POST   /api/reports/{id}/reply {text} → {"task": "S_TERM-41"} or 400 with why (AC-270)
POST   /api/reports/{id}/seen {key}   (the AC-117 line closed)
```

The list's answer carries the unread counts by project, by schedule and by session, so neither a routine's card nor a session row asks again. Every page hears the tasks' marks (`taskmarks`, `terminum-kanban.md` AC-113) with `"reports": {"rev", "unread"}`: `rev` moves with any change of a report (made, changed, read, deleted, a line closed), and the Inbox reads `GET /api/reports` again then; also when its page becomes visible and every 30 s while it is, for what the Supervisor makes without a task's change. A critical report's line joins the marks' `lines` with `"report": true`; its × posts `…/seen`.

**Tools** (`terminum mcp`, `terminum tool`): `create_report {task?, title, summary, body, critical?, follows?}`, `append_to_report {id, text}`, `attach_report_file {id, path | name + text}`, `get_report {id}`, `list_reports {unread?, limit?}`, manager and Supervisor only: `update_report {id, title?, summary?, body?, critical?}`, `delete_report {id}`. Roles by `terminum-agents.md` AC-174 and `terminum-supervisor.md` AC-231.

**Reply** (`POST …/reply`): checks the report is replyable (a task Dispatcher ran, not archived); presses the task's RUN with the reply's fixed words as its message, through `terminum-dispatcher.md`'s press and §9 gate unchanged: the gate's checks of a saved comment apply to the reply; only an accepted press keeps the reply as a `reply` note, in the press's transaction, and sets `replied_at`. The run it starts is marked a reply's, so its finish_task makes no report (AC-272). Nothing else is typed.

**Security.** The reply is a human's message carried by a human's RUN, named in `CLAUDE.md` §2's list as "a human's reply to a report (`terminum-reports.md` AC-270): through the §9 gate, as a RUN of the report's task Dispatcher ran, fixed words with the report's ID and the human's text, never the report's text", and in `SECURITY.md`: only a signed-in page sends it, only into the pane of a task Dispatcher started, through the §9 gate; an agent's report text is never typed. A worker's reports are bounded (10 an hour, one critical line an hour per session) so a misled agent cannot flood the Inbox or the human's attention. A report's body is agents' text: the page shows it only through the Panel's Markdown rules. An agent's report tools write only to its project's reports, a worker only to its own.

**Limits.** Title 200 characters, summary 500, body 1 MiB, comment 256 KiB, 50 files of 100 MB each; the Inbox reads 100 rows a page; deleted reports kept 30 days.

## 10. Delivery stages

| Stage | Cases | Exit |
|---|---|---|
| 2 | AC-270 (with `CLAUDE.md` §2 and `SECURITY.md`, independently reviewed), the rest of AC-273 | landed green |
| 3 | The Supervisor's use of reports, the maintainer's addition (TERM-822): a critical report as a signal and incident, reports as detector evidence, the weekly review and the quarterly audit as reports, reading in the owner's burden, the report tools by the Supervisor's level; its cases written here first, agreed with `terminum-supervisor.md`'s open work | landed green |

## 11. Checklist with Definition of Done

- [x] AC-265 — `cargo test reports` (`ac265_…`: the shared count, a report's ID refused by task tools, limits in words)
- [x] AC-266 — `cargo test reports` (`ac266_…`: a worker's own and its 10 an hour, several tasks, a manager's change and delete, finish_task's report refused), `cargo test cli::mcp_tasks` (the call's shape), `cargo test roles` (`ac231_…`: the tools by role)
- [x] AC-267 — `web/test/e2e_all.mjs` (`webInbox`), Chromium and WebKit, wide and phone
- [x] AC-268 — `web/test/e2e_all.mjs` (`webInbox`: the pane over the board, its address, Esc, from the session page, a phone's sheet)
- [x] AC-269 — `cargo test reports` (`ac269_…`: read, unread, deleted, restored, purged at 30 days with its files), `web/test/e2e_all.mjs` (`webInbox`: read after 2 s, U, Backspace, Undo)
- [ ] AC-270 — `cargo test reports::reply`, `web/test/e2e_live.mjs` (`reports`); `CLAUDE.md` §2 and `SECURITY.md` name the reply, reviewed by the `pragmatic` agent
- [x] AC-271 — `cargo test reports` (`ac271_…`: only critical, one an hour per worker's session, read takes it, an incident's none), `web/test/e2e_all.mjs` (`webInbox`: a line opens the report, its × the report's route)
- [x] AC-272 — `cargo test reports` (`ac272_…`), `cargo test dispatcher::fire` (`ac272_…`: none superseded), `web/test/e2e_liveschedules.mjs`, `scripts/e2e_roles.py`
- [ ] AC-273 — `web/test/e2e_all.mjs` (`webTaskWin`: a task's window lists its reports; `webInbox`: Find); the session row's envelope and ID links not built (§7)
- [x] AC-274 — `cargo test reports` (`ac274_…`: moved once, its task DONE and linked, a question's task stays)
