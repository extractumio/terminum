# terminum — Specification (Integration queue)

Sibling of `terminum-kanban.md` (tasks and the board) and `terminum-dispatcher.md` (who runs a task). This spec holds the integration queue: how finished tasks' commits are merged, checked together as one batch and published to their project's branch, by terminumd, with the project's own commands. Cases are numbered across the specs.

## 0. Metadata

| Field | Value |
|---|---|
| Status | approved by the owner; not built |

A snapshot of what terminum must do, not a record of how it came to be: git holds that.

## 1. Overview

**Problem.** Ten agents finish within minutes of each other. Each merges the project's branch and runs the project's checks on its own; the first to push makes the others' runs moot and they run again, one after another. The machine stalls under ten builds at once, tasks fail for want of time, and the last task is in the branch hours later.

**Who it affects.** One person running many coding agents on one project, on one machine.

**Expected outcome.** A project names how its candidate is checked and published. A finished task queues its commit and stands in INTEGRATION, its time budget stopped. terminumd alone takes the queue: it picks the tasks that merge cleanly together, at most 8, builds one candidate, runs the project's check once, on one candidate at a time on the machine, and publishes a green one: every task of it is DONE. A task that conflicts or whose check is red goes back to its agent with what to do; the others go on. Agents hear of overlapping work while they still work, so that most tasks fit one batch.

**Scenario.** Billing's Integration is on, its check `make smoke`. API-12, API-13, API-14 and API-15 are queued within a minute. API-14 conflicts with API-12: it is held back, told once. Batch 7 is `origin/main` + API-12 + API-13 + API-15; its check runs 20 minutes, green; it is pushed, the three are DONE. API-14 then conflicts with `origin/main` and returns to its agent, which merges, resolves and queues again: batch 8, alone, green, DONE.

## 2. Scope and non-goals

In scope:
- A project's integration settings and the three ways to check a candidate: on this machine, in a CI the project's command drives, or not at all.
- The queue, batches, conflicts, the check, publishing; notices to agents; the board's INTEGRATION column; the tools and the command.

Out of scope (explicit):
- A hosting site's own queue (GitHub's merge queue, GitLab's merge trains): terminum would build no candidate there; no project of the owner's needs it.
- Projects on another server: the queue runs for projects of the Mac terminumd runs on; another server's project says so in its window.
- Resolving a conflict, choosing whose change wins, fixing a red check: the task's agent does, terminum only says where.
- An agent that decides whose failure a red batch is: the halves decide (AC-190).
- Knowing a check's steps: a check is one command and its exit status; a flaky step is retried by the project's own command.
- A long-lived integration branch: a candidate is built anew for each batch and never pushed by terminum but as the target itself.
- Slots for an agent's own builds while it works: the project's scripts (`terminum-lifecycle.md` AC-104).
- Uncommitted work: only commits are queued, merged and compared.

## 3. Assumptions, dependencies, open questions

Assumptions:

| ID | Assumption | Owner | Status |
|---|---|---|---|
| AS-186 | A project's folder is a checkout of one git repository, and its agents work in worktrees of it, so every queued commit is in that repository's store. | Owner | stated |
| AS-187 | `git merge-tree --write-tree` (git 2.38) says whether two commits merge cleanly, and in which files not, without a checkout, in well under a second for this size of repository. | Claude | to verify |
| AS-188 | A check is right to run on the candidate alone: a task's own targeted checks passed before it queued (the project's rule, not terminum's). | Owner | stated |
| AS-189 | A command started in a window of terminum's tmux server outlives terminumd and leaves its exit status in the dead pane (`remain-on-exit`, `#{pane_dead_status}`). | Claude | to verify |

Dependencies:

| ID | Dependency | Owner | Status |
|---|---|---|---|
| DEP-186 | `git` on the Mac, 2.38 or later, run by terminumd as a program: the platform's tool, no library. | Claude | required |
| DEP-187 | `terminum-kanban.md`: tasks, statuses, the board's columns (AC-105, five; a sixth by AC-193), a project's window (AC-139), comments. | Claude | required |
| DEP-188 | `terminum-dispatcher.md`: a budget counts only IN PROGRESS (AC-126), the gate and the hooks' context that carry Dispatcher's words (AC-123, AC-126), a project's place (AC-140), closing a session (AC-125). | Claude | required |
| DEP-189 | `terminum-agents.md`: the tools' catalog (AC-155), manager and worker (AC-174). | Claude | required |

Open questions: none.

## 4. Acceptance cases

