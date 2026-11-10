// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What a walk does with one run (`terminum-dispatcher.md` AC-123, AC-124, AC-125): a pure rule over
//! the run's clocks and what was just read, so every case is a test with no tmux and no waiting.

use super::runs::{Phase, Run};

/// WAIT FOR HUMAN after an agent's question stands this long.
pub const WAIT: i64 = 120;
/// The request after a turn with no result.
pub const NUDGE: i64 = 300;
/// FAILED, with no result, after this long with no activity: counted from that request once it was made.
pub const NOTHING: i64 = 3600;
/// A session closes this long after DONE or FAILED.
pub const CLOSE: i64 = 300;

/// Quiet this long after an agent asked a human before its session closes (AC-157): as an idle holder's.
pub const ASKED_QUIET: i64 = IDLE_QUIET;

/// A subagent silent and idle this long gets the main agent one request, then what can be done (AC-127).
pub const SUB_TELL: i64 = 600;
pub const SUB_END: i64 = 1200;

/// What a walk's reading of a session's memory says, by two walks in a row (AC-128).
#[derive(Clone, Debug, PartialEq)]
pub struct Memory {
    pub gib: f64,
    pub end_gib: f64,
    pub warn_due: bool,
    pub end_due: bool,
}

impl Memory {
    /// This walk's reading against the limits, 0 being off; `over`: the walks in a row each was passed.
    pub fn read(gib: f64, warn_gib: f64, end_gib: f64, over: &mut (u32, u32)) -> Memory {
        over.0 = if warn_gib > 0.0 && gib > warn_gib { over.0 + 1 } else { 0 };
        over.1 = if end_gib > 0.0 && gib > end_gib { over.1 + 1 } else { 0 };
        Memory { gib, end_gib, warn_due: over.0 >= 2, end_due: over.1 >= 2 }
    }
}

/// What this walk read of the run's pane.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    /// the pane still stands, and carries this run's mark
    pub pane: bool,
    pub marked: bool,
    /// what the window command recorded
    pub exit: Option<i32>,
    /// the agent's process still runs under the pane's shell
    pub alive: bool,
    /// its agent waits for an answer: what it asks
    pub waiting: Option<String>,
    /// its turn failed, as the list says: why
    pub failed: Option<String>,
    /// activity since the last walk (§9)
    pub active: bool,
    /// a program other than the agent and its shell runs in the pane
    pub program: Option<String>,
    /// another of the session's tasks is not DONE or FAILED
    pub others_open: bool,
    /// a page in front shows the pane: a finished session is not closed under its reader (AC-125)
    pub watched: bool,
    /// a subagent of it is known to run (AC-127)
    pub subs: bool,
    /// its tree's memory, if `ps` said
    pub memory: Option<Memory>,
    /// seconds to add to the clocks: none over a gap (AC-131)
    pub dt: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// Moves the task, with the words on its history.
    Move { to: &'static str, words: String },
    /// Says this on the task.
    Say(String),
    /// Asks the agent once for its result, through the gate.
    Nudge,
    /// Gives the agent the last warning: seconds left of the budget (AC-126).
    Warn(i64),
    /// Ends the agent: the task was moved first, so its end counts for nothing (AC-123).
    End,
    /// Tells the agent how much memory its session uses, once (AC-128).
    MemoryWarn(f64),
    /// Closes the pane; `keep` keeps the screen's last rows on the task first. The run is closed when tmux did it.
    Close { words: String, keep: bool },
}

/// The one request for a result (AC-123): fixed words and the task's ID, typed through the gate (§9).
pub fn nudge_words(task: &str) -> String {
    format!("[terminum Dispatcher] {task} is still open and you stopped. Is it finished? Call {} with its result in the sections you were asked for, or ask a human by {} what you need; with neither, it is FAILED after an hour.", super::press::FINISH, super::press::ASK_HUMAN)
}

/// When the last warning goes: 3% of the budget left, at least 2 minutes (AC-126).
pub fn warn_at(total: i64) -> i64 {
    (total * 3 / 100).max(120)
}

pub fn finished(status: &str) -> bool {
    matches!(status, "DONE" | "FAILED")
}

/// Whether the last warning is due: what is left of the budget is within `warn_at`.
pub fn warn_due(run: &Run) -> bool {
    run.budget.is_some_and(|total| total - run.budget_used <= warn_at(total))
}

