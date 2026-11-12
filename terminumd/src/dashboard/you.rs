// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The Dashboard's You tab (terminum.md AC-152): how much the human works here and how loaded, from what
//! they sent and typed (Dispatcher's never) and what they did on the board. Active time: those events in
//! time order; a pause over GAP ends a stretch, which lasts from its first to its last event and TAIL more
//! (reading the answer); the time after an event is its project's. From the interaction log's first row on
//! (AC-175) the events are also what the human did in a page, a pause joins its stretch when no longer than
//! NEAR, or than GAP with a page in front through it, the tail is TAIL_SEEN, and engagement is counted: the
//! clock minutes of active time in which the human acted or scrolled.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::Transaction;
use serde_json::{json, Value};

use super::{civil, local, median, stack_key, stacked, Ask, Span, DAY, HOUR, MINUTE};
use crate::tasks::ops::rows;

pub const GAP: i64 = 15 * 60_000;
pub const TAIL: i64 = 2 * 60_000;
pub const NEAR: i64 = 5 * 60_000;
pub const TAIL_SEEN: i64 = 60_000;
/// A minute with a page in front this many seconds was in front.
const IN_FRONT: i64 = 30;
/// The project of what a page with no pane counted: the one of what was done before it.
const UNKNOWN: u32 = u32::MAX;
/// How an event engages: the pointer moved (presence only), the human scrolled (reading), or acted (a key, a
/// press, a drag, something sent, typed or done on the board).
#[derive(Clone, Copy, PartialEq)]
enum How { Here, Read, Act }

/// The active clock minutes from the pages' first count on, by what the human did in them.
#[derive(Clone, Copy, Default)]
struct Minutes { active: i64, acting: i64, reading: i64 }

impl Minutes {
    /// For the page, with the minutes only watched and the share engaged; none where nothing was measured.
    fn json(&self) -> Value {
        if self.active == 0 {
            return Value::Null;
        }
        json!({"active": self.active, "acting": self.acting, "reading": self.reading, "watching": self.active - self.acting - self.reading,
               "share": (self.acting + self.reading) as f64 / self.active as f64})
    }
}
/// A bucket's switches an active hour are counted from this many active minutes on: a few minutes cannot spike.
const RATED_MINUTES: i64 = 30;
/// A bucket switching more than this many times its usual rate stands out.
const OVER_USUAL: f64 = 1.5;
/// What the human does on the board, as the events' kinds say it.
const BOARD: [&str; 5] = ["made", "move", "comment", "edit", "archive"];
/// The pane of what is done on the board: none.
const NO_PANE: u32 = 0;

/// Something the human did: local ms, the project and the pane (numbers of `Names`: a year of the input log
/// is a million of these), whether it sent (a message, an Enter, a paste), whether a page on a phone sent it
/// (None: no page did), how it engages.
#[derive(Clone, Copy)]
struct Event { t: i64, project: u32, pane: u32, sent: bool, phone: Option<bool>, how: How }

/// Each text met, by its number; `""` is 0.
struct Names { ids: HashMap<String, u32>, list: Vec<String> }

impl Names {
    fn new() -> Names {
        Names { ids: HashMap::from([(String::new(), 0)]), list: vec![String::new()] }
    }

    fn id(&mut self, text: String) -> u32 {
        if let Some(id) = self.ids.get(&text) {
            return *id;
        }
        self.list.push(text.clone());
        self.ids.insert(text, self.list.len() as u32 - 1);
        self.list.len() as u32 - 1
    }
}

/// What is read once for both spans.
struct Read {
    events: Vec<Event>,
    /// the projects' ids by their numbers
    projects: Vec<String>,
    /// the human's board events: local ms, kind
    board: Vec<(i64, String)>,
    /// a task's waits for a human: local ms it began, how long until it was moved on (None: it waits)
    waits: Vec<(i64, Option<i64>)>,
    names: HashMap<String, String>,
    /// the local minutes a page was in front, in order
    front: Vec<i64>,
    /// the interaction log's first row, local ms: from it on the pages' counts are events
    measured: Option<i64>,
}