| ID | Priority | Case |
|---|---|---|
| AC-186 | [MUST / P0] | A project's window holds Integration…: a switch, the target branch, the commands Check, Publish and After, the most tasks in a batch and the check's time limit; Check is a command run on this machine (which may itself drive a CI and wait for it) or empty for no check; the window shows what will run and where; off by default, and off nothing is merged or pushed. |
| AC-187 | [MUST / P0] | An agent queues its task's commit with its result by `integrate`: the task stands in INTEGRATION, its budget and idle clocks stopped, its session kept; queued again it replaces its commit; taken out it is IN PROGRESS again. |
| AC-188 | [MUST / P0] | terminumd alone builds a batch: from the fresh target, the queued commits in queue order that merge cleanly together, each its own merge commit named by its task's ID, at most the batch's size, which starts at 4, doubles after a green batch up to the project's most (8) and halves after a red one. |
| AC-189 | [MUST / P0] | A commit that conflicts with the target returns to its agent at once with the target's commit and the files; one that conflicts only with a task of the batch stays queued, told once which task it waits for, and is judged again when that one's batch ends. |
| AC-190 | [MUST / P0] | The project's Check runs once per candidate, one candidate at a time on the machine, in a window the human can read; red or over its time, a batch of several is checked again by halves, the first half first, and a task red alone returns to its agent with the check's last rows; a check outlives terminumd's restart. |
| AC-191 | [MUST / P0] | A green candidate is published only if the target has not moved: by terminum's own push, never forced, or by the project's Publish; then its tasks are DONE with their results, the project's folder is brought up to it where it can be, and After runs once for the batch; a failure there is said and undoes nothing. |
| AC-192 | [MUST / P0] | Agents are told early and in detail, in fixed words with tasks, sessions, files and what to do: an overlap with another task's commits while both still work, the target moving in files they changed, their place and expected batch, being held back, a return for a conflict or a red check, and landing; each notice is a comment on its task and reaches a working agent by its next hook, a waiting one by the command it waits on, and an idle one of Dispatcher's by the gate. |
| AC-193 | [MUST / P0] | The board shows INTEGRATION as a column where a shown project has integration on: the batch under check on top as one group with its state and time, the queue under it in order, each card saying its place or what it waits for; keys pause the queue, take a task out, queue a task and list past batches with their results. |

## 5. BDD scenarios

### AC-186 — A project's integration settings [MUST / P0]

```gherkin
Scenario: AC-186 happy path — the window
  Given Billing on this Mac, its folder /Users/alex/billing a git checkout
  When the user presses Integration… in Billing's window (terminum-kanban.md AC-139), beside Memory limits…
  Then a window "Integration of Billing" shows: On (a switch, off), Target "origin/main", Check, Publish, After (each one line in a field's well), "Tasks in a batch, at most" 8, and "Check ends after, minutes" 90
  And under an empty Check stands "No check: a candidate that merges is published.", under an empty Publish "terminum pushes the candidate to origin/main, never forced.", under an empty After "Nothing runs after."
  When the user turns it on, types Check "make smoke" and presses Save
  Then it is kept at once, every page shows INTEGRATION for Billing (AC-193), and the next batch runs by it

Scenario: AC-186 happy path — what will run
  Then "What runs", open, says in terminumd's words, before Save: where the candidate is built ("~/.terminum/integration/billing"), each command's line in full with the folder it runs in (Check and Publish in the candidate's tree, After in /Users/alex/billing), and the variables they get (§9)
  And the commands run by the user's login shell, as a + Session's shell does: the user's PATH and logins, nothing of terminum's token

Scenario Outline: AC-186 happy path — three ways to check
  Given Check is <check>
  When a candidate is built
  Then <what happens>

  Examples:
    | check | what happens |
    | `make smoke` | it runs on this Mac in the candidate's tree; exit 0 is green |
    | `scripts/ci_check.sh`, which pushes `$TM_INTEGRATION_SHA` to a branch of the project's choosing, waits for that commit's pipeline (GitHub Actions, GitLab CI) and exits with its result | the same rule: terminum knows only the exit status and the time limit; the pipeline's verdict stands on the very commit terminum publishes |
    | empty | no check runs: the candidate is published as soon as it is built, and its batch says "no check" |

Scenario: AC-186 happy path — a manager reads it
  When the project's manager calls integration_status {} (AC-187)
  Then it is told whether integration is on, the target, the three commands and the batch's size; no tool changes them: a signed-in page does

Scenario: AC-186 failure — refused
  When Save is pressed on, and the project has no folder, the folder is no git checkout, git is older than 2.38, Target names no remote branch of it, a command is over 2,000 characters or holds a control character, the batch's most is not 1 to 8, or the minutes not 1 to 1440
  Then the window says which: "Billing has no folder: set it in its window.", "/Users/alex/billing is not a git checkout.", "git 2.38 or later is needed: this is 2.30.", "origin/main: no such branch at origin.", "Check: one line, at most 2,000 characters.", "Tasks in a batch: 1 to 8.", "Check ends after: 1 to 1440 minutes."; nothing is kept and what was typed stays
  When the project is on another server
  Then Integration… says "Integration runs for projects on this Mac." and has no switch

Scenario: AC-186 edge — turned off, changed
  Given a batch under check and two tasks queued
  When the user turns it off
  Then the window asks "Turn integration off? Batch 7's check is ended and 5 tasks go back to IN PROGRESS."; confirmed, the check is ended, nothing is published, each task is IN PROGRESS and told (AC-192)
  When the user changes Check while a batch is under check
  Then that batch ends by the old line and the next runs by the new

Scenario: AC-186 edge — off
  Given integration off for Billing
  Then integrate answers "Integration is off for Billing: publish your work as its rules say.", the board shows no INTEGRATION for it, and terminumd runs no git command in its folder
```

### AC-187 — Queueing a task [MUST / P0]

