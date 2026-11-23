// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-235: resolve typed evidence without exposing transcripts or raw input text.

use rusqlite::{OptionalExtension, Params, Transaction};
use serde_json::Value;

use crate::tasks::ops::sql;

use super::Evidence;

pub(super) fn resolve(tx: &Transaction, project: &str, id: &Evidence) -> Result<Option<Value>, String> {
    fn select(tx: &Transaction, query: &str, params: impl Params) -> Result<Option<Value>, String> {
        tx.query_row(query, params, |r| r.get::<_, String>(0)).optional().map_err(sql)?
            .map(|raw| serde_json::from_str(&raw).map_err(|e| e.to_string())).transpose()
    }
    match id {
        Evidence::Tasks(id) => select(tx, "SELECT json_object('task', slug, 'status', status, 'severity', severity)
            FROM tasks WHERE project = ?1 AND id = ?2", (project, id)),
        Evidence::Events(id) => select(tx, "SELECT json_object('task', t.slug, 'kind', e.kind, 'at', e.at)
            FROM events e JOIN tasks t ON t.id = e.task WHERE t.project = ?1 AND e.id = ?2", (project, id)),
        Evidence::Runs(id) => select(tx, "SELECT json_object('state', state, 'program', kind)
            FROM runs WHERE project = ?1 AND id = ?2", (project, id)),
        Evidence::ScheduleFires(id) => select(tx, "SELECT json_object('at', f.at, 'state', f.state, 'missed', f.missed)
            FROM schedule_fires f JOIN schedules s ON s.id = f.schedule WHERE s.project = ?1 AND f.id = ?2", (project, id)),
        Evidence::TaskLinks { task, other } => select(tx, "SELECT json_object('kind', l.kind, 'task', a.slug, 'other', b.slug)
            FROM task_links l JOIN tasks a ON a.id = l.task JOIN tasks b ON b.id = l.other
            WHERE a.project = ?1 AND b.project = ?1 AND l.task = ?2 AND l.other = ?3",
            (project, task, other)),
        Evidence::Inputs(id) => select(tx, "SELECT json_object('at', at, 'kind', kind, 'bytes', bytes)
            FROM inputs WHERE project = ?1 AND id = ?2", (project, id)),
        Evidence::Usage(id) => select(tx, "SELECT json_object('at', u.at, 'input', u.input, 'output', u.output,
            'cache_read', u.cache_read, 'cache_write', u.cache_write)
            FROM usage u JOIN runs r ON r.id = u.run WHERE r.project = ?1 AND u.rowid = ?2", (project, id)),
        Evidence::UserInteraction(id) => select(tx, "SELECT json_object('at', at)
            FROM user_interaction WHERE project = ?1 AND id = ?2", (project, id)),
        Evidence::Messages(id) => select(tx, "SELECT json_object('at', at, 'via', via)
            FROM messages WHERE project = ?1 AND id = ?2", (project, id)),
        Evidence::SupervisorPoints(id) => select(tx, "SELECT json_object('metric', metric, 'at', at, 'value', value)
            FROM supervisor_points WHERE project = ?1 AND id = ?2", (project, id)),
        Evidence::SupervisorHookCounts(id) => select(tx, "SELECT json_object('event', event, 'at', at, 'count', count)
            FROM supervisor_hook_counts WHERE project = ?1 AND id = ?2", (project, id)),
        Evidence::SupervisorHeartbeat { project: id, last_ok } => select(tx, "SELECT json_object('last_ok', last_ok, 'enabled_at', enabled_at)
            FROM project_supervisor WHERE project = ?1 AND project = ?2 AND last_ok IS ?3", (project, id, last_ok)),
        Evidence::SupervisorLine { channel, failure } => select(tx, "SELECT json_object('channel', f.channel)
            FROM supervisor_line_failures f WHERE f.channel = ?2 AND f.failure = ?3 AND EXISTS (
                SELECT 1 FROM supervisor_signals s, json_each(s.evidence) e
                WHERE s.project = ?1 AND s.detector = 'self.line_dead'
                AND json_extract(e.value, '$.id.channel') = ?2 AND json_extract(e.value, '$.id.failure') = ?3)",
            (project, channel, failure)),
        Evidence::SupervisorActions(id) => select(tx, "SELECT json_object('kind', kind, 'detector', detector, 'response', response, 'at', at)
            FROM supervisor_actions WHERE project = ?1 AND id = ?2", (project, id)),
        Evidence::SupervisorRounds(id) => select(tx, "SELECT json_object('refused', refused, 'ended_at', ended_at)
            FROM supervisor_rounds WHERE project = ?1 AND id = ?2", (project, id)),
    }
}
