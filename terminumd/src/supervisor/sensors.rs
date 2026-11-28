// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Counts accepted hook names in memory (AC-218). A failed write returns the same deltas.

use std::collections::{BTreeMap, HashMap, HashSet};

use parking_lot::Mutex;
use rusqlite::Transaction;
use serde_json::Value;

use crate::dispatcher::runs::STILL;
use crate::tasks::ops::sql;
use super::{Rule, Window};

pub(super) const MAX_SESSIONS: usize = 1024;
const FLUSH_SESSIONS: usize = 256;
const RETAIN_SECONDS: i64 = 30 * super::DAY;
const KEEP_REPEAT_STATE: i64 = super::DAY;
const MAX_THREADS: usize = 64;

#[derive(Default)]
pub(super) struct Sensors(Mutex<HashMap<String, Project>>);

#[derive(Default)]
struct Project {
    generation: i64,
    sessions: HashMap<HookSession, Session>,
    overflow: Gaps,
}

pub(super) const INCOMPLETE: &str = "Hook tracking overflow made this detector window incomplete.";

/// Fixed keys only: the cap that was hit and the detector whose input was lost.
#[derive(Default)]
pub(super) struct Gaps(BTreeMap<(&'static str, &'static str), Gap>);

struct Gap {
    first: i64,
    last: i64,
    count: i64,
}

impl Gaps {
    fn add(&mut self, key: (&'static str, &'static str), gap: Gap) {
        self.0.entry(key).and_modify(|old| {
            old.first = old.first.min(gap.first);
            old.last = old.last.max(gap.last);
            old.count = old.count.saturating_add(gap.count);
        }).or_insert(gap);
    }

    pub(super) fn affects(&self, rule: Rule, window: Window) -> bool {
        self.0.iter().any(|((_, detector), gap)| *detector == rule.id() && gap.first <= window.end && gap.last >= window.start)
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct HookSession {
    agent: &'static str,
    id: String,
    pane: String,
}

#[derive(Default)]
struct Session {
    /// each thread's previous call: the main agent's under "", a subagent's under its ID
    last_calls: HashMap<String, String>,
    counts: Delta,
    seen_at: i64,
}

#[derive(Default)]
struct Delta(BTreeMap<&'static str, i64>);

impl Delta {
    fn add(&mut self, event: &'static str, count: i64) {
        let total = self.0.entry(event).or_default();
        *total = total.saturating_add(count);
    }
}

impl Sensors {
    pub(super) fn any_on(&self) -> bool {
        !self.0.lock().is_empty()
    }

    pub(super) fn set(&self, project: &str, generation: i64, on: bool) {
        let mut scopes = self.0.lock();
        if on {
            scopes.insert(project.to_owned(), Project { generation, ..Default::default() });
        } else {
            scopes.remove(project);
        }
    }

    pub(super) fn refresh<'a>(&self, enabled: impl Iterator<Item = (&'a str, i64)> + Clone, at: i64) {
        let live: HashSet<_> = enabled.clone().collect();
        let mut known = self.0.lock();
        known.retain(|id, scope| live.contains(&(id.as_str(), scope.generation)));
        for (id, generation) in enabled {
            let scope = known.entry(id.to_owned()).or_insert_with(|| Project { generation, ..Default::default() });
            scope.sessions.retain(|_, s| !s.counts.0.is_empty() || at - s.seen_at < KEEP_REPEAT_STATE);
        }
    }

    pub(super) fn heard(&self, project: &str, pane: &str, event: &Value, at: i64) {
        let kind = match event["event"].as_str() {
            Some("PreToolUse") => "tool",
            Some("PostToolUseFailure") => "failure",
            Some("SubagentStart") => "subagent_start",
            Some("SubagentStop") => "subagent_stop",
            Some("Stop" | "StopFailure") => "stop",
            Some("PreCompact") => "compaction",
            _ => return,
        };
        let Some(agent) = event["agent"].as_str().and_then(crate::tmux::hookstate::agent) else { return };
        let Some(id) = event["session"].as_str().filter(|s| !s.is_empty() && s.len() <= 400 && !crate::termtext::has_control(s)) else { return };
        let mut scopes = self.0.lock();
        let Some(scope) = scopes.get_mut(project) else { return };
        let key = HookSession { agent, id: id.to_owned(), pane: pane.to_owned() };
        if scope.sessions.len() >= MAX_SESSIONS && !scope.sessions.contains_key(&key) {
            let detector = match kind {
                "subagent_start" => Rule::Subagents.id(),
                "tool" | "compaction" | "failure" => Rule::Loop.id(),
                _ => "",
            };
            scope.overflow.add(("sessions", detector), Gap { first: at, last: at, count: 1 });
            return;
        }
        let s = scope.sessions.entry(key).or_default();
        s.seen_at = at;
        s.counts.add(kind, 1);
        // AC-212: a repeat is the same call again, its tool and input both equal to its thread's previous call's
        let thread = event["sub"].as_str().unwrap_or_default();
        match kind {
            "tool" => {
                let call = event["tool_call_hash"].as_str().filter(|hash| hash.len() == 16 && hash.bytes().all(|c| c.is_ascii_hexdigit()));
                if call.is_some() && s.last_calls.get(thread).map(String::as_str) == call {
                    s.counts.add("same_call", 1);
                } else if let Some(call) = call.filter(|_| s.last_calls.len() < MAX_THREADS || s.last_calls.contains_key(thread)) {
                    s.last_calls.insert(thread.to_owned(), call.to_owned());
                } else if call.is_some() {
                    scope.overflow.add(("threads", Rule::Loop.id()), Gap { first: at, last: at, count: 1 });
                } else {
                    s.last_calls.remove(thread);
                }
            }
            "subagent_stop" if !thread.is_empty() => drop(s.last_calls.remove(thread)),
            "stop" => s.last_calls.clear(),
            _ => {}
        }
    }

    pub(super) fn take(&self, project: &str, generation: i64) -> Pending<'_> {
        let mut scopes = self.0.lock();
        let (deltas, overflow) = scopes.get_mut(project).filter(|scope| scope.generation == generation).map(|scope| {
            let mut active = scope.sessions.iter_mut().filter(|(_, s)| !s.counts.0.is_empty()).collect::<Vec<_>>();
            active.sort_unstable_by_key(|(_, s)| s.seen_at);
            let deltas = active.into_iter().take(FLUSH_SESSIONS)
                .map(|(session, s)| (session.clone(), std::mem::take(&mut s.counts))).collect();
            (deltas, std::mem::take(&mut scope.overflow))
        }).unwrap_or_default();
        Pending { sensors: self, project: project.into(), generation, deltas, overflow }
    }
}

