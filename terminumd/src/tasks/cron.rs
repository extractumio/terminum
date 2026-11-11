// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A schedule's cadence (`terminum-schedules.md` AC-160): five cron fields in the Mac's local time, minute, hour,
//! day of the month, month, day of the week, each `*`, a number, `a-b`, a list by commas, a step by `/`; a day of
//! the week 0 to 7, 0 and 7 both Sunday. As cron does, when both days are named either one matches. A minute the
//! clock skips (summer time) does not come that day; one it passes twice comes once.

/// One cadence, parsed.
#[derive(Clone, Debug, PartialEq)]
pub struct Cron {
    minute: u64,
    hour: u64,
    day: u64,
    month: u64,
    weekday: u64,
    /// the day of the month or of the week is not `*`: cron's "either" holds when both are named
    day_named: bool,
    weekday_named: bool,
}

/// The least and the most of each field, and its name in a refusal.
const FIELDS: [(u32, u32, &str); 5] = [(0, 59, "minute"), (0, 23, "hour"), (1, 31, "day of the month"), (1, 12, "month"), (0, 7, "day of the week")];

/// The longest a cadence is looked ahead for its next minute: a February 29 on a Monday is decades off.
const MOST_DAYS: i64 = 366 * 8;

fn field(text: &str, (least, most, name): (u32, u32, &str)) -> Result<u64, String> {
    let mut bits = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().ok().filter(|s| *s > 0).ok_or_else(|| format!("The {name}'s step \"{s}\" is not a number over 0."))?),
            None => (part, 1),
        };
        let number = |n: &str| n.parse::<u32>().ok().filter(|n| (least..=most).contains(n)).ok_or_else(|| format!("The {name} \"{n}\" is not {least} to {most}."));
        let (from, to) = match range {
            "*" => (least, most),
            r => match r.split_once('-') {
                Some((a, b)) => (number(a)?, number(b)?),
                None if part.contains('/') => (number(r)?, most),
                None => { let n = number(r)?; (n, n) }
            },
        };
        if from > to {
            return Err(format!("The {name}'s range {range} runs backwards."));
        }
        for n in (from..=to).step_by(step as usize) {
            bits |= 1 << n;
        }
    }
    Ok(bits)
}

impl Cron {
    pub fn parse(text: &str) -> Result<Cron, String> {
        let parts: Vec<&str> = text.split_whitespace().collect();
        if parts.len() != 5 {
            return Err("A cadence is five fields: minute, hour, day of the month, month, day of the week.".into());
        }
        let mut weekday = field(parts[4], FIELDS[4])?;
        if weekday & (1 << 7) != 0 {
            weekday = (weekday | 1) & !(1 << 7);    // 7 is Sunday too
        }
        Ok(Cron {
            minute: field(parts[0], FIELDS[0])?,
            hour: field(parts[1], FIELDS[1])?,
            day: field(parts[2], FIELDS[2])?,
            month: field(parts[3], FIELDS[3])?,
            weekday,
            day_named: parts[2] != "*",
            weekday_named: parts[4] != "*",
        })
    }

    fn day_matches(&self, t: &Local) -> bool {
        let (day, weekday) = (self.day & (1 << t.day) != 0, self.weekday & (1 << t.weekday) != 0);
        match (self.day_named, self.weekday_named) {
            (true, true) => day || weekday,
            _ => day && weekday,
        }
    }

    /// The first minute after `after` (unix seconds) the cadence names, in unix seconds; none within years.
    pub fn next_after(&self, after: i64) -> Option<i64> {
        self.next_by(after, local)
    }

