// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The Dashboard's You tab (terminum.md AC-152): how much the human works here and how loaded, from what
//! they sent and typed (Dispatcher's never) and what they did on the board. Active time: those events in
//! time order; a pause over GAP ends a stretch, which lasts from its first to its last event and TAIL more
//! (reading the answer); the time after an event is its project's.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::Transaction;
use serde_json::{json, Value};

use super::{local, median, stack_key, stacked, Ask, Span, DAY, HOUR};
use crate::tasks::ops::rows;

pub const GAP: i64 = 15 * 60_000;
pub const TAIL: i64 = 2 * 60_000;
/// What the human does on the board, as the events' kinds say it.
const BOARD: [&str; 5] = ["made", "move", "comment", "edit", "archive"];

/// Something the human did: local ms, the project (`""` none), the pane (`""`: on the board), whether it
/// sent (a message, an Enter, a paste), whether a page on a phone sent it (None: no page did).
struct Event { t: i64, project: String, pane: String, sent: bool, phone: Option<bool> }

/// What is read once for both spans.
struct Read {
    events: Vec<Event>,
    /// the human's board events: local ms, kind
    board: Vec<(i64, String)>,
    /// a task's waits for a human: local ms it began, how long until it was moved on (None: it waits)
    waits: Vec<(i64, Option<i64>)>,
    names: HashMap<String, String>,
    kept_from: Option<i64>,
}

pub fn numbers(tx: &Transaction, ask: &Ask) -> Result<Value, String> {
    let read = read(tx, ask)?;
    let mut now = summarize(&read, ask.span, ask);
    let before = ask.earlier(read.kept_from, || summarize(&read, ask.before, ask)["tiles"].take());
    now["before"] = before;
    now["kept_from"] = json!(read.kept_from.filter(|k| *k > ask.span.lo()).map(|k| super::civil(k.div_euclid(DAY))));
    Ok(now)
}