```gherkin
Scenario: AC-187 happy path — queued
  Given API-12 IN PROGRESS, its agent in /Users/alex/billing/.worktrees/retry, a worktree of Billing's checkout, all committed, HEAD 4f2a91c not in origin/main
  When the agent calls integrate {id: "API-12", result: "## Result …"}
  Then API-12 is INTEGRATION, its queue entry 4f2a91c, its history saying "Queued 4f2a91c for integration into origin/main."
  And it is answered: its place ("3rd"), the batch expected ("with API-9 and API-10, after batch 7 ends"), and the conflicts foreseen with queued tasks and with the target (AC-189), in the words of AC-192
  And the result is kept as finish_task keeps one (terminum-dispatcher.md AC-123): it becomes the DONE comment when the task lands, with no further call

Scenario: AC-187 happy path — while it stands there
  Then API-12's time budget does not run (terminum-dispatcher.md AC-126), no idle clock runs and no request for a result is made (AC-123), and its session is not closed (AC-125)
  And the agent's turn may end: it goes on when a notice comes (AC-192)

Scenario: AC-187 happy path — waiting on it
  When the agent, or a project's script, runs "terminum integration wait API-12"
  Then it prints each notice as it comes and ends when API-12 leaves INTEGRATION: exit 0 landed, 1 returned for a red check, 2 returned for a conflict, 3 taken out or integration turned off
  And "terminum integration queue API-12 [--commit <sha>] [--result-file <path>] [--wait]" queues as the tool does, and with --title "<words>" in place of an ID it makes the task first, as the tool does with a title

Scenario: AC-187 happy path — a commit named, a task made
  When integrate is called with commit "9c1d0aa"
  Then that commit is queued instead of the caller's folder's HEAD
  When it is called with no id and a title, by a session with no task of its own
  Then a task is made in the caller's project as create_task makes one, by the caller, already in INTEGRATION

Scenario: AC-187 edge — queued again
  Given API-12 queued with 4f2a91c, not in the batch under check
  When its agent commits a fix and calls integrate again
  Then its entry is the new commit, in its old place, and its history says so
  Given API-12 in the batch under check
  Then the call is answered "API-12 is in batch 7, under check: take it out first (withdraw_integration), which checks the batch again without it." and nothing changes

Scenario: AC-187 happy path — taken out
  When the agent calls withdraw_integration {id: "API-12"}, or a human presses Take out on its card (AC-193)
  Then API-12 is IN PROGRESS, its budget runs on, its entry is gone
  And taken out of the batch under check, that check is ended and the batch is built again without it (AC-188)

Scenario: AC-187 failure — refused
  When the caller's folder has uncommitted changes, the commit is not in the project's repository, it is already in the target, the result lacks its sections, the task is another project's, or a worker names a task that is not its run's
  Then it is told which: "Commit your work first: 2 files are changed in /Users/alex/billing/.worktrees/retry.", "9c1d0aa is not a commit of Billing's repository.", "4f2a91c is already in origin/main: finish the task with finish_task.", finish_task's words for the result, or the tools' words for a task not its own (terminum-agents.md AC-174); nothing is queued
  And a manager queues any task of its project

Scenario: AC-187 edge — its session ends
  Given API-12 in INTEGRATION and its agent's process ends
  Then API-12 stays queued and lands as any; returned, it is IN PROGRESS with no agent, and a human's RUN resumes it (terminum-dispatcher.md AC-122)
```

### AC-188 — A batch [MUST / P0]

```gherkin
Scenario: AC-188 happy path — built
  Given Billing's queue API-12, API-13, API-14, API-15 in the order they were queued, the batch's size 4, no batch under check
  When terminumd's walk comes
  Then it fetches origin, and in the queue's order tries each commit on the candidate so far, by git merge-tree: the first that merges cleanly onto origin/main starts it, each next that merges cleanly onto the result joins, until 4 are in
  And batch 7 is written with its tasks and the target's commit, and the candidate is made in the project's own tree at ~/.terminum/integration/billing: origin/main, then one merge commit a task, "Integrate API-12", in the queue's order: a title, free text, never reaches the published history
  And the tasks left out stay queued in their places

Scenario: AC-188 happy path — its size
  Given Billing's most 8 and the size 4
  When a batch is published green
  Then the next is at most 8; green again, still 8
  When a batch of 8 is red (AC-190)
  Then the next is at most 4; red again 2, then 1; a green one doubles it again
  And the size is the project's and outlives a restart

Scenario: AC-188 happy path — the queue grows meanwhile
  Given batch 7 under check
  When API-16 is queued
  Then it waits: the next batch is built when batch 7 has ended, from all that is queued then

Scenario: AC-188 edge — one candidate at a time on the machine
  Given Billing's batch 7 under check and Web's queue not empty
  Then Web's batch is built and checked after Billing's ended: projects take turns in the order their oldest entry was queued
  And Web's INTEGRATION says "waits for Billing's batch 7"

Scenario: AC-188 edge — nothing fits
  Given every queued commit conflicts with the one before it
  Then each batch holds one task, the first of the queue, and the rest are held back or returned (AC-189): one task a round, one check at a time

Scenario: AC-188 failure — the fetch or the tree
  When origin does not answer, or the candidate's tree cannot be made
  Then no batch is built, the column's head says why in git's words, "origin could not be fetched: <git's message>", and the next walk tries again; the tasks stay queued

Scenario: AC-188 edge — paused
  Given the human pressed Pause (AC-193)
  Then no batch is built, the one under check ends as it would, and tasks still queue
```

### AC-189 — Conflicts [MUST / P0]

