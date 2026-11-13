// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Auto-Dispatcher's rounds (AC-170): when one is made, with what, what it may start, and when it stops itself.

use super::*;
use crate::dispatcher::runs::{Phase, Run};
use crate::tasks::ops::sql;
use crate::tasks::store::{Store, HUMAN};
use rusqlite::Transaction;

const NOW: i64 = 1_000_000;

/// Its setting on: P picks, `default` runs a task naming none, every severity taken, no wait.
fn turn_on(tx: &Transaction, default: Option<&str>, at: i64) -> Result<(), String> {
    let all = crate::tasks::ops::SEVERITIES.iter().map(|p| p.to_string()).collect();
    auto::set(tx, "api", &Setting { on: true, agent: Some("p".into()), default: default.map(str::to_string), severities: all, wait: 0, ..auto::setting(tx, "api")? }, at)
}

/// Project api with the picker P (3 at once) and the executors A (2 at once) and B (1 at once); on, A its default.
fn store(name: &str) -> Store {
    let s = Store::open(crate::testutil::fresh_dir(name).join("kanban.sqlite"));
    s.with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('api', 'api', 0, '/srv', 0)", []).map_err(sql)?;
        tx.execute("INSERT INTO agents (id, project, name, kind, command, color, at_once) VALUES ('p', 'api', 'Picker', 'claude', 'claude', 0, 3), ('a', 'api', 'Smith', 'claude', 'claude', 1, 2), ('b', 'api', 'Jones', 'claude', 'claude', 2, 1)", []).map_err(sql)?;
        crate::tasks::slugs::fill(tx)?;
        turn_on(tx, Some("a"), NOW)
    }).unwrap();
    s
}

fn task(tx: &Transaction, title: &str, severity: &str) -> Task {
    ops::create(tx, HUMAN, NewTask { title: title.into(), text: "SECRET WORDS of its text".into(), project: Some("api".into()), severity: Some(severity.into()), ..Default::default() }, NOW).unwrap()
}

/// A run of `agent` on `t`, working in a pane.
fn running(tx: &Transaction, agent: &str, t: &Task) -> String {
    let id = runs::add(tx, &Run { agent_id: agent.into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: "api".into(), ..Default::default() }, std::slice::from_ref(&t.key), Default::default(), NOW).unwrap();
    runs::change(tx, &id, |r| { r.state = Phase::Working; r.pane_key = Some(format!("tmux:local:t:%{id}")); }).unwrap();
    id
}

fn setting(tx: &Transaction) -> Setting {
    auto::setting(tx, "api").unwrap()
}

fn round(tx: &Transaction) -> (String, Round) {
    let Due::Round(id) = decide(tx, "api", &setting(tx), NOW).unwrap() else { panic!("no round") };
    (id.clone(), auto::round_of(tx, &ops::key_of(tx, &id).unwrap()).unwrap().unwrap())
}

