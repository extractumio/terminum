// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;

fn run(status: &str) -> Run {
    Run { task: "k12".into(), status: status.into(), state: Phase::Working, kind: "claude".into(), ids: vec!["T-12".into()], open: if crate::dispatcher::plan::finished(status) { vec![] } else { vec!["T-12".into()] }, ..Default::default() }
}

fn up(dt: i64) -> Facts {
    Facts { pane: true, marked: true, alive: true, dt, ..Default::default() }
}

fn moved(acts: &[Act]) -> Option<(&'static str, String)> {
    acts.iter().find_map(|a| match a { Act::Move { to, words } => Some((*to, words.clone())), _ => None })
}

#[test]
fn ac123_a_turn_with_no_result_gets_a_request_then_an_hour_then_failed() {
    let mut r = run("IN PROGRESS");
    r.turn_ended = true;
    assert!(step(&mut r, &up(299)).is_empty());
    assert_eq!(step(&mut r, &up(1)), vec![Act::Nudge]);
    assert!(r.nudged == 0 && r.nudge_turn, "`nudged` is written once it went (acts.rs)");
    assert_eq!(step(&mut r, &up(30)), vec![Act::Nudge], "not gone yet (a restart came between): asked again");
    (r.nudged, r.idle_for) = (1, 0);                    // as acts.rs writes it once it went
    assert!(step(&mut r, &up(300)).is_empty(), "its turn has not ended: no second request");
    // the turn's tokens come after its Stop, from the same hook, and a closed pane drops them (sock.rs, AC-129):
    // up to the hour nothing closes or ends it, however late the agent's next event is
    let acts = step(&mut r, &up(3600 - 301));
    assert!(!acts.iter().any(|a| matches!(a, Act::Close { .. } | Act::End)), "{acts:?}");
    assert_eq!(moved(&acts), None, "no WAIT FOR HUMAN and no DONE before the hour after the request");
    let (to, words) = moved(&step(&mut r, &up(1))).unwrap();
    assert_eq!((to, words.as_str()), ("FAILED", "No result from the agent within an hour of Dispatcher's request."));
}

#[test]
fn ac123_the_request_asks_whether_it_is_finished_and_says_the_hour() {
    let w = nudge_words("API-12");
    assert!(w.starts_with("[terminum Dispatcher] API-12 is still open and your turn ended; no human watches this run now. If it is finished, call execute_tool(\"finish_task\", {id, result}) with its result in the sections"), "{w}");
    assert!(w.contains("If a human must decide or approve something, ask by execute_tool(\"ask_human\", {id, questions})"), "a decision or an approval goes to the human, never to Dispatcher: {w}");
    assert!(w.contains("update_session_plan") && w.ends_with("it is FAILED after an hour."), "{w}");
    assert!(!w.chars().any(char::is_control), "typed through the gate: one line, no control characters");
}

#[test]
fn ac123_activity_after_the_request_starts_its_hour_anew() {
    let mut r = run("IN PROGRESS");
    r.turn_ended = true;
    r.nudged = 1;
    step(&mut r, &up(3000));
    step(&mut r, &Facts { active: true, ..up(30) });
    assert_eq!(moved(&step(&mut r, &up(3599))), None, "the agent worked after the request: its hour counts from then");
    assert_eq!(moved(&step(&mut r, &up(1))).unwrap().0, "FAILED");
}

#[test]
fn ac123_activity_starts_the_idle_clock_anew() {
    let mut r = run("IN PROGRESS");
    r.turn_ended = true;
    step(&mut r, &up(299));
    step(&mut r, &Facts { active: true, ..up(30) });
    assert_eq!(r.idle_for, 0);
    assert!(step(&mut r, &up(299)).is_empty());
}

#[test]
fn ac123_an_hour_of_nothing_is_failed_never_done() {
    let mut r = run("IN PROGRESS");
    assert_eq!(moved(&step(&mut r, &up(3599))), None);
    let (to, words) = moved(&step(&mut r, &up(1))).unwrap();
    assert_eq!((to, words.as_str()), ("FAILED", "No activity for 60 minutes and no result from the agent."));
    // TERM-58: a main task with its subtasks open is the same case, and a task in WAIT FOR HUMAN waits for the human
    let mut asked = run("WAIT FOR HUMAN");
    asked.asked = true;
    assert_eq!(moved(&step(&mut asked, &up(7200))), None, "WAIT FOR HUMAN is never moved on by a clock");
}