```gherkin
Scenario: AC-189 happy path — with the target
  Given API-14's commit does not merge cleanly onto origin/main at 77aa210
  When a batch is built
  Then API-14 is IN PROGRESS at once, told "returned: conflict" with 77aa210 and the files (AC-192), and is no part of the queue

Scenario: AC-189 happy path — with a task of the batch
  Given API-14 merges cleanly onto origin/main, but not onto the candidate that holds API-12
  Then API-14 stays queued in its place, its card says "waits for API-12", and its agent is told once that it is held back, with the files; nothing is asked of it
  And the tasks it waits for are those of the candidate it does not merge with one by one; none found so, "waits for batch 7"

Scenario: AC-189 happy path — after that batch
  When batch 7 is published with API-12
  Then at the next batch API-14 is tried on the new origin/main: it conflicts, and returns as a conflict with the target, its agent able to resolve it
  When batch 7 ends red and API-12 returns to its agent
  Then API-14 is tried again as any queued task and may join the next batch

Scenario: AC-189 edge — a commit that carries another task's
  Given API-13's commit has API-12's queued commit among its ancestors: its agent branched from, or merged, API-12's work
  Then API-13 is never in a batch without API-12: it joins only a candidate that already holds API-12, else it stays queued, "waits for API-12"
  When API-12 returns to its agent or is taken out
  Then API-13 returns too, "returned: it carries API-12's commits, which did not land: merge origin/main, take them out or wait for API-12, and integrate again"
  And this holds at queueing, at each batch and for each half (git merge-base --is-ancestor against every entry and every task returned from the batch)

Scenario: AC-189 edge — a returned task's clocks
  When a task returns to IN PROGRESS for a conflict or a red check
  Then the return counts as activity: its hour with no activity and its requests for a result start anew (terminum-dispatcher.md AC-123), and its budget runs on

Scenario: AC-189 edge — a foreseen conflict
  When a task is queued whose commit does not merge with one already queued
  Then integrate's answer says so at once (AC-187), and the agent may take it out, or leave it to wait

Scenario: AC-189 edge — a merge that fails otherwise
  When git cannot merge for another reason (a missing object, a broken tree)
  Then the task returns with git's words, "returned: git could not merge 4f2a91c: <message>"
```

### AC-190 — The check [MUST / P0]

```gherkin
Scenario: AC-190 happy path — green
  Given batch 7's candidate 3e9b5c2 and Check "make smoke"
  Then a window "Integration: Billing, batch 7" opens in the project's place in terminum's tmux server, in the candidate's tree, and runs the line by the user's login shell with the variables of §9
  And the batch's group on the board says "checking, 12 min", a press showing that window (AC-193)
  When the line ends with status 0
  Then the batch is green and is published (AC-191); terminumd closes the window 5 minutes later, as it closes a check's, a Publish's and an After's, their output kept in the batch's log

Scenario: AC-190 happy path — red, by halves
  Given batch 7 of API-12, API-13, API-15, API-17, and its check ends with status 2
  Then the batch is red and is split in the queue's order: API-12 and API-13 are checked as batch 7a on the same target; green, they are published
  And API-15 and API-17 are batch 7b, built on what 7a published; red, it is split again: 7b1 API-15, 7b2 API-17
  And a task red alone returns to its agent: IN PROGRESS, told "returned: red check" with the last 100 rows of the check, redacted (`sent::redact`), the log's path, the batch's tasks and that it was red alone (AC-192)
  And when 7a returns its tasks instead, 7b is built on the batch's target without them
  And tasks queued meanwhile, and tasks held back for one of batch 7's (AC-189), wait until every half has ended

Scenario: AC-190 edge — a flaky check
  Given batch 7 red, and both its halves green
  Then all its tasks are published, the batch's row says "red together, green by halves", and no task returns

Scenario: AC-190 failure — over its time
  When the check runs past "Check ends after"
  Then its process group is ended (SIGTERM, 10 s later SIGKILL), and the batch is red: "The check ran over 90 minutes."

Scenario: AC-190 failure — it cannot start
  When the line cannot be started (the shell says the command is not found, the tree is gone)
  Then no task returns: the queue is paused, the column's head says "Batch 7's check did not start: <the shell's words>. Set Check in Billing's Integration…, then Resume.", and the banner of waiting agents counts it (terminum-kanban.md AC-117)

Scenario: AC-190 edge — one at a time
  Then at most one check, one Publish or one After of any project runs on the machine at a time

Scenario: AC-190 edge — terminumd restarts
  Given batch 7 under check when terminumd is stopped
  Then the check runs on in its window
  When terminumd starts
  Then it finds the batch's window by its mark: still running, it goes on waiting, the time counted from the check's start; ended, it reads its status and goes on as above
  And a batch whose window is gone is built and checked again, its row saying "checked again: its window was lost"

Scenario: AC-190 edge — no check
  Given an empty Check
  Then no window opens and the candidate is green as built
```

### AC-191 — Publishing [MUST / P0]