#[test]
fn ac170_a_round_is_made_for_the_free_slots_and_the_tasks_nobody_holds() {
    store("auto-round").with(|tx| {
        assert_eq!(decide(tx, "api", &setting(tx), NOW)?, Due::Nothing, "nothing in TODO");
        let (low, high, held) = (task(tx, "low", "P3"), task(tx, "high", "P0"), task(tx, "held", "P1"));
        running(tx, "a", &held);
        ops::move_to(tx, HUMAN, &held.id, "IN PROGRESS", NOW)?;
        let (id, r) = round(tx);
        assert_eq!((r.slots, &r.candidates), (1, &vec![high.id.clone(), low.id.clone()]), "Smith runs 2 at once, one is held; priority and saved order");
        let t = ops::get(tx, &id)?;
        assert_eq!((t.status.as_str(), t.budget_seconds, t.owner.as_str()), ("IN PROGRESS", Some(600), DISPATCHER));
        // its run takes no slot of its agent or project, and is shown as a round
        let run = running(tx, "p", &t);
        assert_eq!(decide(tx, "api", &setting(tx), NOW)?, Due::Nothing, "one round at a time");
        assert_eq!(ops::get(tx, &id)?.status, "IN PROGRESS");
        let picker = agents::by_id(tx, "p")?.unwrap();
        let slots = pool::Slots::read(tx)?;
        assert_eq!((slots.free_of(&picker), slots.free_of(&agents::by_id(tx, "a")?.unwrap()), slots.free_in(tx, "api")?), (3, 1, None));
        let shown = pool::board(tx)?;
        assert_eq!((shown["agents"]["p"][0]["round"].as_bool(), shown["agents"]["a"][0]["round"].as_bool()), (Some(true), Some(false)), "{shown}");
        // ended, it leaves the board; the same candidates with no more room are not judged again
        runs::change(tx, &run, |r| r.state = Phase::Closed)?;
        ops::move_to(tx, DISPATCHER, &id, "DONE", NOW)?;
        assert_eq!(decide(tx, "api", &setting(tx), NOW)?, Due::Nothing, "what a round was given and left is passed over");
        assert!(ops::get(tx, &id)?.archived_at.is_some(), "a round that ended is archived");
        let given = |at: i64| offer(tx, "api", &setting(tx), at).map(|(free, n)| (free.into_iter().map(|c| c.id).collect::<Vec<_>>(), n));
        // a round that filled its slots left the rest for want of room: they are offered again
        auto::started(tx, &high.key, Some(&r.task), "api", NOW)?;
        assert_eq!(given(NOW)?, (vec![low.id.clone()], 1));
        auto::not_started(tx, &high.key, Some(&r.task))?;
        assert_eq!(given(NOW)?.1, 0);
        // one a human edits since is judged anew; a comment or a day's end apart, the rest stays passed over
        ops::edit(tx, HUMAN, &low.id, ops::Edit { title: Some("low, reworded".into()), ..Default::default() }, NOW + 60)?;
        assert_eq!(given(NOW + 60)?, (vec![low.id.clone()], 1));
        assert_eq!(given(NOW + auto::PASSED)?, (vec![high.id.clone(), low.id.clone()], 1), "after a day it is judged again");
        // off, an ended round still leaves the board and none is made
        tx.execute("UPDATE projects SET auto_on = 0 WHERE id = 'api'", []).map_err(sql)?;
        assert_eq!(decide(tx, "api", &setting(tx), NOW + auto::PASSED)?, Due::Nothing, "off");
        Ok(())
    }).unwrap();
}

