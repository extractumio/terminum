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
        tx.execute("INSERT INTO agents (id, name, kind, command, color, at_once) VALUES ('p', 'Picker', 'claude', 'claude', 0, 3), ('a', 'Smith', 'claude', 'claude', 1, 2), ('b', 'Jones', 'claude', 'claude', 2, 1)", []).map_err(sql)?;
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
    let id = runs::add(tx, &Run { agent_id: agent.into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: "api".into(), ..Default::default() }, std::slice::from_ref(&t.key), NOW).unwrap();
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
        assert_eq!((r.slots, &r.candidates), (1, &vec![high.id.clone(), low.id.clone()]), "Smith runs 2 at once, one is held; the board's order");
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
        auto::started(tx, &high.key, &r.task, NOW)?;
        assert_eq!(given(NOW)?, (vec![low.id.clone()], 1));
        auto::not_started(tx, &high.key, &r.task)?;
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
        auto::started(tx, &one.key, &round.key, NOW + 1200)?;
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
        // terminumd ended before its press: no run would ever move it
        assert_eq!(decide(tx, "api", &setting(tx), NOW)?, Due::Nothing);
        assert_eq!(ops::get(tx, &id)?.status, "FAILED");
        assert!(matches!(decide(tx, "api", &setting(tx), NOW)?, Due::Off(why) if why == format!("its round {id} failed: see it in Archived.")), "a round that never started trips the breaker");
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
        auto::started(tx, &new.key, &again.task, NOW)?;
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
        auto::not_started(tx, &one.key, &r.task)?;
        assert_eq!(auto::started_by(tx, &r.task)?, 0, "a press that started nothing gives it back");
        // taken meanwhile by a human's session: no longer its to start
        ops::link(tx, HUMAN, &two.id, Some("tmux:local:t:%9"), NOW)?;
        assert!(chosen(tx, &pane, &two.id, NOW).err().unwrap().contains("is no longer free to start"));
        // turned off meanwhile, a round in flight starts no more
        tx.execute("UPDATE projects SET auto_on = 0 WHERE id = 'api'", []).map_err(sql)?;
        assert!(chosen(tx, &pane, &one.id, NOW).err().unwrap().starts_with("Auto-Dispatch was turned off for this project meanwhile"));
        tx.execute("UPDATE projects SET auto_on = 1 WHERE id = 'api'", []).map_err(sql)?;
        // its slots filled, it starts no more
        auto::started(tx, &one.key, &r.task, NOW)?;
        auto::started(tx, &late.key, &r.task, NOW)?;
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
            auto::started(tx, &t.key, &r.task, NOW + 20)?;
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
        let started = started_prompt(&t, "English");
        assert!(started.starts_with(&format!("Auto-Dispatcher, which a human turned on for this project, started task {};", t.id)) && started.contains("is your assignment"), "{started}");
        assert!(!started.contains("TITLE") && !started.contains("SECRET") && !words.contains("TITLE") && !words.contains("SECRET"), "no title or text of a task is typed");
        Ok(())
    }).unwrap();
}
