// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;

/// The clock in UTC: what a test reads does not hang on the machine's zone.
fn utc(at: i64) -> Option<Local> {
    let t: libc::time_t = at;
    // SAFETY: gmtime_r writes only the tm given
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    (!unsafe { libc::gmtime_r(&t, &mut tm) }.is_null()).then(|| of_tm(&tm))
}

/// 2026-11-09, a Monday, 20:35:00 UTC.
const MON: i64 = 1_794_256_500;
const MIN: i64 = 60;
const HOUR: i64 = 3600;
const DAY: i64 = 86_400;

fn next(cron: &str, after: i64) -> i64 {
    Cron::parse(cron).unwrap().next_by(after, utc).unwrap()
}

#[test]
fn ac160_the_kinds_of_cadence_the_window_writes() {
    assert_eq!(next("*/15 * * * *", MON), MON + 10 * MIN, "every 15 minutes: 20:45");
    assert_eq!(next("5 */2 * * *", MON), MON + 90 * MIN, "every 2 hours at :05: 22:05");
    assert_eq!(next("0 9 * * *", MON), MON - 20 * HOUR - 35 * MIN + DAY + 9 * HOUR, "daily at 09:00: tomorrow");
    assert_eq!(next("30 20 * * *", MON - 10 * MIN), MON - 5 * MIN, "today, still ahead");
    assert_eq!(next("0 9 * * 1-5", MON + 4 * DAY), MON - 20 * HOUR - 35 * MIN + 7 * DAY + 9 * HOUR, "weekdays: Friday evening leads to Monday");
    assert_eq!(next("0 9 * * 0,6", MON), MON - 20 * HOUR - 35 * MIN + 5 * DAY + 9 * HOUR, "weekends: Saturday");
    assert_eq!(next("0 8 1 * *", MON), 1_796_083_200 + 8 * HOUR, "monthly on the 1st: December 1");
    assert_eq!(next("0 0 * * 7", MON), next("0 0 * * 0", MON), "7 is Sunday too");
}

#[test]
fn ac160_the_next_minute_is_strictly_after() {
    assert_eq!(next("35 20 * * *", MON), MON + DAY, "the minute now is past");
    assert_eq!(next("* * * * *", MON + 30), MON + MIN, "seconds into a minute: the next one");
}

#[test]
fn ac160_both_days_named_either_matches() {
    // the 13th or a Friday, whichever comes first
    assert_eq!(next("0 0 13 * 5", MON), MON - 20 * HOUR - 35 * MIN + 4 * DAY, "the 13th, a Friday too");
    assert_eq!(next("0 0 10 * 5", MON), MON - 20 * HOUR - 35 * MIN + DAY, "the 10th before Friday");
}

#[test]
fn ac160_a_rare_day_is_found_and_an_impossible_one_is_none() {
    let leap = Cron::parse("0 0 29 2 *").unwrap().next_by(MON, utc).unwrap();
    assert_eq!(leap, 1_835_395_200, "2028-02-29");
    assert_eq!(Cron::parse("0 0 31 2 *").unwrap().next_by(MON, utc), None, "February 31");
}

#[test]
fn ac160_a_wrong_cadence_says_what_is_wrong() {
    let says = |c: &str| Cron::parse(c).unwrap_err();
    assert_eq!(says("* * * *"), "A cadence is five fields: minute, hour, day of the month, month, day of the week.");
    assert_eq!(says("60 * * * *"), "The minute \"60\" is not 0 to 59.");
    assert_eq!(says("* 24 * * *"), "The hour \"24\" is not 0 to 23.");
    assert_eq!(says("* * 0 * *"), "The day of the month \"0\" is not 1 to 31.");
    assert_eq!(says("*/0 * * * *"), "The minute's step \"0\" is not a number over 0.");
    assert_eq!(says("* 9-5 * * *"), "The hour's range 9-5 runs backwards.");
    assert_eq!(says("* * * * mon"), "The day of the week \"mon\" is not 0 to 7.");
}

#[test]
fn ac162_a_minute_already_read_never_comes_again() {
    // at 01:35 the clock goes back an hour: 00:35 to 01:35 is read twice, and 01:30 comes once
    fn back(at: i64) -> Option<Local> {
        const TURN: i64 = MON + 5 * HOUR;
        utc(if at >= TURN { at - HOUR } else { at })
    }
    let c = Cron::parse("30 1 * * *").unwrap();
    let first = c.next_by(MON, back).unwrap();
    assert_eq!(first, MON + 4 * HOUR + 55 * MIN, "the first 01:30");
    let second = c.next_by(first, back).unwrap();
    assert!(second > MON + DAY, "not in the hour passed again: {}", second - MON);
}

#[test]
fn ac160_a_cadence_in_words() {
    assert_eq!(words("*/15 * * * *"), "Every 15 minutes");
    assert_eq!(words("5 */2 * * *"), "Every 2 hours at :05");
    assert_eq!(words("0 3 * * *"), "Every day at 03:00");
    assert_eq!(words("30 9 * * 1-5"), "Weekdays at 09:30");
    assert_eq!(words("0 9 * * 1,3,5"), "Every Mon, Wed, Fri at 09:00");
    assert_eq!(words("0 8 1 * *"), "Every month on day 1 at 08:00");
    assert_eq!(words("0 8  1 1 *"), "Cron 0 8 1 1 *");
}