/// The task fails first, so the end Dispatcher causes counts for nothing; then the agent ends. Ending until it is
/// done: a restart does it again, new work or Give up calls it off (AC-126).
fn end_failed(run: &mut Run, acts: &mut Vec<Act>, words: String) {
    run.state = Phase::Ending;
    acts.push(Act::Move { to: "FAILED", words });
    acts.push(Act::End);
}

/// The walk's step for `run`: its clocks change here, what is to be done comes back.
pub fn step(run: &mut Run, f: &Facts) -> Vec<Act> {
    let mut acts = Vec::new();
    let movable = !finished(&run.status) && !run.ad_hoc();     // + Session's has no task to move
    if !f.pane || !f.marked {
        run.state = Phase::Closed;
        if f.pane {
            run.pane_key = None;
        }
        let words = if f.pane { "Its pane is no longer this run's: the link is dropped." } else { "Its window was closed." };
        if movable {
            acts.push(Act::Move { to: "FAILED", words: words.to_string() });
        } else {
            acts.push(Act::Say(format!("Session ended: {}", words.to_lowercase())));
        }
        return acts;
    }
    // an End decided and not done (a restart, or a human's keys put it off): again; done, its task was moved first
    if run.state == Phase::Ending {
        if f.alive {
            acts.push(Act::End);
            return acts;
        }
        run.gone();
    }
    // the agent's end: its exit code tells a clean one from a signal (AS-127)
    if run.state != Phase::Gone && !f.alive {
        run.gone();
        let how = match f.exit {
            Some(code) => format!("exit {code}"),
            None => "no exit recorded".to_string(),
        };
        if movable {
            // no result can be asked for any more (AC-123)
            let words = if f.exit == Some(0) { "The session ended without a result.".to_string() } else { format!("The agent's process ended: {how}.") };
            acts.push(Act::Move { to: "FAILED", words });
        } else {
            acts.push(Act::Say(format!("Session ended: {} ended, {how}.", run.kind)));
        }
    }
    // + Session's: the human's alone; only its end and its window are Dispatcher's (AC-142)
    if run.ad_hoc() {
        return acts;
    }
    let alive = run.state != Phase::Gone;
    if alive && movable && let Some(why) = &f.failed {
        acts.push(Act::Move { to: "FAILED", words: format!("The agent's turn failed: {why}") });
        return acts;
    }
    // a wait for an answer (AC-124)
    match (&f.waiting, alive) {
        (Some(why), true) => {
            run.waited_for += f.dt;
            if run.waited_for >= WAIT && run.status == "IN PROGRESS" {
                run.wait_moved = true;
                acts.push(Act::Move { to: "WAIT FOR HUMAN", words: if why.is_empty() { "It asks in its window".into() } else { why.clone() } });
            }
        }
        _ => {
            run.waited_for = 0;
            if run.wait_moved && run.status == "WAIT FOR HUMAN" && alive && f.active {
                run.wait_moved = false;
                acts.push(Act::Move { to: "IN PROGRESS", words: "The agent went on.".into() });
            }
        }
    }
    // no activity (AC-123); an answer waited for has no idle clock
    if f.active || !alive {
        run.idle_for = 0;
    } else if f.waiting.is_none() {
        run.idle_for += f.dt;
    }
    // no result: one request once its turn is over, then an hour of nothing is FAILED, never DONE (AC-123)
    if alive && f.waiting.is_none() && run.status == "IN PROGRESS" {
        // `nudged` is written once it went (acts.rs): a restart before then asks again
        if run.turn_ended && run.idle_for >= NUDGE && !run.nudged {
            run.nudge_turn = true;
            acts.push(Act::Nudge);
        } else if run.nudged && run.idle_for >= NUDGE + NOTHING {
            acts.push(Act::Move { to: "FAILED", words: "No result from the agent within an hour of Dispatcher's request.".into() });
        } else if !run.nudged && run.idle_for >= NOTHING {
            acts.push(Act::Move { to: "FAILED", words: "No activity for 60 minutes and no result from the agent.".into() });
        }
    }
    // the agent asked a human itself (ask_human, move_task): once its turn is over and quiet, the session
    // closes and RUN resumes its conversation (AC-157)
    if alive && asked_human(run) && f.waiting.is_none() {
        if f.active || !run.turn_ended || run.open_tools > 0 || f.subs {
            run.closing_for = 0;
        } else {
            run.closing_for += f.dt;
        }
        if run.closing_for >= ASKED_QUIET && !f.others_open && f.program.is_none() {
            acts.push(Act::Close { words: "Session closed: it waits for the human. RUN resumes this conversation.".into(), keep: false });
        }
    } else if !finished(&run.status) {
        run.closing_for = 0;
    }
    // the budget: only IN PROGRESS counts, never a wait for an answer (AC-126)
    if let Some(total) = run.budget.filter(|_| alive && run.status == "IN PROGRESS") {
        if f.waiting.is_none() {
            run.budget_used += f.dt;
        }
        let left = total - run.budget_used;
        if left <= 0 {
            if !run.warned {
                acts.push(Act::Say("No warning could be given before the limit.".into()));
            }
            end_failed(run, &mut acts, format!("Time budget of {} minutes used: the agent was ended.", total / 60));
            return acts;
        }
        if !run.warned && warn_due(run) {
            acts.push(Act::Warn(left));               // `warned` once it went (acts.rs)
        }
    }
    // what it consumes: two walks in a row, as summed memory counts shared pages many times (AC-128)
    if let Some(m) = f.memory.as_ref().filter(|_| alive && !finished(&run.status)) {
        if m.end_due {
            end_failed(run, &mut acts, format!("Memory limit of {} GiB exceeded: {:.1} GiB", m.end_gib, m.gib));
            return acts;
        }
        if m.warn_due && !run.memory_warned {
            acts.push(Act::MemoryWarn(m.gib));
        }
    }
    // closing after DONE or FAILED (AC-125): any activity starts the 5 minutes again, a human's at its
    // shell too; only an agent ended, or its turn over, is closed under no one
    if finished(&run.status) {
        if f.active {
            if !run.again_said && run.closing_for > 0 {
                run.again_said = true;
                acts.push(Act::Say("The 5 minutes before closing start again: there is activity in the session.".into()));
            }
            run.closing_for = 0;
        } else if f.watched {
            run.closing_for = 0;
        } else {
            run.closing_for += f.dt;
        }
        let idle = f.exit.is_some() || (f.alive && run.turn_ended);
        if run.closing_for >= super::close_after() && !f.others_open && f.waiting.is_none() && idle {
            if let Some(program) = &f.program {
                run.state = Phase::Left;
                acts.push(Act::Say(format!("Session left open: {program} runs in it")));
            } else {
                acts.push(Act::Close { words: format!("Session ended: closed 5 minutes after {}.", run.status), keep: !run.result });
            }
        }
    }
    acts
}

