// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The Dashboard's numbers (terminum.md AC-152, AC-170, AC-171): the periods' buckets, the human's
//! figures only, Dispatcher's sessions and failures, what they spent.

use std::collections::{BTreeMap, HashMap};

use serde_json::{json, Value};

use super::system::class;
use super::*;
use crate::dispatcher::runs::{self, Phase, Run};
use crate::dispatcher::usage::{self, Totals};
use crate::messages::{insert, Message};
use crate::tasks::ops::{self, NewTask};
use crate::tasks::store::{Store, DISPATCHER, HUMAN};

const M: i64 = 60_000;

fn store(tag: &str) -> Store {
    Store::open(crate::testutil::fresh_dir(tag).join("kanban.sqlite"))
}

/// Today's local noon, unix ms.
fn noon(s: &Store) -> i64 {
    s.with(|tx| tx.query_row("SELECT CAST(strftime('%s', date('now', 'localtime'), '+12 hours', 'utc') AS INTEGER) * 1000", [], |r| r.get(0)).map_err(sql)).unwrap()
}

fn msg(at: i64, pane: &str, project: Option<&str>) -> Message {
    Message { at, sender: HUMAN.into(), via: "page".into(), pane: pane.into(), host: "local".into(), cwd: "/Users/alex/api".into(), text: "words".into(), project: project.map(str::to_string), ..Default::default() }
}

/// A row of the input log: the human's typed Enter from a browser, or Dispatcher's launch of `run`, or a failed write.
fn input(tx: &rusqlite::Transaction, at: i64, sender: &str, component: &str, pane: &str, browser: Option<&str>, run: Option<&str>, error: Option<&str>) {
    tx.execute("INSERT INTO inputs (at, ended, sender, component, kind, text, bytes, pane, device, browser, run, error, version)
                VALUES (?1, ?1, ?2, ?3, ?4, 'ls<Enter>', 3, ?5, ?6, ?7, ?8, ?9, 'test')",
               rusqlite::params![at, sender, component, if run.is_some() { "launch" } else { "typed" }, pane, browser.map(|_| "web"), browser, run, error]).unwrap();
}

fn ask(s: &Store, now_ms: i64, tab: &str, period: Period, project: Option<&str>) -> Value {
    s.with(|tx| numbers(tx, now_ms, tab, period, project, &HashMap::from([("p1".to_string(), 3usize)]))).unwrap()
}

#[test]
fn ac152_local_days_and_dates() {
    assert_eq!((civil(0), civil(20_400), civil(-1)), ("1970-01-01".into(), "2025-11-08".into(), "1969-12-31".into()));
    for z in [-800_000, -1, 0, 59, 60, 20_400, 20_819, 800_000] {
        let (y, m, d) = civil_of(z);
        assert_eq!(days_of(y, m, d), z);
    }
}

#[test]
fn ac152_a_period_s_buckets() {
    // 2026-11-10 22:30 local
    let now = days_of(2026, 11, 10) * DAY + 22 * HOUR + 30 * M;
    let of = |p| Span::of(p, now);
    let day = of(Period::Day);
    assert_eq!((day.unit, day.len(), day.lo(), day.index(now)), ("hour", 24, now - 30 * M - 23 * HOUR, Some(23)), "the last 24 hours, the hour going on last");
    assert_eq!((of(Period::TwoWeeks).len(), of(Period::Month).len(), of(Period::Month).unit), (14, 30, "day"));
    assert_eq!(of(Period::Month).index(days_of(2026, 10, 12) * DAY), Some(0));
    assert_eq!(of(Period::Month).index(days_of(2026, 10, 12) * DAY - 1), None);
    let quarter = of(Period::Quarter);
    assert_eq!((quarter.unit, quarter.len(), quarter.days(), quarter.end(12) - quarter.edges[12]), ("week", 13, 91, now + 1 - quarter.edges[12]), "13 weeks of 7 days, the last up to now");
    assert_eq!(quarter.edges[12], (days_of(2026, 11, 10) - 6) * DAY);
    let year = of(Period::Year);
    assert_eq!((year.unit, year.len(), civil(year.lo() / DAY), civil(year.edges[11] / DAY)), ("month", 12, "2025-12-01".into(), "2026-11-01".into()), "12 calendar months");
    let before = year.before(Period::Year);
    assert_eq!((before.until, before.holds(year.lo())), (now - 365 * DAY, false), "the span before ends where this one's time of year was");
    assert_eq!(Period::of("2w"), Period::TwoWeeks);
    assert_eq!(Period::of("decade"), Period::Month, "an unknown period is the month");
    let said = year.json();
    assert_eq!((said[1]["from"].clone(), said[1]["to"].clone(), day.json()[23]["hour"].clone()), (json!("2026-01-01"), json!("2026-01-31"), json!(22)));
}