#[test]
fn ac123_the_exit_code_decides_a_clean_end_from_a_signal() {
    let mut r = run("IN PROGRESS");
    let (to, words) = moved(&step(&mut r, &Facts { exit: Some(0), alive: false, ..up(0) })).unwrap();
    assert_eq!((to, words.as_str()), ("FAILED", "The session ended without a result."), "no result can be asked for any more");
    assert_eq!(r.state, Phase::Gone);
    let mut r = run("IN PROGRESS");
    let (to, words) = moved(&step(&mut r, &Facts { exit: Some(143), alive: false, ..up(0) })).unwrap();
    assert_eq!((to, words.as_str()), ("FAILED", "The agent's process ended: exit 143."));
    let mut r = run("IN PROGRESS");
    assert_eq!(moved(&step(&mut r, &Facts { alive: false, ..up(0) })).unwrap().0, "FAILED", "dead with no exit recorded");
    assert_eq!(step(&mut r, &Facts { alive: false, ..up(30) }), vec![], "said once");
}

#[test]
fn ac123_a_failed_turn_is_failed_with_its_reason() {
    let mut r = run("IN PROGRESS");
    let acts = step(&mut r, &Facts { failed: Some("rate_limit".into()), ..up(0) });
    assert_eq!(moved(&acts).unwrap(), ("FAILED", "The agent's turn failed: rate_limit".to_string()));
}

#[test]
fn ac124_two_minutes_of_a_question_wait_for_a_human_and_it_returns() {
    let mut r = run("IN PROGRESS");
    let asks = Facts { waiting: Some("Approve Bash".into()), ..up(90) };
    assert!(step(&mut r, &asks).is_empty(), "a short wait moves nothing");
    let acts = step(&mut r, &Facts { dt: 30, ..asks.clone() });
    assert_eq!(moved(&acts).unwrap(), ("WAIT FOR HUMAN", "Approve Bash".to_string()));
    r.status = "WAIT FOR HUMAN".into();
    assert!(step(&mut r, &Facts { dt: 86400, ..asks.clone() }).is_empty(), "no time limit on a wait");
    assert_eq!(r.idle_for, 0, "no idle clock while it waits");
    let acts = step(&mut r, &Facts { active: true, ..up(30) });
    assert_eq!(moved(&acts).unwrap().0, "IN PROGRESS");
}

#[test]
fn ac125_it_closes_five_minutes_after_the_end_only_when_idle() {
    let ended = |dt| Facts { exit: Some(0), alive: false, dt, ..up(0) };
    let mut r = run("DONE");
    r.state = Phase::Gone;
    assert!(step(&mut r, &ended(299)).is_empty());
    let acts = step(&mut r, &ended(1));
    assert_eq!(acts, vec![Act::Close { words: "Session ended: closed 5 minutes after DONE.".into(), keep: true }]);
    assert_eq!(r.state, Phase::Gone, "closed only when tmux did it");
    let mut r = run("DONE");
    r.result = true;
    r.turn_ended = true;
    let acts = step(&mut r, &up(300));
    assert!(matches!(&acts[0], Act::Close { keep: false, .. }), "with a result no screen is kept");
}

#[test]
fn ac125_activity_starts_the_five_minutes_again_said_once() {
    let mut r = run("DONE");
    step(&mut r, &up(270));
    let acts = step(&mut r, &Facts { active: true, ..up(30) });
    assert_eq!(acts, vec![Act::Say("The 5 minutes before closing start again: there is activity in the session.".into())]);
    assert_eq!(r.closing_for, 0);
    step(&mut r, &up(30));
    assert!(step(&mut r, &Facts { active: true, ..up(30) }).is_empty(), "once");
}

#[test]
fn ac125_a_human_at_the_shell_keeps_it_open() {
    let mut r = run("DONE");
    r.state = Phase::Gone;
    let shell = |dt, active| Facts { exit: Some(0), alive: false, active, dt, ..up(0) };
    step(&mut r, &shell(270, false));
    step(&mut r, &shell(30, true));
    assert_eq!(r.closing_for, 0, "their keys start the clock again");
    assert!(step(&mut r, &shell(299, false)).is_empty());
}

#[test]
fn ac125_a_session_someone_reads_is_not_closed_under_them() {
    let mut r = run("DONE");
    r.turn_ended = true;
    let read = |dt| Facts { watched: true, ..up(dt) };
    assert!(step(&mut r, &read(9000)).is_empty(), "shown in a page in front: no close, and nothing said");
    assert_eq!(r.closing_for, 0);
    assert!(step(&mut r, &up(299)).is_empty(), "the 5 minutes start when they leave");
    assert!(matches!(&step(&mut r, &up(1))[0], Act::Close { .. }));
}