#[test]
fn ac170_one_round_at_a_time_over_the_projects_and_a_main_task_goes_with_its_subtasks() {
    store("auto-shared").with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('web', 'web', 1, '/srv', 1)", []).map_err(sql)?;
        crate::tasks::slugs::fill(tx)?;
        tx.execute("INSERT INTO agents (id, project, name, kind, command, color, at_once) VALUES ('wp', 'web', 'Picker', 'claude', 'claude', 0, 3), ('wa', 'web', 'Smith', 'claude', 'claude', 1, 2)", []).map_err(sql)?;
        auto::set(tx, "web", &Setting { on_at: 0, agent: Some("wp".into()), default: Some("wa".into()), ..setting(tx) }, NOW)?;
        let parent = task(tx, "parent", "P1");
        let child = ops::create(tx, HUMAN, NewTask { title: "child".into(), parent: Some(parent.id.clone()), project: Some("api".into()), severity: Some("P2".into()), ..Default::default() }, NOW)?;
        ops::create(tx, HUMAN, NewTask { title: "of web".into(), project: Some("web".into()), severity: Some("P1".into()), ..Default::default() }, NOW)?;
        let (id, r) = round(tx);
        assert_eq!(r.candidates, std::slice::from_ref(&parent.id), "AC-185: the subtask is no candidate by itself: it goes with its main task");
        let web = auto::setting(tx, "web")?;
        assert_eq!(decide(tx, "web", &web, NOW)?, Due::Nothing, "api's round is open: one round at a time over the projects");
        let run = running(tx, "p", &ops::get(tx, &id)?);
        runs::change(tx, &run, |r| r.state = Phase::Closed)?;
        ops::move_to(tx, DISPATCHER, &id, "DONE", NOW)?;
        assert_eq!(auto::due_for(tx)?.into_iter().map(|(p, _)| p).collect::<Vec<_>>(), ["web", "api"], "the project whose last round is oldest goes first");
        assert!(matches!(decide(tx, "web", &web, NOW)?, Due::Round(_)));
        // AC-185: a main task is a candidate only while each subtask still to do stands in TODO, free, for the wait
        let found = |tx: &Transaction, s: &Setting, now: i64| auto::candidates(tx, "api", s, now, None).map(|all| all.into_iter().map(|c| c.id).collect::<Vec<_>>());
        ops::move_to(tx, HUMAN, &child.id, "FAILED", NOW)?;
        assert!(found(tx, &setting(tx), NOW)?.is_empty(), "a failed subtask is the human's to run or move first");
        ops::move_to(tx, HUMAN, &child.id, "TODO", NOW + 100)?;
        // the round above was given the main task and left it: the human's move of its subtask has it judged anew
        let waits = Setting { wait: 1, ..setting(tx) };
        assert!(found(tx, &waits, NOW + 120)?.is_empty(), "its wait counts from its subtask's move");
        assert_eq!(found(tx, &waits, NOW + 160)?, std::slice::from_ref(&parent.id));
        let only = |p: &str| Setting { severities: vec![p.to_string()], ..setting(tx) };
        assert_eq!(found(tx, &only("P2"), NOW + 160)?, std::slice::from_ref(&parent.id), "taken for its subtask's severity");
        assert!(found(tx, &only("P3"), NOW + 160)?.is_empty());
        ops::link(tx, HUMAN, &child.id, Some("tmux:local:t:%9"), NOW + 100)?;
        assert!(found(tx, &setting(tx), NOW + 160)?.is_empty(), "a session holds its subtask");
        ops::link(tx, HUMAN, &child.id, None, NOW + 100)?;
        ops::move_to(tx, HUMAN, &parent.id, "DONE", NOW + 100)?;
        assert_eq!(found(tx, &setting(tx), NOW + 160)?, std::slice::from_ref(&child.id), "its main task DONE, a subtask left is a candidate itself");
        // what its agent is told: terminum's words and the IDs alone
        let prompt = started_prompt(&[ops::get(tx, &parent.id)?, ops::get(tx, &child.id)?], "English");
        assert!(prompt.starts_with(&format!("Auto-Dispatcher, which a human turned on for this project, started main task {} with its subtask still to do: {}; no human started this run or watches it now.", parent.id, child.id)), "{prompt}");
        assert!(prompt.contains("a main task goes last") && !prompt.contains("SECRET"));
        Ok(())
    }).unwrap();
}