```gherkin
Scenario: AC-191 happy path — terminum's push
  Given batch 7 green at 3e9b5c2 on origin/main 77aa210 and an empty Publish
  When terminumd fetches and origin/main is still 77aa210
  Then it runs "git push origin 3e9b5c2:main", never forced, fetches, and finds 3e9b5c2 in origin/main
  And API-12, API-13 and API-15 are DONE, each with its kept result as its comment and "Integrated into origin/main at 3e9b5c2 in batch 7, with API-13 and API-15."; their agents are told (AC-192) and their sessions close as after any DONE (terminum-dispatcher.md AC-125)

Scenario: AC-191 happy path — the project's Publish
  Given Publish "scripts/publish.sh", for a branch that takes changes only by a pull request
  Then it runs in the candidate's tree as the check does; at status 0 terminumd fetches and looks for 3e9b5c2 in origin/main
  When it is there
  Then the batch is published as above
  When it is not, or the line ends with another status
  Then nothing is DONE: the queue is paused, the batch stands "not published" with the line's last rows, and its key Publish again runs it once more

Scenario: AC-191 edge — the target moved
  Given origin/main moved to 8d00e1f while batch 7 was checked: a human pushed
  Then nothing is pushed: the candidate is built again on 8d00e1f and checked again, its row saying "origin/main moved: checked again"; a task that conflicts with the new target returns (AC-189)
  And after 3 such rounds in one batch the column's head says "origin/main keeps moving under batch 7" and it goes on

Scenario: AC-191 failure — the push is refused
  When the push fails and origin/main has not moved
  Then the queue is paused and the batch stands "not published: <git's message>", with Publish again

Scenario: AC-191 happy path — the project's folder
  Given Billing's folder on branch main, clean, behind origin/main
  When batch 7 is published
  Then terminumd fast-forwards it to origin/main (git merge --ff-only)
  When it is on another branch, has uncommitted changes that the move would touch, or has commits of its own
  Then it stays as it is and the batch's row says "Billing's folder stays where it is: <git's words>"

Scenario: AC-191 happy path — After
  Given After "make install"
  Then once the batch is published and the folder moved, the line runs in the project's folder, in a window as the check's, once for the batch
  When it ends with another status, or runs over the check's time
  Then the batch stays published and its tasks DONE; its row and a toast say "After failed for batch 7: <its last rows>", and the next batch goes on
  And with the folder left where it was, After does not run: "After did not run: Billing's folder is not on batch 7."

Scenario: AC-191 edge — terminumd stops while publishing
  When terminumd starts with a batch written green or publishing, and no window of its Publish
  Then it fetches: the candidate in origin/main, it finishes the batch as published; else it publishes as above
  When a window of the project's Publish stands, found by its mark
  Then it waits for it or reads its status, as a check's (AC-190)

Scenario: AC-191 edge — After restarts terminumd
  Given After "make install", which installs and restarts terminumd
  Then the batch was written published and its tasks DONE before After began, and the line runs on in its window
  When terminumd starts
  Then it finds After's window by its mark and waits for it or reads its status; the next batch waits for its end
  And with that window lost, the batch's row says "After's outcome is not known for batch 7" and After is not run again
```

### AC-192 — What agents are told [MUST / P0]

```gherkin
Scenario: AC-192 happy path — an overlap while both work
  Given API-12 and API-13 IN PROGRESS in worktrees of Billing's checkout, each with commits not in origin/main
  When a walk finds that their HEADs do not merge cleanly with each other
  Then each agent is told once for that pair of commits: "[terminum] API-12: its commits and API-13's (session web-2, IN PROGRESS) change the same lines: src/retry.rs 40–58. They cannot be in one batch. Agree with that session who goes first; the other merges origin/main after the first lands, then queues."
  And each card in IN PROGRESS says "overlaps API-13" (AC-193)
  When API-13 is already queued or in a batch
  Then API-12 is told instead "… API-13 is queued for integration (2nd). Go on; merge origin/main once API-13 lands, before you queue."

Scenario: AC-192 happy path — the target moved in its files
  Given API-12 IN PROGRESS, having changed src/retry.rs
  When a batch is published that changed src/retry.rs
  Then API-12's agent is told "[terminum] API-12: origin/main moved to 3e9b5c2 (batch 7: API-13, API-15) in files you changed: src/retry.rs. Merge origin/main now, before you queue." and, when its commits no longer merge with it, "It conflicts in: src/retry.rs 40–58."

Scenario: AC-192 happy path — in the queue
  Then each of these is said once, in the words of §9: queued (place, the batch expected, conflicts foreseen); held back (the task it waits for, the files; "nothing to do until API-12's batch ends"); in a batch (its number, its tasks); returned for a conflict (the target's commit, the files and lines; "merge 77aa210, resolve, commit, integrate again"); returned for a red check (the last rows, the log, the batch's tasks, red alone); taken out (by whom); landed (the commit, the batch)

Scenario: AC-192 happy path — how a notice arrives
  Then every notice is a comment by terminum on its task, read by get_task
  And an agent at work gets it as context with its next hook that carries context (terminum-dispatcher.md AC-126)
  And a caller of "terminum integration wait" gets it printed, and the command's end for a return, a landing or a taking out (AC-187)
  And an idle agent in a session Dispatcher started gets, through the gate, "[terminum Dispatcher] API-12 came back from integration: conflict. Read why by execute_tool("get_task", {id}), fix it and integrate again.", or for a red check "… red check. …" (terminum-dispatcher.md §9 lists them among what the gate types)
  And nothing is typed into a session Dispatcher did not start

Scenario: AC-192 edge — what is compared
  Then only commits are compared, each tree's HEAD against the others' and the target: uncommitted work is not seen, and a notice says "commits"
  And a pair is judged again only when one of its commits or the target changed; trees of the project with no task are compared and named by their session
  And over 30 such trees in a project the newest 30 are compared and the column's head says so

Scenario: AC-192 failure — it cannot be told
  When no hook comes, no command waits and the gate may not type
  Then the comment stands, the card shows it, and a returned task with no live agent is counted by the banner of waiting agents (terminum-kanban.md AC-117)
```