#[test]
fn ac125_an_agent_still_working_is_never_closed() {
    // its start failed by our reading (a Codex with hooks not trusted): it runs on, no turn's end heard
    let mut r = run("FAILED");
    r.state = Phase::Gone;
    assert!(step(&mut r, &up(9000)).is_empty());
    r.turn_ended = true;
    assert!(matches!(&step(&mut r, &up(30))[0], Act::Close { .. }), "once its turn is over");
}

#[test]
fn ac125_other_tasks_and_a_human_s_program_keep_it_open() {
    let mut r = run("DONE");
    r.turn_ended = true;
    assert!(step(&mut r, &Facts { others_open: true, ..up(9000) }).is_empty());
    let acts = step(&mut r, &Facts { program: Some("vim".into()), ..up(30) });
    assert_eq!(acts, vec![Act::Say("Session left open: vim runs in it".into())]);
    assert_eq!(r.state, Phase::Left);
}

#[test]
fn ac125_a_pane_not_this_run_s_is_never_acted_on() {
    let mut r = run("IN PROGRESS");
    let acts = step(&mut r, &Facts { pane: true, marked: false, alive: true, dt: 30, ..Default::default() });
    assert_eq!(moved(&acts).unwrap(), ("FAILED", "Its pane is no longer this run's: the link is dropped.".to_string()));
    assert_eq!((r.state, r.pane_key), (Phase::Closed, None));
    let mut gone = run("IN PROGRESS");
    gone.pane_key = Some("tmux:local:s:%1".into());
    step(&mut gone, &Facts::default());
    assert_eq!(gone.pane_key.as_deref(), Some("tmux:local:s:%1"), "a closed window's key stays: RUN goes on by the resume");
    let mut r = run("DONE");
    assert!(matches!(&step(&mut r, &Facts::default())[0], Act::Say(_)), "a closed window of a finished task is only said");
}

#[test]
fn ac131_no_clock_gains_a_gap() {
    let mut r = run("IN PROGRESS");
    r.turn_ended = true;
    step(&mut r, &up(0));
    assert_eq!(r.idle_for, 0);
    let mut d = run("DONE");
    step(&mut d, &up(0));
    assert_eq!(d.closing_for, 0);
}

#[test]
fn ac126_only_in_progress_counts_and_never_a_wait() {
    let mut r = run("IN PROGRESS");
    r.budget = Some(3600);
    step(&mut r, &up(600));
    assert_eq!(r.budget_used, 600);
    step(&mut r, &Facts { waiting: Some("Approve".into()), ..up(600) });
    assert_eq!(r.budget_used, 600, "a wait for an answer stops it");
    let mut held = run("WAIT FOR HUMAN");
    held.budget = Some(3600);
    step(&mut held, &up(600));
    assert_eq!(held.budget_used, 0, "outside IN PROGRESS it does not count");
    let mut slept = run("IN PROGRESS");
    slept.budget = Some(3600);
    step(&mut slept, &up(0));
    assert_eq!(slept.budget_used, 0, "a gap adds nothing (AC-131)");
}

#[test]
fn ac126_the_last_warning_is_three_percent_at_least_two_minutes() {
    assert_eq!(warn_at(3600), 120);
    assert_eq!(warn_at(12000), 360);
    let mut r = run("IN PROGRESS");
    r.budget = Some(3600);
    r.budget_used = 3400;
    assert!(step(&mut r, &up(70)).is_empty());
    assert_eq!(step(&mut r, &up(10)), vec![Act::Warn(120)]);
    assert!(!r.warned && warn_due(&r), "`warned` once it went (acts.rs)");
    assert_eq!(step(&mut r, &up(10)), vec![Act::Warn(110)], "lost before it went: given again");
    r.warned = true;
    assert!(step(&mut r, &up(10)).is_empty(), "once");
    r.new_work();
    assert!(!warn_due(&r), "new work: its budget starts anew, a warning written late would not hold");
}

#[test]
fn ac126_at_the_limit_the_task_fails_first_then_the_agent_ends() {
    let mut r = run("IN PROGRESS");
    r.budget = Some(3600);
    r.budget_used = 3590;
    r.warned = true;
    let acts = step(&mut r, &up(30));
    assert_eq!(acts, vec![Act::Move { to: "FAILED", words: "Time budget of 60 minutes used: the agent was ended.".into() }, Act::End]);
    assert_eq!(r.state, Phase::Ending, "until it is done");
    r.status = "FAILED".into();
    assert_eq!(step(&mut r, &up(30)), vec![Act::End], "not done yet (a restart, a human's keys put it off): again");
    assert_eq!((r.state, r.budget_used), (Phase::Ending, 3620), "no clock runs meanwhile");
    let acts = step(&mut r, &Facts { exit: Some(143), alive: false, ..up(30) });
    assert_eq!(moved(&acts), None, "the end Dispatcher caused counts for nothing");
    assert_eq!(r.state, Phase::Gone);
}