/// Whether its tasks wait for a human by the agent's own word, in a session whose conversation is known: closing it
/// loses nothing RUN cannot resume (AC-157).
pub fn asked_human(run: &Run) -> bool {
    run.asked && run.status == "WAIT FOR HUMAN" && run.state == Phase::Working && !run.ad_hoc() && run.agent_session.is_some()
}

/// Whether a close of either kind may come once `dt` more passes: then the walk reads whether another task of
/// its session is open (AC-125, AC-157).
pub fn close_near(run: &Run, dt: i64) -> bool {
    (finished(&run.status) && run.closing_for + dt >= super::close_after()) || (asked_human(run) && run.closing_for + dt >= ASKED_QUIET)
}

/// Whether a Close for an agent's question still holds, read just before the pane goes.
pub fn asked_closes(run: &Run) -> bool {
    asked_human(run) && run.turn_ended && run.open_tools == 0 && run.closing_for >= ASKED_QUIET
}

/// Quiet this long, two walks' worth, before an idle session can be closed for a slot (AC-141).
const IDLE_QUIET: i64 = 60;

/// Whether a session can be closed for a slot, by what the store knows: its tasks ended, its turn over, no tool open and
/// two walks quiet; what the pane shows is read just before (AC-141).
pub fn idle_holder(run: &Run) -> bool {
    run.state == Phase::Working && finished(&run.status) && run.open.is_empty() && run.turn_ended && run.open_tools == 0 && run.closing_for >= IDLE_QUIET
}

/// Whether a Close a walk decided still holds, read just before the pane goes: a press or a prompt may have come since.
pub fn still_closes(run: &Run) -> bool {
    finished(&run.status) && !run.is_over() && !run.busy() && run.closing_for >= super::close_after()
}

/// How far a subagent has been dealt with, by how long it has been silent and idle (AC-127). A busy
/// tree, or an open tool, runs neither clock: the caller does not ask.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub enum SubStep {
    #[default]
    Nothing,
    Tell,
    Fail,
}

/// The step `silent` seconds reach; it is done once, when it passes what was `told`.
pub fn subagent_step(silent: i64, told: SubStep) -> Option<SubStep> {
    let reached = if silent >= SUB_END { SubStep::Fail } else if silent >= SUB_TELL { SubStep::Tell } else { SubStep::Nothing };
    (reached > told).then_some(reached)
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