pub fn numbers(tx: &Transaction, ask: &Ask) -> Result<Value, String> {
    let read = read(tx, ask)?;
    let mut now = summarize(&read, ask.span, ask);
    // a span from before the pages counted is measured another way: not compared
    let before = ask.before.filter(|b| read.measured.is_none_or(|m| m <= b.lo()));
    now["before"] = before.map_or(Value::Null, |b| summarize(&read, b, ask)["tiles"].take());
    now["measured_from"] = json!(read.measured.map(|m| civil(m.div_euclid(DAY))));
    // the rules the page draws and words by
    now["rules"] = json!({"night": NIGHT, "rated_minutes": RATED_MINUTES, "over_usual": OVER_USUAL});
    Ok(now)
}

fn read(tx: &Transaction, ask: &Ask) -> Result<Read, String> {
    let only = "AND (?2 IS NULL OR coalesce(project, '') = ?2)";
    let mut names = super::project_names(tx)?;
    let (mut projects, mut panes) = (Names::new(), Names::new());
    let mut events = Vec::new();
    let msgs: Vec<(i64, String, String, String)> = rows(tx, &format!("SELECT {}, coalesce(project, ''), coalesce(project_name, ''), pane FROM messages
        WHERE sender = 'human' AND at >= ?1 {only}", local("at")), (ask.since, ask.project), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    for (t, project, name, pane) in msgs {
        if !name.is_empty() && !names.contains_key(&project) { names.insert(project.clone(), name); }
        events.push(Event { t, project: projects.id(project), pane: panes.id(pane), sent: true, phone: None, how: How::Act });
    }
    // a row is its first and its last key; the first one says whether it sent and from where
    let sent = "component = 'page' AND (kind = 'paste' OR (kind = 'typed' AND text LIKE '%<Enter>'))";
    // a client that sent no User-Agent has no browser: not a phone
    let phone = "coalesce(device, '') = 'web' AND (coalesce(browser, '') LIKE '%iPhone%' OR coalesce(browser, '') LIKE '%Android%')";
    let mut st = tx.prepare_cached(&format!("SELECT {}, {}, coalesce(project, ''), pane, {sent}, {phone} FROM inputs
        WHERE sender = 'human' AND at >= ?1 {only}", local("at"), local("ended"))).map_err(crate::tasks::ops::sql)?;
    let mut found = st.query((ask.since, ask.project)).map_err(crate::tasks::ops::sql)?;
    while let Some(r) = found.next().map_err(crate::tasks::ops::sql)? {
        let get = |e: rusqlite::Error| crate::tasks::ops::sql(e);
        let (t, ended, sent): (i64, i64, bool) = (r.get(0).map_err(get)?, r.get(1).map_err(get)?, r.get(4).map_err(get)?);
        let (project, pane) = (projects.id(r.get(2).map_err(get)?), panes.id(r.get(3).map_err(get)?));
        if ended > t { events.push(Event { t: ended, project, pane, sent: false, phone: None, how: How::Act }); }
        events.push(Event { t, project, pane, sent, phone: if sent { Some(r.get(5).map_err(get)?) } else { None }, how: How::Act });
    }
    // what a page counted in a minute: its first and its last; the minutes in front are no events, and no project's
    let counted: Vec<(i64, i64, String, Option<String>)> = rows(tx, &format!("SELECT {}, {}, kind, project FROM user_interaction
        WHERE at >= ?1 AND ((?2 IS NULL OR project = ?2) AND kind != 'front' OR kind = 'front' AND amount >= {IN_FRONT})", local("at"), local("ended")), (ask.since, ask.project), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    let mut front = Vec::new();
    for (t, ended, kind, project) in counted {
        if kind == "front" {
            front.push(t.div_euclid(MINUTE));
            continue;
        }
        let how = match kind.as_str() { "move" => How::Here, "scroll" => How::Read, _ => How::Act };
        let project = project.map_or(UNKNOWN, |p| projects.id(p));
        for t in [t, ended].into_iter().take(if ended > t { 2 } else { 1 }) {
            events.push(Event { t, project, pane: NO_PANE, sent: false, phone: None, how });
        }
    }
    front.sort_unstable();
    front.dedup();
    let measured: Option<i64> = tx.query_row(&format!("SELECT {} FROM (SELECT min(at) AS at FROM user_interaction) WHERE at IS NOT NULL", local("at")), [], |r| r.get(0)).ok();
    let at = local("e.at * 1000");
    let mine = "(?2 IS NULL OR t.project = ?2)";
    let board: Vec<(i64, String, String)> = rows(tx, &format!("SELECT {at}, e.kind, CASE WHEN p.is_default THEN '' ELSE p.id END FROM events e JOIN tasks t ON t.id = e.task JOIN projects p ON p.id = t.project
        WHERE e.author = 'human' AND e.at >= ?1 / 1000 AND {mine}"), (ask.since, ask.task_project()), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let board: Vec<(i64, String)> = board.into_iter().map(|(t, kind, project)| {
        events.push(Event { t, project: projects.id(project), pane: NO_PANE, sent: false, phone: None, how: How::Act });
        (t, kind)
    }).collect();
    // a wait: a move to WAIT FOR HUMAN, until the task's next move
    let waits = rows(tx, &format!("SELECT {at}, (SELECT min(o.at) FROM events o WHERE o.task = e.task AND o.kind = 'move' AND o.at >= e.at AND o.id != e.id AND json_extract(o.data, '$.from') = 'WAIT FOR HUMAN') - e.at
        FROM events e JOIN tasks t ON t.id = e.task WHERE e.kind = 'move' AND json_extract(e.data, '$.to') = 'WAIT FOR HUMAN' AND e.at >= ?1 / 1000 AND {mine}"),
        (ask.since, ask.task_project()), |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.map(|s| s * 1000))))?;
    events.sort_by_key(|e| e.t);
    let mut last: Option<(i64, u32)> = None;
    for e in &mut events {
        if e.project == UNKNOWN {
            e.project = last.filter(|l| e.t - l.0 <= GAP).map_or(0, |l| l.1);
        }
        last = Some((e.t, e.project));
    }
    Ok(Read { events, projects: projects.list, board, waits, names, front, measured })
}

/// The night's hours: from 22:00 to 06:00.
const NIGHT: [usize; 2] = [22, 6];

/// Whether an hour of weekday `wd` (0: Sunday) is outside a working day: a weekend, or the night.
fn off_hours(wd: usize, hour: usize) -> bool {
    wd == 0 || wd == 6 || !(NIGHT[1]..NIGHT[0]).contains(&hour)
}

/// Every number of `span`.
fn summarize(read: &Read, span: &Span, ask: &Ask) -> Value {
    let (first, last) = (read.events.partition_point(|e| e.t < span.lo()), read.events.partition_point(|e| e.t <= span.until));
    let evs = &read.events[first..last];
    let n = span.len();
    let mut active = vec![0i64; n];
    let mut by: Vec<HashMap<u32, i64>> = vec![HashMap::new(); n];
    let mut per_project: HashMap<u32, i64> = HashMap::new();
    let mut spans: Vec<Vec<[i64; 2]>> = vec![Vec::new(); n];
    let mut heat = [[0i64; 24]; 7];
    let (mut off, mut longest, mut began) = (vec![0i64; n], 0i64, None);
    let mut active_units: HashSet<i64> = HashSet::new();
    // from the pages' first count on, the clock minutes: the last one counted active; the one going on with
    // its bucket and whether the human acted in it (else they scrolled)
    let mut minutes = vec![Minutes::default(); n];
    let mut last_active = i64::MIN;
    let mut going: Option<(i64, usize, bool)> = None;
    let engaged = |minutes: &mut [Minutes], going: Option<(i64, usize, bool)>| {
        if let Some((_, k, acted)) = going { if acted { minutes[k].acting += 1 } else { minutes[k].reading += 1 } }
    };
    let in_front = |a: i64, b: i64| {
        let (lo, hi) = (a.div_euclid(MINUTE) + 1, b.div_euclid(MINUTE));
        (read.front.partition_point(|m| *m < hi) - read.front.partition_point(|m| *m < lo)) as i64 >= hi - lo
    };
    for (i, e) in evs.iter().enumerate() {
        // the time after an event, up to the next one of its stretch, is its project's
        let seen = read.measured.is_some_and(|m| e.t >= m);
        let next = evs.get(i + 1).map(|x| x.t).filter(|x| if seen { x - e.t <= NEAR || (x - e.t <= GAP && in_front(e.t, *x)) } else { x - e.t <= GAP });
        let end = next.unwrap_or(e.t + if seen { TAIL_SEEN } else { TAIL });
        if seen && e.how != How::Here && let Some(k) = span.index(e.t) {
            let m = e.t.div_euclid(MINUTE);
            if going.is_none_or(|g| g.0 != m) {
                engaged(&mut minutes, going.replace((m, k, false)));
            }
            if let Some(g) = going.as_mut() { g.2 |= e.how == How::Act; }
        }
        let start = *began.get_or_insert(e.t);
        if next.is_none() {
            longest = longest.max(end - start);
            began = None;
        }
        let mut t = e.t;
        while t < end {
            let to = ((t.div_euclid(HOUR) + 1) * HOUR).min(end);
            if let Some(k) = span.index(t) {
                let (d, len) = (t.div_euclid(DAY), to - t);
                active[k] += len;
                let (lo, hi) = (t.div_euclid(MINUTE).max(last_active + 1), (to - 1).div_euclid(MINUTE));
                if seen && hi >= lo {
                    minutes[k].active += hi - lo + 1;
                    last_active = hi;
                }
                *by[k].entry(e.project).or_default() += len;
                *per_project.entry(e.project).or_default() += len;
                let (wd, hour) = ((d + 4).rem_euclid(7) as usize, (t.rem_euclid(DAY) / HOUR) as usize);   // 1970-01-01 was a Thursday; 0: Sunday
                heat[wd][hour] += len;
                if off_hours(wd, hour) { off[k] += len; }
                active_units.insert(if span.by_hour() { t.div_euclid(HOUR) } else { d });
                if span.by_day() {
                    let (a, b) = (t.rem_euclid(DAY) / 60_000, (to - d * DAY) / 60_000);
                    match spans[k].last_mut() {
                        Some(last) if last[1] >= a => last[1] = b,
                        _ => spans[k].push([a, b]),
                    }
                }
            }
            t = to;
        }
    }
    let mut weekdays = [0i64; 7];
    for d in span.lo().div_euclid(DAY)..=span.until.div_euclid(DAY) {
        weekdays[(d + 4).rem_euclid(7) as usize] += 1;
    }
    let heat: Vec<Vec<i64>> = heat.iter().enumerate().map(|(wd, row)| row.iter().map(|v| v / 60_000 / weekdays[wd].max(1)).collect()).collect();
    // sessions written to: a bucket's, the most in one hour; a switch: a send to another pane than the send before it, within a stretch
    let mut panes: Vec<HashSet<u32>> = vec![HashSet::new(); n];
    let mut hourly: HashMap<i64, HashSet<u32>> = HashMap::new();
    let mut switches = vec![0i64; n];
    let mut sent = vec![0i64; n];
    let (mut last, mut phone, mut paged): (Option<&Event>, i64, i64) = (None, 0, 0);
    for e in evs.iter().filter(|e| e.pane != NO_PANE) {
        let Some(k) = span.index(e.t) else { continue };
        panes[k].insert(e.pane);
        hourly.entry(e.t.div_euclid(HOUR)).or_default().insert(e.pane);
        if let Some(p) = e.phone {
            paged += 1;
            phone += i64::from(p);
        }
        if e.sent {
            sent[k] += 1;
            if last.is_some_and(|l| l.pane != e.pane && e.t - l.t <= GAP) { switches[k] += 1; }
            last = Some(e);
        }
    }
    let mut peak = vec![0i64; n];
    for (h, set) in &hourly {
        if let Some(k) = span.index(h * HOUR) { peak[k] = peak[k].max(set.len() as i64); }
    }
    // the board
    let mut board: Vec<BTreeMap<&str, i64>> = vec![BTreeMap::new(); n];
    let mut board_by: BTreeMap<&str, i64> = BOARD.iter().map(|k| (*k, 0)).collect();
    for (t, kind) in &read.board {
        if let (Some(k), Some(kind)) = (span.index(*t), BOARD.iter().find(|b| *b == kind)) {
            *board[k].entry(kind).or_default() += 1;
            *board_by.entry(kind).or_default() += 1;
        }
    }
    let waits: Vec<&(i64, Option<i64>)> = read.waits.iter().filter(|w| span.holds(w.0)).collect();
    // projects, most time first
    let id = |project: &u32| read.projects[*project as usize].as_str();
    let mut ranked: Vec<(&str, i64)> = per_project.iter().map(|(p, ms)| (id(p), *ms)).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let stack = stacked(ranked.iter().map(|p| p.0));
    let projects: Vec<Value> = ranked.iter().map(|(id, ms)| {
        let mut p = ask.project_json(id, read.names.get(*id).map_or("", String::as_str));
        p["n"] = json!(ms);
        p
    }).collect();
    engaged(&mut minutes, going);
    let all = minutes.iter().fold(Minutes::default(), |a, m| Minutes { active: a.active + m.active, acting: a.acting + m.acting, reading: a.reading + m.reading });
    // switches an active hour, a bucket's with enough active time; the usual: their median, and those over it
    let rates: Vec<Option<f64>> = (0..n).map(|k| (active[k] >= RATED_MINUTES * MINUTE).then(|| switches[k] as f64 / (active[k] as f64 / HOUR as f64))).collect();
    let usual_rate = median(rates.iter().flatten().copied().collect());
    let over: Vec<bool> = rates.iter().map(|r| r.zip(usual_rate).is_some_and(|(r, u)| r > u * OVER_USUAL)).collect();
    let series: Vec<Value> = (0..n).map(|k| {
        let mut parts: BTreeMap<String, i64> = BTreeMap::new();
        for (p, ms) in &by[k] { *parts.entry(stack_key(&stack, id(p))).or_default() += ms; }
        json!({"active_ms": active[k], "by": parts, "sessions": panes[k].len(), "peak": peak[k], "switches": switches[k], "sent": sent[k],
               "board": board[k], "board_n": board[k].values().sum::<i64>(), "spans": span.by_day().then(|| &spans[k]),
               "engaged": minutes[k].json(), "off_ms": off[k], "switches_per_hour": rates[k],
               "switches_over": over[k]})
    }).collect();
    let total: i64 = active.iter().sum();
    let worked: Vec<&Vec<[i64; 2]>> = spans.iter().filter(|s| !s.is_empty()).collect();
    let all_switches: i64 = switches.iter().sum();
    let tiles = json!({
        "active_ms": total, "active_units": active_units.len(), "units": if span.by_hour() { n as i64 } else { span.days() },
        "active_per_unit_ms": if active_units.is_empty() { 0 } else { total / active_units.len() as i64 },
        "longest_ms": longest,
        "engaged": all.json(),
        "sessions": median(panes.iter().map(|p| p.len() as i64).filter(|p| *p > 0).collect()), "peak": peak.iter().max().copied().unwrap_or(0),
        "switches": all_switches, "switches_per_hour": if total > 0 { all_switches as f64 / (total as f64 / HOUR as f64) } else { 0.0 },
        "board": board_by.values().sum::<i64>(), "board_by": board_by,
        "asks": waits.len(), "waiting": waits.iter().filter(|w| w.1.is_none()).count(), "answer_ms": median(waits.iter().filter_map(|w| w.1).collect()),
        "off": (total > 0).then(|| off.iter().sum::<i64>() as f64 / total as f64),
        // a usual bucket: the median of those with any, to draw a line at (sessions: above)
        "usual_active_ms": median(active.iter().copied().filter(|a| *a > 0).collect()),
        "usual_switches": usual_rate, "rated": rates.iter().flatten().count(), "over": over.iter().filter(|o| **o).count(),
        // a usual working day: the medians of the first and the last minute of the days with any
        "starts": median(worked.iter().map(|s| s[0][0]).collect()), "ends": median(worked.iter().map(|s| s[s.len() - 1][1]).collect()),
        "phone": (paged > 0).then(|| phone as f64 / paged as f64),
    });
    let other: i64 = ranked.iter().skip(stack.len()).map(|p| p.1).sum();
    json!({"tiles": tiles, "series": series, "stacked": stack, "other_ms": other, "projects": projects, "heat": heat})
}