### AC-193 — The board [MUST / P0]

```gherkin
Scenario: AC-193 happy path — the column
  Given Billing with integration on, shown on the board
  Then a sixth column INTEGRATION stands between WAIT FOR HUMAN and FAILED, with its count; a board showing only projects with integration off, and no task there, has none
  And on a phone it is one of the columns stepped through (terminum-kanban.md AC-105)

Scenario: AC-193 happy path — the batch and the queue
  Given batch 7 of API-12, API-13, API-15 under check and API-14, API-16 queued
  Then on top stands one group, framed: its head "Batch 7 · checking · 12 min", then its three cards, each marked "in batch 7"
  And under it the queued cards in the queue's order, each with its place, "1st", or what it waits for, "waits for API-12" (AC-189)
  When the user presses the group's head
  Then the check's window is shown as a session is (terminum.md AC-01)
  And the head says each state in words: "building", "checking", "checking again by halves: 7a", "publishing", "After runs", "not published: <why>", "paused"
  And the column has no sort keys: its order is the queue's

Scenario: AC-193 happy path — keys
  Then the column's head holds Pause, which becomes Resume, and Batches…
  And a card's menu holds Take out (AC-187); a batch that stands "not published" holds Publish again (AC-191)
  When the user drops an IN PROGRESS card on INTEGRATION, or presses Integrate in its menu
  Then its session's folder's HEAD is queued as integrate would, its result "Queued by <the human>: no result from its agent."; refused, a toast says integrate's words
  When the user drops a card from INTEGRATION on another column
  Then it is taken out first, then moved

Scenario: AC-193 happy path — overlaps and returns
  Then an IN PROGRESS card whose commits overlap another task's says "overlaps API-13", a press opening that task
  And a returned task's card says "returned: conflict" or "returned: red check" until its agent queues it again or it leaves IN PROGRESS

Scenario: AC-193 happy path — past batches
  When the user presses Batches…
  Then a window lists the project's batches, newest first, 100 at a time: number, its tasks, the target's commit, the candidate's, its result ("green", "red: API-17 returned", "red together, green by halves", "no check", "not published"), how long its check ran, and its log's key
  And its head says "green 14 of the last 20"

Scenario: AC-193 edge — another project's turn, a pause
  Then a project waiting for another's batch says so in its column's head (AC-188), and a paused queue says "Paused by <who>" or why terminumd paused it (AC-190, AC-191)

Scenario: AC-193 failure — a key refused
  When Pause, Take out or Publish again fails
  Then a toast says terminumd's words where it was pressed and the column shows what holds
```

## 6. Flow and sequence diagrams

```mermaid
sequenceDiagram
  participant A as Agent
  participant T as terminumd
  participant G as git, the project's repository
  participant C as Check's window (tmux)
  A->>T: integrate {id, result}
  T->>G: merge-tree with the target and the queue
  T-->>A: place, expected batch, foreseen conflicts
  loop each walk, no batch under check on the machine
    T->>G: fetch; merge-tree in queue order, up to the size
    T-->>A: returned: conflict with the target / held back for a batch-mate
    T->>G: candidate = target + a merge per task
    T->>C: Check
    alt green and the target has not moved
      T->>G: push candidate to the target, never forced
      T-->>A: landed; task DONE
      T->>C: After, once
    else red, several tasks
      T->>C: halves, the first first
    else red, one task
      T-->>A: returned: red check, its last rows
    end
  end
```

## 7. Current behavior and gaps

| AC | Gap | Evidence |
|---|---|---|
| AC-186 … AC-193 | Not built: each tree lands by itself with the project's own `scripts/land.py` (`terminum-lifecycle.md` AC-104). | `scripts/land.py` |

## 8. Recommendation and ownership

| Part | Owner |
|---|---|
| The queue, batches, git, the check's and After's windows, notices | terminumd |
| The column, the Integration… window, Batches… | Page |
| What a check is, how to publish where a push is not enough, what runs after | the project, in its settings |
| A task's own targeted checks before it queues; resolving conflicts | the task's agent, by its project's rules |

## 9. Technical details and specifications