#[test]
fn ac170_by_the_boards_order_terminumd_starts_them_itself_top_down() {
    store("auto-order").with(|tx| {
        auto::set(tx, "api", &Setting { mode: auto::ORDER.into(), agent: None, day_most: 5, ..setting(tx) }, NOW)?;
        let (low, high, mid, other) = (task(tx, "low", "P2"), task(tx, "high", "P0"), task(tx, "mid", "P1"), task(tx, "other", "P1"));
        auto::set_agent(tx, &other.id, Some("b"), NOW)?;
        let ids = |due: Due| { let Due::Start(l) = due else { panic!("no start: {due:?}") }; l.into_iter().map(|c| (c.id, c.agent)).collect::<Vec<_>>() };
        let first = ids(decide(tx, "api", &setting(tx), NOW)?);
        assert_eq!(first[0], (high.id.clone(), "a".to_string()), "P0 first");
        let mut rest = first[1..].to_vec();
        rest.sort();
        let mut wanted = vec![(mid.id.clone(), "a".to_string()), (other.id.clone(), "b".to_string())];
        wanted.sort();
        assert_eq!(rest, wanted, "then P1, each while its agent has a free slot: Smith's 2, Jones's 1; P2 waits for room");
        assert_eq!(auto::started_in_day(tx, "api", NOW)?, 3, "each took its slot of the day");
        assert!(auto::rounds_shown(tx, "api")?.is_empty(), "no round: no agent picks");
        // a press that found no room gives its slot back, and a later walk takes it again; what it started and its agent
        // put back is not started twice
        auto::not_started(tx, &mid.key, None)?;
        for t in [&high, &other] {
            // its session opened and its agent never took the prompt: the task never left TODO, its run ended
            let run = running(tx, "a", t);
            runs::change(tx, &run, |r| r.state = Phase::Closed)?;
        }
        let mut again = ids(decide(tx, "api", &setting(tx), NOW)?);
        again.sort();
        let mut wanted = vec![(mid.id.clone(), "a".to_string()), (low.id.clone(), "a".to_string())];
        wanted.sort();
        assert_eq!(again, wanted, "high and other stand in TODO still, started once: not again");
        // terminumd stopped between taking a slot and its press: the start is given back at the next walk
        let lost = task(tx, "lost", "P3");
        auto::started(tx, &lost.key, None, "api", NOW)?;
        assert_eq!(auto::started_in_day(tx, "api", NOW)?, 5);
        tx.execute("UPDATE projects SET auto_day_most = 9 WHERE id = 'api'", []).map_err(sql)?;
        tx.execute("UPDATE agents SET at_once = 0 WHERE id IN ('a', 'b')", []).map_err(sql)?;
        assert_eq!(decide(tx, "api", &setting(tx), NOW)?, Due::Nothing, "no room");
        assert_eq!(auto::started_in_day(tx, "api", NOW)?, 2, "the three never pressed are free again; the two that ran are kept");
        tx.execute("UPDATE agents SET at_once = 2 WHERE id IN ('a', 'b')", []).map_err(sql)?;
        // its day's cap holds here too
        tx.execute("UPDATE projects SET auto_day_most = 2 WHERE id = 'api'", []).map_err(sql)?;
        assert_eq!(decide(tx, "api", &setting(tx), NOW)?, Due::Nothing, "the cap of 24 hours is reached");
        // a human moves one that failed back to TODO: a candidate anew
        tx.execute("UPDATE projects SET auto_day_most = 9 WHERE id = 'api'", []).map_err(sql)?;
        ops::move_to(tx, HUMAN, &high.id, "FAILED", NOW + 1)?;
        assert!(auto::candidates(tx, "api", &setting(tx), NOW + 1, None)?.iter().all(|c| c.id != high.id), "FAILED is never taken");
        ops::move_to(tx, HUMAN, &high.id, "TODO", NOW + 2)?;
        assert_eq!(auto::candidates(tx, "api", &setting(tx), NOW + 2, None)?[0].id, high.id);
        // a round left from the other way of picking starts no more
        tx.execute("UPDATE projects SET auto_day_most = 9, auto_mode = 'agent', auto_agent = 'p' WHERE id = 'api'", []).map_err(sql)?;
        let (id, r) = round(tx);
        let pane = format!("tmux:local:t:%{}", running(tx, "p", &ops::get(tx, &id)?));
        tx.execute("UPDATE projects SET auto_mode = 'order' WHERE id = 'api'", []).map_err(sql)?;
        assert!(chosen(tx, &pane, &r.candidates[0], NOW).err().unwrap().starts_with("Auto-Dispatch was set to priority and saved order"));
        assert_eq!(decide(tx, "api", &setting(tx), NOW)?, Due::Nothing, "its round is still open");
        Ok(())
    }).unwrap();
}

#[test]
fn ac170_it_starts_no_more_than_its_days_cap() {
    store("auto-cap").with(|tx| {
        auto::set(tx, "api", &Setting { day_most: 2, ..setting(tx) }, NOW)?;
        let (one, two, three) = (task(tx, "one", "P1"), task(tx, "two", "P1"), task(tx, "three", "P1"));
        auto::set_agent(tx, &three.id, Some("b"), NOW)?;
        let (id, r) = round(tx);
        assert_eq!(r.slots, 2, "Smith's 2 and Jones's 1 are free: the cap leaves 2");
        let pane = format!("tmux:local:t:%{}", running(tx, "p", &ops::get(tx, &id)?));
        auto::started(tx, &one.key, Some(&r.task), "api", NOW)?;
        assert_eq!((auto::started_in_day(tx, "api", NOW)?, offer(tx, "api", &setting(tx), NOW)?.1), (1, 1));
        auto::started(tx, &two.key, Some(&r.task), "api", NOW)?;
        assert_eq!(offer(tx, "api", &setting(tx), NOW)?.1, 0, "the day's cap is reached: no round");
        assert!(chosen(tx, &pane, &three.id, NOW).err().unwrap().starts_with("This project's 2 tasks a day are started"));
        // 24 hours on, it starts again
        assert_eq!(offer(tx, "api", &setting(tx), NOW + auto::DAY)?.1, 1);
        Ok(())
    }).unwrap();
}

