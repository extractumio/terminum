// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Counts accepted hook names in memory (AC-218). A failed write returns the same deltas.

use std::collections::{BTreeMap, HashMap, HashSet};

use parking_lot::Mutex;
use rusqlite::Transaction;
use serde_json::Value;

use crate::dispatcher::runs::STILL;
use crate::tasks::ops::sql;

const MAX_SESSIONS: usize = 1024;
const FLUSH_SESSIONS: usize = 256;
const RETAIN_SECONDS: i64 = 30 * super::DAY;
const KEEP_REPEAT_STATE: i64 = super::DAY;

#[derive(Default)]
pub(super) struct Sensors(Mutex<HashMap<String, Project>>);

struct Project {
    generation: i64,
    sessions: HashMap<HookSession, Session>,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct HookSession {
    agent: &'static str,
    id: String,
    pane: String,
}

#[derive(Default)]
struct Session {
    last_call: Option<String>,
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
            scopes.insert(project.to_owned(), Project { generation, sessions: HashMap::new() });
        } else {
            scopes.remove(project);
        }
    }

    pub(super) fn refresh<'a>(&self, enabled: impl Iterator<Item = (&'a str, i64)> + Clone, at: i64) {
        let live: HashSet<_> = enabled.clone().collect();
        let mut known = self.0.lock();
        known.retain(|id, scope| live.contains(&(id.as_str(), scope.generation)));
        for (id, generation) in enabled {
            let scope = known.entry(id.to_owned()).or_insert_with(|| Project { generation, sessions: HashMap::new() });
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
            return;
        }
        let s = scope.sessions.entry(key).or_default();
        s.seen_at = at;
        s.counts.add(kind, 1);
        // AC-212: a repeat is the same call again, its tool and input both equal to the previous call's
        if kind == "tool" {
            let call = event["tool_call_hash"].as_str().filter(|hash| hash.len() == 16 && hash.bytes().all(|c| c.is_ascii_hexdigit()));
            if call.is_some() && call == s.last_call.as_deref() {
                s.counts.add("same_call", 1);
            }
            s.last_call = call.map(str::to_owned);
        }
        if kind == "stop" {
            s.last_call = None;
        }
    }

    pub(super) fn take(&self, project: &str, generation: i64) -> Pending<'_> {
        let mut scopes = self.0.lock();
        let deltas = scopes.get_mut(project).filter(|scope| scope.generation == generation).map(|scope| {
            let mut active = scope.sessions.iter_mut().filter(|(_, s)| !s.counts.0.is_empty()).collect::<Vec<_>>();
            active.sort_unstable_by_key(|(_, s)| s.seen_at);
            active.into_iter().take(FLUSH_SESSIONS)
                .map(|(session, s)| (session.clone(), std::mem::take(&mut s.counts))).collect()
        }).unwrap_or_default();
        Pending { sensors: self, project: project.into(), generation, deltas }
    }
}

pub(super) struct Pending<'a> {
    sensors: &'a Sensors,
    project: String,
    generation: i64,
    deltas: Vec<(HookSession, Delta)>,
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

    pub(super) fn write(tx: &Transaction, project: &str, at: i64, counts: Option<String>) -> Result<(), String> {
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
        tx.execute("DELETE FROM supervisor_hook_counts WHERE id IN (
            SELECT id FROM supervisor_hook_counts WHERE project = ?1 AND at < ?2 ORDER BY at LIMIT 2048)",
            (project, at - RETAIN_SECONDS)).map(drop).map_err(sql)
    }

    pub(super) fn committed(mut self) {
        self.deltas.clear();
    }
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        if self.deltas.is_empty() {
            return;
        }
        let mut scopes = self.sensors.0.lock();
        let Some(scope) = scopes.get_mut(&self.project).filter(|scope| scope.generation == self.generation) else { return };
        for (key, delta) in self.deltas.drain(..) {
            let s = scope.sessions.entry(key).or_default();
            for (event, count) in delta.0 {
                s.counts.add(event, count);
            }
        }
    }
}