fn read(tx: &Transaction, ask: &Ask) -> Result<Read, String> {
    let only = "AND (?2 IS NULL OR coalesce(project, '') = ?2)";
    let mut names = super::project_names(tx)?;
    let mut events = Vec::new();
    let msgs: Vec<(i64, String, String, String)> = rows(tx, &format!("SELECT {}, coalesce(project, ''), coalesce(project_name, ''), pane FROM messages
        WHERE sender = 'human' AND at >= ?1 {only}", local("at")), (ask.since, ask.project), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    for (t, project, name, pane) in msgs {
        if !name.is_empty() { names.entry(project.clone()).or_insert(name); }
        events.push(Event { t, project, pane, sent: true, phone: None });
    }
    // a row is its first and its last key; the first one says whether it sent and from where
    let sent = "component = 'page' AND (kind = 'paste' OR (kind = 'typed' AND text LIKE '%<Enter>'))";
    let phone = "coalesce(device, '') = 'web' AND (browser LIKE '%iPhone%' OR browser LIKE '%Android%')";
    let inputs: Vec<(i64, i64, String, String, bool, bool)> = rows(tx, &format!("SELECT {}, {}, coalesce(project, ''), pane, {sent}, {phone} FROM inputs
        WHERE sender = 'human' AND at >= ?1 {only}", local("at"), local("ended")), (ask.since, ask.project), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?;
    for (t, ended, project, pane, sent, phone) in inputs {
        if ended > t { events.push(Event { t: ended, project: project.clone(), pane: pane.clone(), sent: false, phone: None }); }
        events.push(Event { t, project, pane, sent, phone: sent.then_some(phone) });
    }
    let at = local("e.at * 1000");
    let mine = "(?2 IS NULL OR t.project = ?2)";
    let board: Vec<(i64, String, String)> = rows(tx, &format!("SELECT {at}, e.kind, CASE WHEN p.is_default THEN '' ELSE p.id END FROM events e JOIN tasks t ON t.id = e.task JOIN projects p ON p.id = t.project
        WHERE e.author = 'human' AND e.at >= ?1 / 1000 AND {mine}"), (ask.since, ask.task_project()), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    for (t, _, project) in &board {
        events.push(Event { t: *t, project: project.clone(), pane: String::new(), sent: false, phone: None });
    }
    // a wait: a move to WAIT FOR HUMAN, until the task's next move
    let waits = rows(tx, &format!("SELECT {at}, (SELECT min(o.at) FROM events o WHERE o.task = e.task AND o.kind = 'move' AND o.at >= e.at AND o.id != e.id AND json_extract(o.data, '$.from') = 'WAIT FOR HUMAN') - e.at
        FROM events e JOIN tasks t ON t.id = e.task WHERE e.kind = 'move' AND json_extract(e.data, '$.to') = 'WAIT FOR HUMAN' AND e.at >= ?1 / 1000 AND {mine}"),
        (ask.since, ask.task_project()), |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.map(|s| s * 1000))))?;
    events.sort_by_key(|e| e.t);
    // what the store still has of the human's typing: the input log is trimmed by age and by rows
    let kept_from: Option<i64> = tx.query_row(&format!("SELECT {} FROM (SELECT min(at) AS at FROM inputs) WHERE at IS NOT NULL", local("at")), [], |r| r.get(0)).ok();
    Ok(Read { events, board: board.into_iter().map(|(t, kind, _)| (t, kind)).collect(), waits, names, kept_from })
}

/// Whether an hour of weekday `wd` (0: Sunday) is outside a working day: a weekend, or 22:00 to 06:00.
fn off_hours(wd: usize, hour: usize) -> bool {
    wd == 0 || wd == 6 || !(6..22).contains(&hour)
}

/// Every number of `span`.
fn summarize(read: &Read, span: &Span, ask: &Ask) -> Value {
    let evs: Vec<&Event> = read.events.iter().filter(|e| span.holds(e.t)).collect();
    let n = span.len();
    let by_day = span.unit == "day";
    let mut active = vec![0i64; n];
    let mut by: Vec<HashMap<&str, i64>> = vec![HashMap::new(); n];
    let mut per_project: HashMap<&str, i64> = HashMap::new();
    let mut spans: Vec<Vec<[i64; 2]>> = vec![Vec::new(); n];
    let mut heat = [[0i64; 24]; 7];
    let (mut off, mut longest, mut began) = (0i64, 0i64, None);
    let mut active_units: HashSet<i64> = HashSet::new();
    for (i, e) in evs.iter().enumerate() {
        // the time after an event, up to the next one of its stretch, is its project's
        let next = evs.get(i + 1).map(|x| x.t).filter(|x| x - e.t <= GAP);
        let end = next.unwrap_or(e.t + TAIL);
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
                *by[k].entry(e.project.as_str()).or_default() += len;
                *per_project.entry(e.project.as_str()).or_default() += len;
                let (wd, hour) = ((d + 4).rem_euclid(7) as usize, (t.rem_euclid(DAY) / HOUR) as usize);   // 1970-01-01 was a Thursday; 0: Sunday
                heat[wd][hour] += len;
                if off_hours(wd, hour) { off += len; }
                active_units.insert(if span.unit == "hour" { t.div_euclid(HOUR) } else { d });
                if by_day {
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
    // sessions written to: a bucket's, a day's, the most in one hour; a switch: a send to another pane than the send before it, within a stretch
    let mut panes: Vec<HashSet<&str>> = vec![HashSet::new(); n];
    let mut day_panes: HashMap<i64, HashSet<&str>> = HashMap::new();
    let mut hourly: HashMap<i64, HashSet<&str>> = HashMap::new();
    let mut switches = vec![0i64; n];
    let mut sent = vec![0i64; n];
    let (mut last, mut phone, mut paged): (Option<&Event>, i64, i64) = (None, 0, 0);
    for e in evs.iter().copied().filter(|e| !e.pane.is_empty()) {
        let Some(k) = span.index(e.t) else { continue };
        panes[k].insert(&e.pane);
        day_panes.entry(e.t.div_euclid(DAY)).or_default().insert(&e.pane);
        hourly.entry(e.t.div_euclid(HOUR)).or_default().insert(&e.pane);
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
    let mut ranked: Vec<(String, i64)> = per_project.iter().map(|(id, ms)| (id.to_string(), *ms)).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let stack = stacked(&ranked);
    let projects: Vec<Value> = ranked.iter().map(|(id, ms)| {
        let mut p = ask.project_json(id, read.names.get(id).map_or("", String::as_str));
        p["n"] = json!(ms);
        p
    }).collect();
    let series: Vec<Value> = (0..n).map(|k| {
        let mut parts: BTreeMap<String, i64> = BTreeMap::new();
        for (id, ms) in &by[k] { *parts.entry(stack_key(&stack, id)).or_default() += ms; }
        json!({"active_ms": active[k], "by": parts, "sessions": panes[k].len(), "peak": peak[k], "switches": switches[k], "sent": sent[k],
               "board": board[k], "spans": by_day.then(|| &spans[k])})
    }).collect();
    let total: i64 = active.iter().sum();
    let worked: Vec<&Vec<[i64; 2]>> = spans.iter().filter(|s| !s.is_empty()).collect();
    let all_switches: i64 = switches.iter().sum();
    let tiles = json!({
        "active_ms": total, "active_units": active_units.len(), "units": if span.unit == "hour" { n as i64 } else { span.days() },
        "longest_ms": longest,
        "sessions": median(day_panes.values().map(|s| s.len() as i64).collect()), "peak": peak.iter().max().copied().unwrap_or(0),
        "switches": all_switches, "switches_per_hour": if total > 0 { all_switches as f64 / (total as f64 / HOUR as f64) } else { 0.0 },
        "board": board_by.values().sum::<i64>(), "board_by": board_by,
        "asks": waits.len(), "waiting": waits.iter().filter(|w| w.1.is_none()).count(), "answer_ms": median(waits.iter().filter_map(|w| w.1).collect()),
        "off": (total > 0).then(|| off as f64 / total as f64),
        // a usual working day: the medians of the first and the last minute of the days with any
        "starts": median(worked.iter().map(|s| s[0][0]).collect()), "ends": median(worked.iter().map(|s| s[s.len() - 1][1]).collect()),
        "phone": (paged > 0).then(|| phone as f64 / paged as f64),
    });
    json!({"tiles": tiles, "series": series, "stacked": stack, "projects": projects, "heat": heat})
}