#[test]
fn ac170_after_a_round_that_started_nothing_it_rests_unless_a_task_changed() {
    store("auto-rests").with(|tx| {
        tx.execute("UPDATE agents SET at_once = 1 WHERE id = 'a'", []).map_err(sql)?;
        let all = [task(tx, "one", "P1"), task(tx, "two", "P1"), task(tx, "three", "P1"), task(tx, "four", "P1")];
        let (id, r) = round(tx);
        assert_eq!((r.slots, r.candidates.len()), (1, 3), "three candidates a slot");
        let below = all.iter().find(|t| !r.candidates.contains(&t.id)).unwrap();
        let run = running(tx, "p", &ops::get(tx, &id)?);
        runs::change(tx, &run, |r| r.state = Phase::Closed)?;
        ops::move_to(tx, DISPATCHER, &id, "DONE", NOW)?;
        assert_eq!(offer(tx, "api", &setting(tx), NOW + 1)?.0.into_iter().map(|c| c.id).collect::<Vec<_>>(), std::slice::from_ref(&below.id), "the task below is next");
        assert_eq!(decide(tx, "api", &setting(tx), NOW + 1)?, Due::Nothing, "it rests: a long list is not read through at once");
        // an agent's edit of one it left does not bring that one back; a task made since is news: a round at once
        ops::edit(tx, DISPATCHER, &r.candidates[0], ops::Edit { title: Some("reworded by an agent".into()), ..Default::default() }, NOW + 2)?;
        assert_eq!(offer(tx, "api", &setting(tx), NOW + 3)?.0.len(), 1, "what a round left stays passed over");
        assert_eq!(decide(tx, "api", &setting(tx), NOW + 3)?, Due::Nothing, "still resting");
        let fresh = ops::create(tx, HUMAN, NewTask { title: "fresh".into(), project: Some("api".into()), severity: Some("P1".into()), ..Default::default() }, NOW + 3)?;
        let Due::Round(next) = decide(tx, "api", &setting(tx), NOW + 3)? else { panic!("a new task makes a round at once") };
        let mut given = auto::round_of(tx, &ops::key_of(tx, &next)?)?.unwrap().candidates;
        given.sort();
        let mut wanted = vec![below.id.clone(), fresh.id];
        wanted.sort();
        assert_eq!(given, wanted);
        Ok(())
    }).unwrap();
}

#[test]
fn ac170_it_takes_the_severities_chosen_after_the_wait_and_none_twice() {
    store("auto-takes").with(|tx| {
        auto::set(tx, "api", &Setting { severities: vec!["P0".into(), "P1".into()], wait: 10, ..setting(tx) }, NOW)?;
        let (one, low, round) = (task(tx, "one", "P1"), task(tx, "low", "P2"), task(tx, "stands for a round", "P3"));
        let ids = |at: i64| auto::candidates(tx, "api", &setting(tx), at, None).map(|l| l.into_iter().map(|c| c.id).collect::<Vec<_>>());
        assert!(ids(NOW + 599)?.is_empty(), "made less than the wait ago");
        assert_eq!(ids(NOW + 600)?, std::slice::from_ref(&one.id), "P2 is not taken");
        assert!(auto::candidates(tx, "api", &setting(tx), NOW + 600, Some(&low.id))?.is_empty(), "nor by its ID: a round in flight cannot start it");
        // an edit starts the wait anew; a comment does not
        ops::edit(tx, HUMAN, &one.id, ops::Edit { title: Some("one, reworded".into()), ..Default::default() }, NOW + 590)?;
        assert!(ids(NOW + 600)?.is_empty());
        crate::tasks::comments::comment(tx, HUMAN, &one.id, "a word", NOW + 1100)?;
        assert_eq!(ids(NOW + 1190)?, std::slice::from_ref(&one.id));
        // started by it once and put back by an agent: not run again; a human's move makes it a candidate anew
        auto::started(tx, &one.key, Some(&round.key), "api", NOW + 1200)?;
        ops::move_to(tx, DISPATCHER, &one.id, "IN PROGRESS", NOW + 1200)?;
        ops::move_to(tx, DISPATCHER, &one.id, "TODO", NOW + 1300)?;
        assert!(ids(NOW + 9000)?.is_empty(), "no loop over a task its agent gives back");
        ops::move_to(tx, HUMAN, &one.id, "FAILED", NOW + 9000)?;
        ops::move_to(tx, HUMAN, &one.id, "TODO", NOW + 9001)?;
        assert_eq!(ids(NOW + 9601)?, std::slice::from_ref(&one.id));
        Ok(())
    }).unwrap();
}