pub(super) struct Pending<'a> {
    sensors: &'a Sensors,
    project: String,
    generation: i64,
    deltas: Vec<(HookSession, Delta)>,
    pub(super) overflow: Gaps,
}

impl Pending<'_> {
    pub(super) fn encode(&self) -> Result<Option<String>, String> {
        if self.deltas.is_empty() {
            return Ok(None);
        }
        let counts = self.deltas.iter().flat_map(|(session, delta)| {
            delta.0.iter().map(move |(event, count)| (session.agent, &session.id, event, count, &session.pane))
        }).collect::<Vec<_>>();
        serde_json::to_string(&counts).map(Some).map_err(|err| err.to_string())
    }

    pub(super) fn write(&self, tx: &Transaction, at: i64, counts: Option<String>) -> Result<(), String> {
        let project = &self.project;
        if let Some(counts) = counts {
            // Reconfirm ownership before storing counters. The pane identifier is
            // only an in-memory routing key and does not become counter evidence.
            tx.execute(&format!("INSERT INTO supervisor_hook_counts (project, agent, session, at, event, count)
                SELECT ?1, e.value ->> 0, e.value ->> 1, ?2, e.value ->> 2, e.value ->> 3 FROM json_each(?3) e
                WHERE coalesce(
                    (SELECT r.project FROM runs r WHERE r.pane_key = (e.value ->> 4) AND {STILL} ORDER BY r.rowid DESC LIMIT 1),
                    (SELECT m.project FROM project_members m WHERE m.pane_key = (e.value ->> 4))) = ?1
                ON CONFLICT (project, agent, session, at, event) DO UPDATE SET count = count + excluded.count"),
                (project, at, counts)).map_err(sql)?;
        }
        for ((reason, detector), gap) in &self.overflow.0 {
            tx.execute("INSERT INTO supervisor_hook_overflow (project,reason,detector,first_at,last_at,count)
                VALUES (?1,?2,?3,?4,?5,?6)
                ON CONFLICT DO UPDATE SET count = count + excluded.count",
                (project, reason, detector, gap.first, gap.last, gap.count)).map_err(sql)?;
        }
        tx.execute("DELETE FROM supervisor_hook_overflow WHERE (project,last_at,reason,detector,first_at) IN (
            SELECT project,last_at,reason,detector,first_at FROM supervisor_hook_overflow
            WHERE project = ?1 AND last_at < ?2 ORDER BY last_at LIMIT 2048)", (project, at - RETAIN_SECONDS)).map_err(sql)?;
        tx.execute("DELETE FROM supervisor_hook_counts WHERE id IN (
            SELECT id FROM supervisor_hook_counts WHERE project = ?1 AND at < ?2 ORDER BY at LIMIT 2048)",
            (project, at - RETAIN_SECONDS)).map(drop).map_err(sql)
    }

    pub(super) fn committed(mut self) {
        self.deltas.clear();
        self.overflow.0.clear();
    }
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        if self.deltas.is_empty() && self.overflow.0.is_empty() {
            return;
        }
        let mut scopes = self.sensors.0.lock();
        let Some(scope) = scopes.get_mut(&self.project).filter(|scope| scope.generation == self.generation) else { return };
        for (key, gap) in std::mem::take(&mut self.overflow.0) { scope.overflow.add(key, gap); }
        for (key, delta) in self.deltas.drain(..) {
            let s = scope.sessions.entry(key).or_default();
            for (event, count) in delta.0 {
                s.counts.add(event, count);
            }
        }
    }
}

#[cfg(test)]
#[path = "sensors_overflow_tests.rs"]
mod overflow_tests;
