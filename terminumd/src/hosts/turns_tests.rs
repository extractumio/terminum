// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;

// AC-99 edge — on and off asked while the other runs; AC-83, AC-99: Files' operations in the same turn

/// Records its start and its end, or that it was stopped (dropped) before ending.
struct Step<'a>(&'a Mutex<Vec<String>>, &'static str, bool);

impl Drop for Step<'_> {
    fn drop(&mut self) {
        if !self.2 {
            self.0.lock().push(format!("{} stopped", self.1));
        }
    }
}

async fn after_ms(n: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(n)).await;
}

async fn step(log: &Mutex<Vec<String>>, name: &'static str, n: u64) -> Result<&'static str, String> {
    let mut s = Step(log, name, false);
    log.lock().push(format!("{name} began"));
    after_ms(n).await;
    log.lock().push(format!("{name} ended"));
    s.2 = true;
    Ok(name)
}

#[tokio::test]
async fn an_off_during_a_long_on_stops_it_before_it_runs() {
    let (turns, log) = (Turns::default(), Mutex::new(Vec::new()));
    let on = turns.run("devbox", step(&log, "on", 5_000));
    let off = async { after_ms(50).await; turns.run("devbox", step(&log, "off", 10)).await };
    let (on, off) = tokio::join!(on, off);
    assert_eq!((on, off), (Err(SUPERSEDED.to_string()), Ok("off")));
    assert_eq!(*log.lock(), ["on began", "on stopped", "off began", "off ended"], "the on never saves after the off");
}

#[tokio::test]
async fn the_last_asked_wins_and_one_waiting_meanwhile_never_runs() {
    let (turns, log) = (Turns::default(), Mutex::new(Vec::new()));
    let later = |ms, name, work| { let (turns, log) = (&turns, &log); async move { after_ms(ms).await; turns.run("devbox", step(log, name, work)).await } };
    // each later one stops the one running
    let (a, b, c) = tokio::join!(later(0, "on", 5_000), later(30, "off", 5_000), later(60, "on again", 10));
    assert_eq!((a.is_err(), b.is_err(), c), (true, true, Ok("on again")));
    assert_eq!(*log.lock(), ["on began", "on stopped", "off began", "off stopped", "on again began", "on again ended"]);
}

#[tokio::test]
async fn one_waiting_when_a_later_one_is_asked_never_begins() {
    let (turns, log) = (Arc::new(Turns::default()), Arc::new(Mutex::new(Vec::new())));
    let turn = turns.0.lock().entry("devbox".into()).or_default().clone();
    let ending = turn.running.lock().await;                     // a turn still ending
    let ask = |name| { let (turns, log) = (turns.clone(), log.clone()); tokio::spawn(async move { turns.run("devbox", step(&log, name, 10)).await }) };
    let off = ask("off");
    after_ms(20).await;
    let on = ask("on again");
    after_ms(20).await;
    drop(ending);
    assert_eq!((off.await.unwrap(), on.await.unwrap()), (Err(SUPERSEDED.to_string()), Ok("on again")));
    assert_eq!(*log.lock(), ["on again began", "on again ended"]);
}

#[tokio::test]
async fn another_hosts_turn_runs_beside_it() {
    let (turns, log) = (Turns::default(), Mutex::new(Vec::new()));
    let (a, b) = tokio::join!(turns.run("devbox", step(&log, "devbox on", 100)), turns.run("other", step(&log, "other off", 10)));
    assert_eq!((a, b), (Ok("devbox on"), Ok("other off")));
}

#[tokio::test]
async fn a_refresh_waits_for_the_one_running_and_a_later_on_or_off_stops_it() {
    let (turns, log) = (Turns::default(), Mutex::new(Vec::new()));
    // after an on, left to end: the refresh runs next
    let on = turns.run("devbox", step(&log, "on", 100));
    let refresh = async { after_ms(20).await; assert!(turns.busy("devbox")); turns.after("devbox", step(&log, "refresh", 10)).await };
    assert_eq!(tokio::join!(on, refresh), (Ok("on"), Ok("refresh")));
    assert_eq!(*log.lock(), ["on began", "on ended", "refresh began", "refresh ended"]);
    assert!(!turns.busy("devbox"));
    // a refresh running, an off asked: the refresh stops
    log.lock().clear();
    let refresh = turns.after("devbox", step(&log, "refresh", 5_000));
    let off = async { after_ms(20).await; turns.run("devbox", step(&log, "off", 10)).await };
    let (refresh, off) = tokio::join!(refresh, off);
    assert_eq!((refresh.is_err(), off), (true, Ok("off")));
    assert_eq!(*log.lock(), ["refresh began", "refresh stopped", "off began", "off ended"]);
}

#[tokio::test(start_paused = true)]
async fn an_agents_off_waits_for_files_operation_and_never_stops_it() {
    // AC-83, AC-99: Files' enable (its copy) ends before the off's removal of the helper begins
    let (turns, log) = (Turns::default(), Mutex::new(Vec::new()));
    let enable = turns.hold("devbox", step(&log, "files enable", 100));
    let off = async { after_ms(20).await; assert!(turns.busy("devbox")); turns.run("devbox", step(&log, "agents off", 10)).await };
    assert_eq!(tokio::join!(enable, off), (Ok("files enable"), Ok("agents off")));
    assert_eq!(*log.lock(), ["files enable began", "files enable ended", "agents off began", "agents off ended"]);
}

#[tokio::test(start_paused = true)]
async fn files_operation_waits_for_an_agents_on_and_never_stops_it() {
    // AC-83, AC-99: Files' removal never runs beside an agents on's copy
    let (turns, log) = (Turns::default(), Mutex::new(Vec::new()));
    let on = turns.run("devbox", step(&log, "agents on", 100));
    let removal = async { after_ms(20).await; turns.hold("devbox", step(&log, "files removal", 10)).await };
    assert_eq!(tokio::join!(on, removal), (Ok("agents on"), Ok("files removal")));
    assert_eq!(*log.lock(), ["agents on began", "agents on ended", "files removal began", "files removal ended"]);
}

#[tokio::test(start_paused = true)]
async fn files_operations_and_the_helpers_update_run_in_the_order_asked() {
    // AC-83: a removal, an enable and an update asked while an enable runs
    let (turns, log) = (Turns::default(), Mutex::new(Vec::new()));
    let later = |ms, name| { let (turns, log) = (&turns, &log); async move { after_ms(ms).await; turns.hold("devbox", step(log, name, 30)).await } };
    let (a, b, c) = tokio::join!(later(0, "enable"), later(10, "removal"), later(20, "update"));
    assert_eq!((a, b, c), (Ok("enable"), Ok("removal"), Ok("update")));
    assert_eq!(*log.lock(), ["enable began", "enable ended", "removal began", "removal ended", "update began", "update ended"]);
}

#[tokio::test(start_paused = true)]
async fn a_later_agents_off_stops_the_on_waiting_behind_files_operation() {
    // AC-99: the last asked of on and off wins also while both wait for Files' turn to end
    let (turns, log) = (Turns::default(), Mutex::new(Vec::new()));
    let enable = turns.hold("devbox", step(&log, "files enable", 100));
    let on = async { after_ms(20).await; turns.run("devbox", step(&log, "on", 10)).await };
    let off = async { after_ms(40).await; turns.run("devbox", step(&log, "off", 10)).await };
    let (enable, on, off) = tokio::join!(enable, on, off);
    assert_eq!((enable, on, off), (Ok("files enable"), Err(SUPERSEDED.to_string()), Ok("off")));
    assert_eq!(*log.lock(), ["files enable began", "files enable ended", "off began", "off ended"]);
}