    /// `next_after` by the clock `read`.
    fn next_by(&self, after: i64, read: fn(i64) -> Option<Local>) -> Option<i64> {
        // a local minute comes once: in an hour the clock passes twice, what it already read is passed over
        let floor = read(after)?.wall();
        let mut at = after - after.rem_euclid(60) + 60;
        let end = after + MOST_DAYS * 86_400;
        while at <= end {
            let t = read(at)?;
            if self.month & (1 << t.month) == 0 || !self.day_matches(&t) {
                at += (24 - t.hour) * 3600 - t.minute * 60;    // to the next local midnight, near enough: read again there
                continue;
            }
            if self.hour & (1 << t.hour) == 0 {
                at += (60 - t.minute) * 60;
                continue;
            }
            if self.minute & (1 << t.minute) != 0 && t.wall() > floor {
                return Some(at);
            }
            at += 60;
        }
        None
    }
}

/// A moment as the Mac's clock reads it.
struct Local {
    year: i64,
    minute: i64,
    hour: i64,
    day: i64,
    month: i64,
    weekday: i64,
}

impl Local {
    /// The wall clock's reading, in its order.
    fn wall(&self) -> (i64, i64, i64, i64, i64) {
        (self.year, self.month, self.day, self.hour, self.minute)
    }

    fn clock(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `at` (unix seconds) on the Mac's clock: "Tue 10 Nov 03:00".
pub fn when_words(at: i64) -> String {
    local(at).map_or_else(|| at.to_string(), |t| format!("{} {} {} {}", DAYS[t.weekday as usize], t.day, MONTHS[(t.month - 1) as usize], t.clock()))
}

/// A cadence in words, for the kinds the schedule's window writes (AC-160); any other is its fields.
pub fn words(text: &str) -> String {
    let f: Vec<&str> = text.split_whitespace().collect();
    let num = |s: &str| s.parse::<u32>().ok();
    let at = |h: &str, m: &str| num(h).zip(num(m)).map(|(h, m)| format!("{h:02}:{m:02}"));
    let days = |list: &str| -> Option<String> {
        let names: Option<Vec<&str>> = list.split(',').map(|d| num(d).filter(|d| *d <= 7).map(|d| DAYS[(d % 7) as usize])).collect();
        names.map(|n| n.join(", "))
    };
    let said = match f.as_slice() {
        [m, "*", "*", "*", "*"] if m.starts_with("*/") => m[2..].parse::<u32>().ok().map(|n| if n == 1 { "Every minute".into() } else { format!("Every {n} minutes") }),
        [m, h, "*", "*", "*"] if h.starts_with("*/") => num(m).zip(h[2..].parse::<u32>().ok()).map(|(m, n)| format!("Every {n} hours at :{m:02}")),
        [m, h, "*", "*", "*"] => at(h, m).map(|t| format!("Every day at {t}")),
        [m, h, "*", "*", "1-5"] => at(h, m).map(|t| format!("Weekdays at {t}")),
        [m, h, "*", "*", d] => at(h, m).zip(days(d)).map(|(t, d)| format!("Every {d} at {t}")),
        [m, h, d, "*", "*"] => at(h, m).zip(num(d)).map(|(t, d)| format!("Every month on day {d} at {t}")),
        _ => None,
    };
    said.unwrap_or_else(|| format!("Cron {}", f.join(" ")))
}

/// `at` (unix s) as the Mac's clock reads it: "2026-11-11 10:49".
pub fn stamp(at: i64) -> String {
    local(at).map_or_else(String::new, |l| format!("{:04}-{:02}-{:02} {:02}:{:02}", l.year, l.month, l.day, l.hour, l.minute))
}

fn local(at: i64) -> Option<Local> {
    let t: libc::time_t = at;
    // SAFETY: localtime_r writes only the tm given, from a time_t it reads
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return None;
    }
    Some(of_tm(&tm))
}

fn of_tm(tm: &libc::tm) -> Local {
    Local { year: (tm.tm_year + 1900).into(), minute: tm.tm_min.into(), hour: tm.tm_hour.into(), day: tm.tm_mday.into(), month: (tm.tm_mon + 1).into(), weekday: tm.tm_wday.into() }
}

#[cfg(test)]
#[path = "cron_tests.rs"]
mod tests;