#[test]
fn ac152_the_human_s_time_and_load_only() {
    let s = store("dashboard-you");
    let noon = noon(&s);
    let mut robot = msg(noon + 7 * M, "k3", None);
    robot.sender = DISPATCHER.into();
    s.with(|tx| {
        // noon: a prompt in k1 (project p1), 12:05 and 12:06 in k2, Dispatcher's at 12:07; an Enter from an iPhone at 12:06
        for m in [msg(noon, "k1", Some("p1")), msg(noon + 5 * M, "k2", None), msg(noon + 6 * M, "k2", None), robot.clone(), msg(noon - DAY, "k1", None)] {
            insert(tx, &m)?;
        }
        input(tx, noon + 6 * M, HUMAN, "page", "k2", Some("Safari 18 · iPhone"), None, None);
        input(tx, noon + 6 * M, DISPATCHER, "dispatcher.gate", "k3", None, None, None);
        // the board at 12:30, a pause over 15 minutes: the human makes a task and moves it; an agent's ask at 12:31 moved on at 12:41
        let at = (noon + 30 * M) / 1000;
        let t = ops::create(tx, HUMAN, NewTask { title: "t".into(), ..Default::default() }, at)?;
        ops::move_to(tx, HUMAN, &t.key, "IN PROGRESS", at)?;
        ops::move_to(tx, DISPATCHER, &t.key, "WAIT FOR HUMAN", at + 60)?;
        ops::move_to(tx, DISPATCHER, &t.key, "IN PROGRESS", at + 660)?;
        ops::move_to(tx, DISPATCHER, &t.key, "WAIT FOR HUMAN", at + 700)?;
        Ok(())
    }).unwrap();
    let d = ask(&s, noon + 50 * M, "you", Period::TwoWeeks, None);
    let (tiles, today) = (&d["tiles"], &d["series"][13]);
    assert_eq!((d["unit"].clone(), d["series"].as_array().unwrap().len(), d["buckets"][13]["from"].clone()), (json!("day"), 14, d["buckets"][13]["to"].clone()));
    assert_eq!((tiles["active_ms"].clone(), today["active_ms"].clone()), (json!(12 * M), json!(10 * M)), "today noon to 12:06 and 2 minutes more, then the board's 2 minutes; yesterday's 2: Dispatcher's rows add none");
    assert_eq!(today["by"], json!({"": 5 * M, "p1": 5 * M}), "the time after an event is its project's; a task of the default project is no project's");
    assert_eq!(d["projects"][1], json!({"id": "p1", "name": "", "n": 5 * M, "color": 3}));
    assert_eq!((tiles["longest_ms"].clone(), tiles["active_units"].clone(), tiles["units"].clone()), (json!(8 * M), json!(2), json!(14)));
    assert_eq!((today["sessions"].clone(), today["peak"].clone(), today["switches"].clone(), today["sent"].clone()), (json!(2), json!(2), json!(1), json!(4)), "k1 then k2: one switch");
    assert_eq!(today["spans"], json!([[720, 728], [750, 752]]));
    assert_eq!((tiles["board"].clone(), tiles["board_by"]["made"].clone(), tiles["board_by"]["move"].clone(), today["board"].clone()), (json!(2), json!(1), json!(1), json!({"made": 1, "move": 1})), "the human's board events only");
    assert_eq!((tiles["asks"].clone(), tiles["waiting"].clone(), tiles["answer_ms"].clone()), (json!(2), json!(1), json!(600_000)), "a wait lasts until the task's next move");
    assert_eq!(tiles["phone"], 1.0, "of what the pages sent");
    assert_eq!(d["before"], Value::Null, "what is kept does not reach the span before: no comparison");
    assert_eq!(d["kept_from"], d["buckets"][13]["from"], "the page says from when it counts");
    // the log reaches back: the day before counts, up to the same time
    s.with(|tx| { input(tx, noon - 40 * DAY, HUMAN, "page", "k1", None, None, None); Ok(()) }).unwrap();
    let d = ask(&s, noon + 50 * M, "you", Period::Day, None);
    assert_eq!((d["unit"].clone(), d["before"]["active_ms"].clone(), d["kept_from"].clone()), (json!("hour"), json!(2 * M), Value::Null));
    assert_eq!(d["series"][23]["spans"], Value::Null, "a working day is drawn of days only");
    let only = ask(&s, noon + 50 * M, "you", Period::TwoWeeks, Some("p1"));
    assert_eq!((only["tiles"]["active_ms"].clone(), only["tiles"]["board"].clone()), (json!(2 * M), json!(0)));
    let none = ask(&s, noon + 50 * M, "you", Period::TwoWeeks, Some(""));
    assert_eq!((none["tiles"]["active_ms"].clone(), none["tiles"]["board"].clone()), (json!(7 * M), json!(2)), "no project: the default project's tasks");
}