#[test]
fn ac126_no_budget_no_clock() {
    let mut r = run("IN PROGRESS");
    step(&mut r, &up(600));
    assert_eq!(r.budget_used, 0);
}

fn walks(gib: f64, count: &mut (u32, u32)) -> Option<Memory> {
    Some(Memory::read(gib, 4.0, 8.0, count))
}

#[test]
fn ac128_memory_warns_once_after_two_walks_and_ends_after_two() {
    let (mut r, mut count) = (run("IN PROGRESS"), (0, 0));
    assert!(step(&mut r, &Facts { memory: walks(5.0, &mut count), ..up(30) }).is_empty(), "one walk is not enough");
    assert_eq!(step(&mut r, &Facts { memory: walks(5.1, &mut count), ..up(30) }), vec![Act::MemoryWarn(5.1)]);
    r.memory_warned = true;                            // as acts.rs writes it once it went
    assert!(step(&mut r, &Facts { memory: walks(5.1, &mut count), ..up(30) }).is_empty(), "once");
    assert!(step(&mut r, &Facts { memory: walks(9.2, &mut count), ..up(30) }).is_empty());
    let acts = step(&mut r, &Facts { memory: walks(9.2, &mut count), ..up(30) });
    assert_eq!(acts, vec![Act::Move { to: "FAILED", words: "Memory limit of 8 GiB exceeded: 9.2 GiB".into() }, Act::End]);
}

#[test]
fn ac128_a_limit_of_zero_is_off_and_a_dip_starts_the_count_again() {
    let mut count = (5, 5);
    let m = Memory::read(99.0, 0.0, 0.0, &mut count);
    assert!(!m.warn_due && !m.end_due);
    assert_eq!(count, (0, 0));
    let mut count = (1, 1);
    Memory::read(1.0, 4.0, 8.0, &mut count);
    assert_eq!(count, (0, 0));
}

#[test]
fn ac127_a_silent_subagent_gets_one_request_then_fails_its_task() {
    assert_eq!(subagent_step(599, SubStep::Nothing), None);
    assert_eq!(subagent_step(600, SubStep::Nothing), Some(SubStep::Tell));
    assert_eq!(subagent_step(900, SubStep::Tell), None, "once");
    assert_eq!(subagent_step(1200, SubStep::Tell), Some(SubStep::Fail));
    assert_eq!(subagent_step(1200, SubStep::Nothing), Some(SubStep::Fail), "a jump past both");
    assert_eq!(subagent_step(9000, SubStep::Fail), None, "failed once");
}

#[test]
fn ac125_a_close_decided_is_read_again_before_the_pane_goes() {
    let due = || { let mut r = run("DONE"); r.turn_ended = true; r.closing_for = CLOSE; r };
    assert!(still_closes(&due()));
    assert!(still_closes(&Run { state: Phase::Gone, turn_ended: false, ..due() }), "its agent ended");
    let mut pressed = due();
    pressed.new_work();
    assert!(!still_closes(&pressed), "a human's own prompt, or RUN, since the walk");
    assert!(!still_closes(&Run { turn_ended: false, ..due() }), "RUN took it to type in it");
    assert!(!still_closes(&Run { status: "IN PROGRESS".into(), ..due() }), "open again");
    assert!(!still_closes(&Run { state: Phase::Left, ..due() }), "given up since");
}

#[test]
fn ac141_only_a_session_with_all_over_is_idle_for_a_slot() {
    let idle = || { let mut r = run("DONE"); r.turn_ended = true; r.closing_for = 60; r };
    assert!(idle_holder(&idle()));
    assert!(!idle_holder(&Run { closing_for: 59, ..idle() }), "two walks quiet first");
    assert!(!idle_holder(&Run { turn_ended: false, ..idle() }), "in a turn");
    assert!(!idle_holder(&Run { open_tools: 1, ..idle() }), "a tool open");
    assert!(!idle_holder(&Run { status: "IN PROGRESS".into(), ..idle() }));
    assert!(!idle_holder(&Run { open: vec!["T-13".into()], ..idle() }), "a task of it open");
    assert!(!idle_holder(&Run { state: Phase::Gone, ..idle() }), "holds no slot");
}