#[test]
fn ac170_a_new_task_a_round_with_no_run_and_a_picker_past_its_limit() {
    store("auto-news").with(|tx| {
        let old = task(tx, "old", "P1");
        let (id, _) = round(tx);
        // terminumd ended before its press: it fails into Archived, said why, and a new round is made at once
        let (fresh, again) = round(tx);
        let gone = ops::get(tx, &id)?;
        assert_eq!((gone.status.as_str(), gone.archived_at.is_some()), ("FAILED", true));
        let why: String = tx.query_row("SELECT text FROM events WHERE task = ?1 AND kind = 'why'", [&gone.key], |r| r.get(0)).map_err(sql)?;
        assert_eq!(why, "It did not start: terminumd stopped before its session opened.");
        assert_eq!(auto::round_of(tx, &gone.key)?, None, "no round any more: the breaker does not read it");
        assert!(setting(tx).on, "it stays on: nothing is wrong");
        assert_eq!(again.candidates, std::slice::from_ref(&old.id), "what it was given is offered again");
        // a press that failed for the picker's own fault, with no run either, still trips it
        ops::move_to(tx, DISPATCHER, &fresh, "FAILED", NOW)?;
        assert_eq!(decide(tx, "api", &setting(tx), NOW)?, Due::Off(format!("its round {fresh} failed: see it in Archived.")));
        turn_on(tx, Some("a"), NOW + 1)?;
        tx.execute("UPDATE auto_rounds SET at = ?1", [NOW - 1]).map_err(sql)?;
        // a new task is news, at once
        let new = task(tx, "new", "P2");
        let (fresh, again) = round(tx);
        assert_eq!(again.candidates, [old.id.clone(), new.id.clone()]);
        // its agent past its week's limit picks nothing
        let run = running(tx, "p", &ops::get(tx, &fresh)?);
        runs::change(tx, &run, |r| r.state = Phase::Closed)?;
        ops::move_to(tx, DISPATCHER, &fresh, "DONE", NOW)?;
        auto::started(tx, &new.key, Some(&again.task), "api", NOW)?;
        tx.execute("UPDATE agents SET week_limit = 1 WHERE id = 'p'", []).map_err(sql)?;
        tx.execute("INSERT INTO usage (run, agent_id, at, input, output, cache_write, cache_read) VALUES (?1, 'p', ?2, 2000000, 0, 0, 0)", (&run, NOW)).map_err(sql)?;
        task(tx, "newer", "P2");
        assert_eq!(decide(tx, "api", &setting(tx), NOW + 2)?, Due::Nothing, "the picking agent is held to its limits");
        tx.execute("UPDATE agents SET week_limit = NULL WHERE id = 'p'", []).map_err(sql)?;
        assert!(matches!(decide(tx, "api", &setting(tx), NOW + 2)?, Due::Round(_)));
        Ok(())
    }).unwrap();
}