**Settings.** In the store, per project: `on`, `remote`, `branch`, `check`, `publish`, `after`, `batch_most` (1–8, default 8), `check_minutes` (1–1440, default 90), `size` (the batch's size at this time), `paused`, `paused_why`. Written only by a signed-in page (`PUT /api/task-projects/<id>/integration`), with the checks of a project's other settings; read by pages and by `integration_status`.

**Commands.** Each is one line, run as `<the user's login shell> -lc <line>` in a window terminumd opens in its own tmux server and marks (`@terminum-integration` = batch and step), `remain-on-exit` on; its status is the dead pane's (AS-189), its output is also written to `~/.terminum/integration/<project>/batch-<n>.<step>.log`. The line is the human's, given to tmux as one quoted argument (CLAUDE.md §2). Variables:

| Variable | Value |
|---|---|
| `TM_INTEGRATION_BATCH` | the batch, `7` or a half's `7a` |
| `TM_INTEGRATION_SHA` | the candidate's commit |
| `TM_INTEGRATION_BASE` | the target's commit the candidate stands on |
| `TM_INTEGRATION_TARGET` | `origin/main` |
| `TM_INTEGRATION_TASKS` | the batch's task IDs, spaces between |
| `TM_INTEGRATION_TREE` | the candidate's tree |
| `TM_INTEGRATION_PROJECT_DIR` | the project's folder |

**The three ways to check.** One rule, a line and its exit status: a line that builds and tests in `TM_INTEGRATION_TREE`; a line that pushes `TM_INTEGRATION_SHA` to a branch of its own, waits for the hosting site's pipeline on that commit and exits with its result (the project's script, with the user's own `gh` or `glab` login; terminum holds no token for it); or no line. Because terminum's push publishes that same commit, a pipeline's verdict on it stands for the target.

**The candidate's tree.** `~/.terminum/integration/<project's id>/tree`: a worktree of the project's repository, detached, made once (`git worktree add --detach`) and kept, so a build's caches stay warm; before each batch `git reset --hard <target>` and `git clean -fd` (ignored files, the caches, stay), then `git merge --no-ff --no-verify -m "Integrate <ID>" <commit>` a task: the repository's hooks do not run on the candidate's merges, so what must hold before a push (a scan of what is published) belongs in the project's Check. terminumd writes there and in the project's `.git` only through git; Roots (`TM_WRITABLE_ROOTS`) bound the Files panel's writes, not terminum's own folder.

**Choosing a batch.** In one pass over the queue's entries by `queued_at`, then task number: `git merge-tree --write-tree <candidate so far> <commit>`; a commit with another entry's commit among its ancestors waits for that entry unless the candidate so far holds it (AC-189); status 0 joins, and its written tree's merge becomes the candidate so far; conflicts with the target alone → returned; else held back, its `waits_for` the batch's tasks it does not merge with pairwise. The pass ends at `size` tasks. It is the queue's order, not the largest set that could fit: the same queue gives the same batch.

**Halves.** A red batch of n > 1 is split at ⌈n/2⌉ in the queue's order; the first half is built on the batch's target and checked; the second on what the first published, or on the target when the first returned. A half is a batch of its own with the parent's number and a suffix. `size` halves once for the whole red batch, not for each half.

**Store.** In `kanban.sqlite` of `terminum-kanban.md`:

```sql
CREATE TABLE integration_entries (task TEXT PRIMARY KEY REFERENCES tasks, project TEXT NOT NULL REFERENCES projects, commit_sha TEXT NOT NULL,
  result TEXT NOT NULL, queued_at INTEGER NOT NULL, batch INTEGER REFERENCES integration_batches, waits_for TEXT, told TEXT NOT NULL DEFAULT '');
CREATE TABLE integration_batches (id INTEGER PRIMARY KEY, project TEXT NOT NULL REFERENCES projects, number INTEGER NOT NULL, half TEXT NOT NULL DEFAULT '',
  tasks TEXT NOT NULL, base_sha TEXT NOT NULL, candidate_sha TEXT, state TEXT NOT NULL, why TEXT, check_started INTEGER, check_ended INTEGER,
  rounds INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, ended_at INTEGER);
```

A task's status `INTEGRATION` is the one fact that it is queued; its entry holds with what. `state`: `building`, `checking`, `green`, `publishing`, `published`, `after`, `red`, `split`, `not published`. `told` holds the notices given, so each is said once.

**Notices.** Fixed words with names filled in; a file's lines are those git's conflict names in the merge's written tree.

| Notice | Words |
|---|---|
| overlap | "API-12: its commits and API-13's (session web-2, IN PROGRESS) change the same lines: <files and lines>. They cannot be in one batch. Agree with that session who goes first; the other merges origin/main after the first lands, then queues." |
| overlap, the other queued | "… API-13 is queued for integration (2nd). Go on; merge origin/main once API-13 lands, before you queue." |
| target moved | "API-12: origin/main moved to <sha> (batch 7: <tasks>) in files you changed: <files>. Merge origin/main now, before you queue." |
| queued | "API-12 is queued 3rd for integration into origin/main. Expected batch: with API-9 and API-10, after batch 7 ends. Conflicts foreseen: none." |
| held back | "API-12 is held back: it does not merge with API-9 of batch 8 in <files and lines>. Nothing to do until API-9's batch ends; it is judged again then." |
| in a batch | "API-12 is in batch 8 with API-9 and API-10: its check runs." |
| returned, conflict | "API-12 returned: conflict with origin/main at <sha> in <files and lines>. Merge <sha>, resolve, commit, and integrate again." |
| returned, carried | "API-13 returned: it carries API-12's commits, which did not land. Merge origin/main, take them out or wait for API-12, and integrate again." |
| returned, red | "API-12 returned: red check, alone on origin/main at <sha>. Its last rows are below; the whole log: <path>. Fix it, commit, and integrate again." |
| taken out | "API-12 was taken out of the queue by <who>." |
| landed | "API-12 is integrated into origin/main at <sha> in batch 8, with API-9 and API-10. It is DONE." |

**Tools** (`terminum-agents.md` AC-155's catalog), a worker's for its run's tasks, a manager's for its project's:

| Tool | Does |
|---|---|
| `integrate {id? , title?, text?, severity?, commit?, result}` | queues, AC-187 |
| `withdraw_integration {id}` | takes out |
| `integration_status {id?}` | the project's settings, the batch under check, the queue with places and waits, the last 10 batches; with an id, that task's entry and notices |

**Command.** `terminum integration queue (<ID> | --title <words>) [--commit <sha>] [--result-file <path>] [--wait]`, `terminum integration wait <ID>`, `terminum integration status [<ID>]`, over terminumd's socket; exits as AC-187.

**Routes.**

```text
GET  /api/task-projects/<id>/integration → {"on", "remote", "branch", "check", "publish", "after", "batch_most", "check_minutes", "size", "paused", "paused_why", "runs": {"tree", "check", "publish", "after"}}
PUT  /api/task-projects/<id>/integration {same but size, runs} → 200 or 400 with the field's words
GET  /api/integration?project=<id> → {"batch": {"number", "half", "state", "why", "tasks", "base", "candidate", "check_started", "window"}, "queue": [{"task", "place", "commit", "waits_for"}], "waits_project": "Billing's batch 7"}
POST /api/integration/<project>/pause | resume | publish-again
POST /api/tasks/<id>/integrate {commit?} | withdraw
GET  /api/integration-batches?project=<id>&before=<n> → {"batches": […100], "more", "green": 14, "of": 20}
```

Every page hears `{"t": "integration"}` when a batch or the queue changes.

**This repository's own settings.** terminum checks on this machine (the first way):

| Field | Value |
|---|---|
| On | on |
| Target | `origin/main` |
| Check | `python3 scripts/check_changed.py --land` |
| Publish | empty: terminum's push |
| After | `make build install installed` |
| Tasks in a batch, at most | 8 |
| Check ends after, minutes | 90 |

`make land SLUG=<slug>` then runs the tree's targeted pass (`terminum-lifecycle.md` AC-104), queues its HEAD by `terminum integration queue --wait`, with the ID of the task its session works on, else `--title` with the tree's branch, and ends with its exit status: 0 landed, 1 red, 2 conflict. The values are set in the project's window on the machine that works on terminum; they are no file of the repository.

**Security.** The three lines are a standing permission to run commands and to push as the user: only a signed-in page writes them, the window shows each line and its folder before Save, and no tool of an agent changes them. An agent chooses only which commit of the project's repository is queued; no title or text of a task reaches a commit, a command or a shell: a merge is named by the task's ID alone. The check is the candidate's own, so a task can change what checks it, as a tree that lands by itself can: the human reads Batches… and the published history. terminum's own push names one remote, one branch and one commit, never forced, never a tag; a project's Publish is the human's line. A check runs the queued code as the user, as the agents that wrote it already do. A check's rows reach a task's comment only redacted (`sent::redact`). The gate types only a task's ID with fixed words, only into a pane Dispatcher started and marked.

**Limits.** 200 queued tasks a project; a batch at most 8 tasks; 30 trees compared for overlaps a project; a command 2,000 characters; a check 1 to 1440 minutes; batches' rows kept, their logs 30 days; one check, Publish or After on the machine at a time.

## 10. Delivery stages

| Stage | Cases | What |
|---|---|---|
| Stage 1 | AC-186, AC-187, AC-188, AC-189, AC-190, AC-191 | The settings, the queue, batches by halves, publishing; notices as comments and by the waiting command; the column as a plain list. Ends when, in the commit that brings the push: CLAUDE.md §9 (the one standing permission to push, who moves the primary checkout) and SECURITY.md say the queue, after an independent review; AS-187 and AS-189 are measured; this repository's settings are set and `make land` queues. |
| Stage 2 | AC-192, AC-193 | Overlaps while tasks are in work, notices by hooks and the gate, the batch's group, keys and Batches…. |

## 11. Checklist with Definition of Done

- [ ] AC-186 — `cargo test integration::settings` (each refusal's words, off runs no git), `web/test/e2e_liveintegration.mjs` (the window, What runs, the three ways on a private repository with a stand-in pipeline script)
- [ ] AC-187 — `cargo test integration::queue` (queued, again, taken out, each refusal, a task made), `cargo test cli::integration` (the wait's exit statuses), `cargo test dispatcher` (no budget, no idle clock, no closing in INTEGRATION)
- [ ] AC-188 — `cargo test integration::batch` on private repositories (`crate::testutil::temp_root()`): the queue's order, the size's doubling and halving, two projects' turns, everything conflicting gives one task a round
- [ ] AC-189 — `cargo test integration::conflicts` (with the target, with a batch-mate, judged again after that batch, a foreseen one)
- [ ] AC-190 — `cargo test integration::check` with stand-in lines (green, red by halves, a flaky one, over its time, not started), `scripts/e2e_integration.py` (a check outlives a restart of a private terminumd, private tmux)
- [ ] AC-191 — `cargo test integration::publish` against a private bare remote (pushed never forced, the target moved, a refused push, the project's Publish that does not land, the folder left where it is, After's failure undoes nothing)
- [ ] AC-192 — `cargo test integration::notices` (each notice's words, said once, what is compared), `scripts/e2e_integration.py` (a hook's context, the gate only in Dispatcher's pane), `web/test/e2e_liveintegration.mjs` (the comment on the card)
- [ ] AC-193 — `web/test/e2e_liveintegration.mjs` (Chromium and WebKit, wide and phone: the column only where on, the batch's group, places and waits, each key and its refusal, Batches…)