/// A run of `agent` for a new task in `project`, launched at unix ms `at`; closed after `minutes` when given.
fn run(tx: &rusqlite::Transaction, agent: &str, project: &str, at: i64, minutes: Option<i64>) -> (String, String) {
    let task = ops::create(tx, HUMAN, NewTask { title: "t".into(), ..Default::default() }, at / 1000).unwrap();
    let id = runs::add(tx, &Run { agent_id: agent.into(), agent_name: agent.to_uppercase(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: project.into(), ..Default::default() },
        std::slice::from_ref(&task.key), at / 1000).unwrap();
    input(tx, at, DISPATCHER, "dispatcher.start", "k9", None, Some(&id), None);
    if let Some(m) = minutes {
        runs::change(tx, &id, |r| { r.state = Phase::Closed; r.last_walk = at / 1000 + m * 60; }).unwrap();
    }
    (id, task.key)
}

#[test]
fn ac170_dispatcher_s_sessions_failures_and_load() {
    let s = store("dashboard-system");
    let noon = noon(&s);
    s.with(|tx| {
        tx.execute("INSERT INTO agents (id, name, kind, command, color, at_once) VALUES ('a', 'Opus', 'claude', 'claude', 0, 2)", []).map_err(sql)?;
        let (_, done) = run(tx, "a", "default", noon - 3 * HOUR, Some(60));
        let (_, failed) = run(tx, "a", "default", noon - 2 * HOUR, Some(30));
        let (_, by_hand) = run(tx, "gone", "default", noon - HOUR, None);
        let at = noon / 1000;
        ops::move_to(tx, DISPATCHER, &done, "DONE", at - 7000)?;
        for (i, why) in ["The agent's process ended: exit 2.", "The agent's process ended: exit 137."].iter().enumerate() {
            ops::move_to(tx, DISPATCHER, &failed, "FAILED", at - 5000 + i as i64 * 10)?;
            ops::log(tx, &failed, DISPATCHER, "why", why, None, at - 5000 + i as i64 * 10)?;
            ops::move_to(tx, HUMAN, &failed, "TODO", at - 4995 + i as i64 * 10)?;
        }
        ops::move_to(tx, HUMAN, &by_hand, "FAILED", at - 60)?;
        input(tx, noon - 10 * M, HUMAN, "page", "k1", None, None, Some("This pane has closed."));
        input(tx, noon - 40 * DAY, HUMAN, "page", "k1", None, None, None);
        Ok(())
    }).unwrap();
    let d = ask(&s, noon, "system", Period::Day, None);
    let tiles = &d["tiles"];
    assert_eq!((tiles["sessions"].clone(), tiles["done"].clone(), tiles["failed"].clone(), tiles["failure_rate"].clone()), (json!(3), json!(1), json!(2), json!(2.0 / 3.0)), "a human's own move to FAILED is no failure of the system");
    assert_eq!(d["failures"], json!([{"kind": "task", "why": "The agent's process ended", "n": 2}, {"kind": "write", "why": "This pane has closed", "n": 1}]), "reasons by their class, most first");
    assert_eq!((tiles["errors"].clone(), tiles["failed_writes"].clone(), tiles["session_ms"].clone()), (json!(3), json!(1), json!(60 * M)));
    // 60 + 30 minutes of a's two slots, an hour of the one slot of an agent no longer in the pool, over 24 hours less what of the first hour is to come
    let whole = 23 * HOUR + 1;
    assert_eq!(tiles["slots"], 3);
    assert_eq!(tiles["busy_ms"], 150 * M);
    assert_eq!(tiles["load"], (150.0 * M as f64 / (whole * 3) as f64 * 100.0).round() as i64);
    assert_eq!(d["agents"][0], json!({"id": "a", "name": "A", "color": 0, "busy_ms": 90 * M, "sessions": 2, "slots": 2, "load": (90.0 * M as f64 / (whole * 2) as f64 * 100.0).round() as i64}));
    assert_eq!((d["series"][20]["started"].clone(), d["series"][20]["load"].clone(), d["series"][23]["load"].clone()), (json!(1), json!(33), json!(0)), "an hour of one of three slots; the hour that began at noon holds nothing yet");
    assert_eq!(class("Memory limit of 8 GiB exceeded: 12.0 GiB"), "Memory limit of N GiB exceeded");
    assert_eq!(class(""), "No reason said");
    assert_ne!(d["before"], Value::Null, "the log reaches the day before");
    assert_eq!(ask(&s, noon, "system", Period::Day, Some("p1"))["tiles"]["sessions"], 0);
}

#[test]
fn ac171_what_dispatcher_s_sessions_spent() {
    let s = store("dashboard-costs");
    let noon = noon(&s);
    let one = |model: &str, input: i64, output: i64| BTreeMap::from([(model.to_string(), Totals { input, output, ..Totals::default() })]);
    s.with(|tx| {
        tx.execute("INSERT INTO agents (id, name, kind, command, color, at_once, week_limit) VALUES ('a', 'Opus', 'claude', 'claude', 4, 1, 5)", []).map_err(sql)?;
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('p1', 'Api', 3, '/srv', 1)", []).map_err(sql)?;
        let (first, _) = run(tx, "a", "p1", noon - HOUR, Some(30));
        let (second, _) = run(tx, "b", "default", noon - HOUR, Some(30));
        usage::heard(tx, &first, "a", &one("claude-opus-5-5", 1_000_000, 100_000), noon / 1000 - 3000, None)?;
        usage::heard(tx, &second, "b", &one("some-model", 500, 0), noon / 1000 - 2000, None)?;
        usage::heard(tx, &first, "a", &one("claude-opus-5-5", 1_000_000, 100_000), (noon - 3 * DAY) / 1000, None)?;
        Ok(())
    }).unwrap();
    let d = ask(&s, noon, "costs", Period::Day, None);
    let tiles = &d["tiles"];
    assert_eq!((tiles["counted"].clone(), tiles["usd"].clone(), tiles["unpriced"].clone(), tiles["sessions"].clone(), tiles["tasks"].clone(), tiles["usd_per_task"].clone()),
        (json!(1_100_500), json!(6.0), json!(500), json!(2), json!(2), json!(3.0)), "a model with no price adds tokens and no dollars");
    assert_eq!(d["projects"][0], json!({"id": "p1", "name": "Api", "color": 3, "counted": 1_100_000, "cache_read": 0, "usd": 6.0, "unpriced": 0}));
    assert_eq!((d["agents"][0]["name"].clone(), d["agents"][1]["name"].clone(), d["models"][1]["model"].clone()), (json!("Opus"), json!("B"), json!("some-model")), "the pool's name, else the run's");
    assert_eq!((d["series"][23 - 1]["by_usd"].clone(), d["stacked"].clone()), (json!({"a": 6.0, "b": 0.0}), json!(["a", "b"])));
    assert_eq!(d["limits"], json!([{"id": "a", "name": "Opus", "color": 4, "week": 1_100_000, "month": 1_100_000, "week_limit": 5.0, "month_limit": null}]));
    assert_eq!(d["before"], Value::Null, "a report does not add the same totals twice: nothing earlier is kept");
    assert_eq!(ask(&s, noon, "costs", Period::Day, Some("p1"))["tiles"]["counted"], 1_100_000);
    assert_eq!(ask(&s, noon, "costs", Period::Day, Some(""))["tiles"]["counted"], 500);
}