#[test]
fn ac170_slots_are_those_of_the_agents_its_tasks_run_on_never_past_a_limit() {
    store("auto-slots").with(|tx| {
        let (one, two, three) = (task(tx, "one", "P1"), task(tx, "two", "P1"), task(tx, "three", "P1"));
        auto::set_agent(tx, &three.id, Some("b"), NOW)?;
        assert_eq!(auto::candidates(tx, "api", &setting(tx), NOW, None)?.into_iter().map(|c| (c.id, c.agent)).collect::<Vec<_>>(),
                   [(three.id.clone(), "b".to_string()), (two.id.clone(), "a".to_string()), (one.id.clone(), "a".to_string())], "a task's own agent, else the default");
        assert_eq!(offer(tx, "api", &setting(tx), NOW)?.1, 3, "Smith's 2 and Jones's 1");
        tx.execute("UPDATE projects SET agent_limit = 2 WHERE id = 'api'", []).map_err(sql)?;
        assert_eq!(offer(tx, "api", &setting(tx), NOW)?.1, 2, "the project runs 2 at once");
        // Smith past its week's limit starts nothing for Auto-Dispatcher: only Jones's task is offered
        tx.execute("UPDATE agents SET week_limit = 1 WHERE id = 'a'", []).map_err(sql)?;
        let run = running(tx, "a", &one);
        tx.execute("INSERT INTO usage (run, agent_id, at, input, output, cache_write, cache_read) VALUES (?1, 'a', ?2, 2000000, 0, 0, 0)", (&run, NOW)).map_err(sql)?;
        runs::change(tx, &run, |r| r.state = Phase::Closed)?;
        let (free, n) = offer(tx, "api", &setting(tx), NOW)?;
        assert_eq!((free.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), n), (vec![three.id.as_str()], 1));
        // with no default, a task naming no agent is no candidate
        turn_on(tx, None, NOW)?;
        assert_eq!(auto::candidates(tx, "api", &setting(tx), NOW, None)?.len(), 1);
        // a task naming an agent deleted since is no candidate: never the default's
        turn_on(tx, Some("a"), NOW)?;
        tx.execute("DELETE FROM agents WHERE id = 'b'", []).map_err(sql)?;
        assert!(auto::candidates(tx, "api", &setting(tx), NOW, None)?.iter().all(|c| c.id != three.id));
        Ok(())
    }).unwrap();
}

#[test]
fn ac170_only_a_rounds_own_pane_starts_only_its_candidates_while_they_are_free() {
    store("auto-chosen").with(|tx| {
        let (one, two) = (task(tx, "one", "P1"), task(tx, "two", "P2"));
        let (id, r) = round(tx);
        assert_eq!(r.slots, 2);
        let late = task(tx, "late", "P0");
        let run = running(tx, "p", &ops::get(tx, &id)?);
        let pane = format!("tmux:local:t:%{run}");
        assert_eq!(chosen(tx, "tmux:local:t:%other", &one.id, NOW).err().unwrap(), ONLY_ROUND);
        let other = running(tx, "b", &late);
        assert_eq!(chosen(tx, &format!("tmux:local:t:%{other}"), &one.id, NOW).err().unwrap(), ONLY_ROUND, "a task's run is no round");
        assert!(chosen(tx, &pane, &late.id, NOW).err().unwrap().starts_with(&format!("{} is not among this round's candidates", late.id)));
        let c = chosen(tx, &pane, &one.id, NOW)?;
        assert_eq!((c.task.id.as_str(), c.agent.id.as_str(), c.left), (one.id.as_str(), "a", 1));
        assert_eq!(auto::started_by(tx, &r.task)?, 1, "its slot of the round is taken with the reading: two calls at once never both find it");
        auto::not_started(tx, &one.key, Some(&r.task))?;
        assert_eq!(auto::started_by(tx, &r.task)?, 0, "a press that started nothing gives it back");
        // taken meanwhile by a human's session: no longer its to start
        ops::link(tx, HUMAN, &two.id, Some("tmux:local:t:%9"), NOW)?;
        assert!(chosen(tx, &pane, &two.id, NOW).err().unwrap().contains("is no longer free to start"));
        // turned off meanwhile, a round in flight starts no more
        tx.execute("UPDATE projects SET auto_on = 0 WHERE id = 'api'", []).map_err(sql)?;
        assert!(chosen(tx, &pane, &one.id, NOW).err().unwrap().starts_with("Auto-Dispatch was turned off for this project meanwhile"));
        tx.execute("UPDATE projects SET auto_on = 1 WHERE id = 'api'", []).map_err(sql)?;
        // its slots filled, it starts no more
        auto::started(tx, &one.key, Some(&r.task), "api", NOW)?;
        auto::started(tx, &late.key, Some(&r.task), "api", NOW)?;
        assert!(chosen(tx, &pane, &one.id, NOW).err().unwrap().starts_with("This round's 2 free slots are filled"));
        // ended, its pane starts nothing
        ops::move_to(tx, DISPATCHER, &id, "DONE", NOW)?;
        assert_eq!(chosen(tx, &pane, &one.id, NOW).err().unwrap(), ONLY_ROUND);
        Ok(())
    }).unwrap();
}