#[test]
fn ac142_a_session_with_no_task_is_watched_for_its_end_alone() {
    let mut r = Run { task: String::new(), status: String::new(), ..run("IN PROGRESS") };
    let mut quiet = up(7200);
    quiet.memory = Some(Memory { gib: 20.0, end_gib: 8.0, warn_due: true, end_due: true });
    assert!(step(&mut r, &quiet).is_empty(), "no nudge, budget, memory end or closing: the human's");
    assert_eq!(r.state, Phase::Working);
    let ended = step(&mut r, &Facts { alive: false, exit: Some(0), ..up(30) });
    assert_eq!(r.state, Phase::Gone, "its agent's end frees its slot");
    assert!(ended.iter().all(|a| matches!(a, Act::Say(_))), "no task to move: {ended:?}");
}

/// The agent asked a human (ask_human): its conversation is known, its turn over.
fn asked() -> Run {
    Run { agent_session: Some("c0ffee".into()), turn_ended: true, asked: true, ..run("WAIT FOR HUMAN") }
}

fn closes(acts: &[Act]) -> bool {
    acts.iter().any(|a| matches!(a, Act::Close { keep: false, .. }))
}

#[test]
fn ac157_a_session_whose_agent_asked_closes_once_its_turn_is_over_and_quiet() {
    let mut r = asked();
    assert!(!closes(&step(&mut r, &up(30))), "one quiet walk is not enough");
    let acts = step(&mut r, &up(30));
    assert!(closes(&acts), "{acts:?}");
    assert!(asked_closes(&r), "and the check just before the pane goes agrees");
    assert_eq!(moved(&acts), None, "the task stays waiting for the human");
}

#[test]
fn ac157_activity_a_tool_a_subagent_or_its_turn_keep_it_open() {
    for (what, mut r, f) in [
        ("activity", asked(), Facts { active: true, ..up(60) }),
        ("a subagent", asked(), Facts { subs: true, ..up(60) }),
        ("another task open", asked(), Facts { others_open: true, ..up(60) }),
        ("a human's program", asked(), Facts { program: Some("vim".into()), ..up(60) }),
        ("an answer it waits for in its window", asked(), Facts { waiting: Some("Allow?".into()), ..up(60) }),
        ("its turn goes on", Run { turn_ended: false, ..asked() }, up(60)),
        ("a tool open", Run { open_tools: 1, ..asked() }, up(60)),
        ("no conversation to resume", Run { agent_session: None, ..asked() }, up(60)),
        ("a wait the agent did not ask for (AC-124, AC-123)", Run { asked: false, ..asked() }, up(60)),
    ] {
        let mut acts = step(&mut r, &f);
        acts.extend(step(&mut r, &f));
        assert!(!closes(&acts), "{what}: {acts:?}");
    }
}

#[test]
fn ac157_working_again_starts_its_quiet_anew() {
    let mut r = asked();
    step(&mut r, &up(30));
    r.status = "IN PROGRESS".into();
    step(&mut r, &up(30));
    assert_eq!(r.closing_for, 0, "its own move back");
    r.status = "WAIT FOR HUMAN".into();
    assert!(!closes(&step(&mut r, &up(30))));
}

#[test]
fn ac123_a_request_that_never_goes_holds_no_task_past_the_hour() {
    let mut r = run("IN PROGRESS");
    r.turn_ended = true;
    assert_eq!(step(&mut r, &up(300)), vec![Act::Nudge]);
    assert_eq!(step(&mut r, &up(3299)), vec![Act::Nudge], "a new turn met it: asked again");
    assert_eq!(moved(&step(&mut r, &up(1))).unwrap().1, "No activity for 60 minutes and no result from the agent.");
}

#[test]
fn ac123_each_quiet_stop_gets_a_request_three_a_run_at_most() {
    let mut r = run("IN PROGRESS");
    r.turn_ended = true;
    for n in 1..=NUDGES {
        assert_eq!(step(&mut r, &up(300)), vec![Act::Nudge], "request {n}");
        (r.nudged, r.idle_for) = (n, 0);                // it went (acts.rs); its turn runs
        assert!(step(&mut r, &up(600)).is_empty(), "no other before its turn's Stop");
        r.nudge_turn = false;                           // its Stop (hooks.rs)
        step(&mut r, &Facts { active: true, ..up(30) }); // the agent went back to work, then stopped again
    }
    assert!(step(&mut r, &up(3599)).is_empty(), "three went: the hour of nothing alone is left");
    assert_eq!(moved(&step(&mut r, &up(1))).unwrap().1, "No result from the agent within an hour of Dispatcher's request.");
    let mut fresh = run("IN PROGRESS");
    (fresh.turn_ended, fresh.nudged) = (true, NUDGES);
    fresh.new_work();
    assert_eq!(step(&mut fresh, &up(300)), vec![Act::Nudge], "a human's prompt or RUN counts them anew");
}