#[test]
fn ac170_it_turns_itself_off_when_its_round_or_two_of_its_tasks_fail() {
    store("auto-breaker").with(|tx| {
        let (one, two) = (task(tx, "one", "P1"), task(tx, "two", "P2"));
        let (id, r) = round(tx);
        ops::move_to(tx, DISPATCHER, &id, "FAILED", NOW)?;
        assert_eq!(decide(tx, "api", &setting(tx), NOW)?, Due::Off(format!("its round {id} failed: see it in Archived.")));
        let off = setting(tx);
        assert_eq!((off.on, off.agent.as_deref(), off.off_why.as_deref()), (false, Some("p"), Some(format!("its round {id} failed: see it in Archived.").as_str())), "off, its agent still chosen");
        // a human turns it on again: what failed before does not count
        turn_on(tx, Some("a"), NOW + 10)?;
        assert_eq!(setting(tx).off_why, None);
        assert_eq!(auto::tripped(tx, "api", NOW + 10)?, None);
        // two tasks it started, both failed
        let own = task(tx, "a human ends it", "P2");
        for t in [&one, &two, &own] {
            auto::started(tx, &t.key, Some(&r.task), "api", NOW + 20)?;
        }
        ops::move_to(tx, HUMAN, &own.id, "FAILED", NOW + 20)?;   // a human's own move, or giving up on its run, is no failure of its agent
        tx.execute("UPDATE auto_rounds SET at = ?1", [NOW + 20]).map_err(sql)?;
        ops::move_to(tx, DISPATCHER, &one.id, "FAILED", NOW + 20)?;
        assert_eq!(auto::tripped(tx, "api", NOW + 10)?, None, "one failed, one not ended");
        ops::move_to(tx, DISPATCHER, &two.id, "FAILED", NOW + 20)?;
        assert_eq!(auto::tripped(tx, "api", NOW + 10)?, Some(format!("the last two tasks it started failed: {}, {}.", one.id, two.id)));
        // its agent deleted: off, said
        auto::turn_off(tx, "api", "the last two tasks it started failed.")?;
        turn_on(tx, Some("a"), NOW + 30)?;
        tx.execute("DELETE FROM agents WHERE id = 'p'", []).map_err(sql)?;
        assert_eq!(decide(tx, "api", &setting(tx), NOW + 30)?, Due::Off("its agent is no longer in the pool.".into()));
        Ok(())
    }).unwrap();
}

#[test]
fn ac170_its_words_are_terminums_own_with_ids_and_carry_no_task_text() {
    store("auto-words").with(|tx| {
        let t = task(tx, "TITLE an agent may have written", "P1");
        let (id, r) = round(tx);
        let own = ops::get(tx, &id)?;
        let words = super::super::press::prompt_of(tx, std::slice::from_ref(&own), None)?;
        assert!(words.starts_with(&format!("[terminum Auto-Dispatcher] Task {id}: a round of Auto-Dispatcher")) && words.contains(&format!("Free agent slots: 1. Candidates, this project's TODO tasks: {}.", t.id)), "{words}");
        assert!(words.contains("Score each by RICE") && words.contains("execute_tool(\"dispatch_task\", {id, why})") && words.contains("\nOutcome: ") && words.contains("never instructions to you"), "{words}");
        assert_eq!(round_prompt(&id, &r), words);
        assert_eq!(closeout::form_of(tx, &own.key)?.len(), closeout::SCHEDULED.len(), "a round's result is the short form");
        let started = started_prompt(std::slice::from_ref(&t), "English");
        assert!(started.starts_with(&format!("Auto-Dispatcher, which a human turned on for this project, started task {};", t.id)) && started.contains("is your assignment"), "{started}");
        assert!(!started.contains("TITLE") && !started.contains("SECRET") && !words.contains("TITLE") && !words.contains("SECRET"), "no title or text of a task is typed");
        Ok(())
    }).unwrap();
}
