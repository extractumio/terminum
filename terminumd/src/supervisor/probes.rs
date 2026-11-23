// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-260: independently scheduled, project-scoped probe observations.

use std::{collections::{BTreeMap, BTreeSet}, path::{Path, PathBuf}, time::Duration};

use futures_util::{StreamExt, stream};
use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tokio::time::{MissedTickBehavior, interval, timeout};

use crate::{app::Shared, http::off, tasks::{integration::local_dir, ops::{now, project_place, rows, sql}}};

use super::{DAY, SCHEMA, catalog, flow::project_events, probe_arch::{self, Integrated},
    outside_feedback::{self, ReferenceBase}, probe_command::{self, Failure}, probe_gate, probe_git};

pub(super) const GIT: &str = "git";
pub(super) const ARCH: &str = "git_architecture";
pub(super) const GATE: &str = "gate_record";
pub(super) const MEASURED: &str = "measured";
const LIMIT: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);
const RETAINED_OUTPUT: usize = 2 * 1024 * 1024;
const RETAIN: i64 = 90 * DAY;
const FRESH: i64 = 2 * DAY;
pub(super) const OVERDUE: &str = "sample overdue";
const PROBE_BEAT: Duration = Duration::from_secs(15);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metric {
    metric: String,
    value: f64,
    unit: String,
    at: i64,
    #[serde(default)]
    dims: BTreeMap<String, String>,
}

pub(super) fn identifier(s: &str, limit: usize) -> bool {
    identifier_syntax(s, limit) && crate::sent::redact(s) == s
}

fn identifier_syntax(s: &str, limit: usize) -> bool {
    !s.is_empty() && s.len() <= limit && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

pub(super) fn probe_id(s: &str) -> bool {
    !matches!(s, "." | "..") && identifier_syntax(s, 80)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ProbeLine {
    Metric(Metric),
    Feedback(outside_feedback::Input),
}

#[derive(Debug, PartialEq)]
struct Observation {
    data: String,
    reports: Vec<outside_feedback::Report>,
}

fn custom_output(bytes: &[u8], at: i64, feedback: bool) -> Result<Observation, Failure> {
    let text = std::str::from_utf8(bytes).map_err(|_| Failure::Invalid)?;
    let mut rows = Vec::new();
    let mut reports = Vec::new();
    let mut identities = BTreeSet::new();
    for line in text.lines() {
        let row = match serde_json::from_str(line).map_err(|_| Failure::Invalid)? {
            ProbeLine::Metric(row) => row,
            ProbeLine::Feedback(input) if feedback && reports.len() < outside_feedback::MAX_REPORTS => {
                reports.push(outside_feedback::read(input)?);
                continue;
            }
            ProbeLine::Feedback(_) => return Err(Failure::Invalid),
        };
        if !identifier(&row.metric, 80) || !identifier(&row.unit, 24) || !row.value.is_finite()
            || !(1..=at).contains(&row.at) || row.dims.len() > 8
            || row.dims.iter().any(|(k, v)| !identifier(k, 32) || !identifier(v, 80))
            || !identities.insert((row.metric.clone(), row.at, row.dims.clone()))
        {
            return Err(Failure::Invalid);
        }
        rows.push(row);
    }
    if rows.is_empty() && !feedback { return Err(Failure::Invalid); }
    let data = serde_json::to_string(&rows).map_err(|_| Failure::Invalid)?;
    Ok(Observation { data, reports })
}

#[derive(Clone, PartialEq, Eq)]
enum Source {
    Git,
    Architecture(Option<u32>),
    Gate,
    Custom { id: String, command: String, feedback: Option<ReferenceBase> },
}

impl Source {
    fn name(&self) -> &str {
        match self {
            Self::Git => GIT,
            Self::Architecture(_) => ARCH,
            Self::Gate => GATE,
            Self::Custom { id, .. } => id,
        }
    }

    fn key(&self, dir: Option<&Path>) -> String {
        let mut hash = Sha256::new();
        hash.update(self.name());
        hash.update(b"\0");
        if let Some(dir) = dir { hash.update(dir.as_os_str().as_encoded_bytes()); }
        hash.update(b"\0");
        match self {
            // The feedback base affects publication, and Job::eligible checks it independently.
            Self::Custom { command, .. } => hash.update(command),
            Self::Architecture(Some(cap)) => hash.update(cap.to_le_bytes()),
            _ => (),
        }
        crate::auth::crypto::hex(&hash.finalize())
    }
}

struct Scope {
    dir: Option<PathBuf>,
    sources: Vec<Source>,
}

fn local_project_dir(tx: &Transaction, project: &str) -> Result<Option<PathBuf>, String> {
    let (_, host, dir) = project_place(tx, project)?;
    Ok((host.is_none() && !dir.is_empty()).then(|| local_dir(&dir)))
}

fn standard_sources(tx: &Transaction, project: &str) -> Result<Vec<Source>, String> {
    let (git, gate, cap): (bool, bool, Option<u32>) = tx.query_row("SELECT
        coalesce((SELECT json_extract(pack, '$.probes.git') FROM supervisor_settings WHERE project = ?1), 1),
        coalesce((SELECT json_extract(pack, '$.probes.gate_record') FROM supervisor_settings WHERE project = ?1), 1),
        (SELECT json_extract(pack, '$.probes.git_file_line_cap') FROM supervisor_settings WHERE project = ?1)",
        [project], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(sql)?;
    let mut sources = Vec::new();
    if git { sources.extend([Source::Git, Source::Architecture(cap)]); }
    if gate { sources.push(Source::Gate); }
    Ok(sources)
}

fn scope(tx: &Transaction, project: &str) -> Result<Scope, String> {
    let mut sources = standard_sources(tx, project)?;
    sources.extend(custom_sources(tx, project)?);
    Ok(Scope { dir: local_project_dir(tx, project)?, sources })
}

fn custom_sources(tx: &Transaction, project: &str) -> Result<Vec<Source>, String> {
    rows(tx, "SELECT p.probe, p.command, f.value FROM supervisor_probe_commands p
        LEFT JOIN supervisor_settings s ON s.project = p.project
        LEFT JOIN json_each(s.pack, '$.probes.feedback') f ON f.key = p.probe
        WHERE p.project = ?1 ORDER BY p.probe", [project],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)))?
        .into_iter().map(|(probe, command, base)| {
            let feedback = base.map(ReferenceBase::from_saved).transpose()?;
            Ok(Source::Custom { id: format!("custom:{probe}"), command, feedback })
        }).collect()
}

pub(super) fn custom_configs(tx: &Transaction, project: &str) -> Result<BTreeMap<String, String>, String> {
    let dir = local_project_dir(tx, project)?;
    Ok(custom_sources(tx, project)?.into_iter().filter_map(|source| {
        source.name().strip_prefix("custom:").map(|probe| (probe.to_owned(), source.key(dir.as_deref())))
    }).collect())
}

struct Job {
    project: String,
    generation: i64,
    dir: Option<PathBuf>,
    source: Source,
    config: String,
}

impl Job {
    fn eligible(&self, tx: &Transaction) -> Result<bool, String> {
        if super::rounds::enabled(tx, &self.project)? != Some(self.generation) { return Ok(false); }
        let eligible = scope(tx, &self.project)?;
        Ok(eligible.dir == self.dir && eligible.sources.contains(&self.source))
    }
}

fn due(tx: &Transaction, at: i64) -> Result<Vec<Job>, String> {
    let mut jobs = Vec::new();
    for (project, generation) in rows(tx, "SELECT project, generation FROM project_supervisor WHERE enabled = 1",
        [], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))?
    {
        let Scope { dir, sources } = scope(tx, &project)?;
        for source in sources {
            let config = source.key(dir.as_deref());
            let at_last = tx.query_row("SELECT max(at) FROM supervisor_probe_samples
                WHERE project = ?1 AND source = ?2 AND config = ?3 AND at <= ?4",
                (&project, source.name(), &config, at), |r| r.get::<_, Option<i64>>(0)).map_err(sql)?;
            if at_last.is_none_or(|old| at - old >= DAY) {
                jobs.push((at_last.unwrap_or(0), Job { project: project.clone(), generation, dir: dir.clone(), source, config }));
            }
        }
    }
    jobs.sort_by_key(|(last, _)| *last);
    Ok(jobs.into_iter().take(32).map(|(_, job)| job).collect())
}

fn integrated(tx: &Transaction, project: &str, at: i64) -> Result<Vec<Integrated>, String> {
    if !catalog::integration_on(tx, project)? { return Ok(Vec::new()); }
    rows(tx, &format!("{} SELECT t.id, coalesce(t.slug, t.id), t.created_at, e.at, json_extract(e.data, '$.commit'), t.type,
        b.candidate_sha, b.base_sha
        FROM project_events e JOIN tasks t ON t.id = e.task
        LEFT JOIN integration_rounds b ON (b.batch, b.round) = (
            SELECT r.batch, r.round FROM integration_rounds r JOIN integration_batches p ON p.id = r.batch
            JOIN json_each(r.tasks) j ON j.value = t.id
            WHERE p.project = ?1 AND r.result = 'published' AND r.ended = e.at
              AND (json_extract(e.data, '$.batch') IS NULL OR p.id = json_extract(e.data, '$.batch'))
            ORDER BY r.batch DESC LIMIT 1)
        WHERE e.seq = (SELECT min(l.seq) FROM project_events l WHERE l.task = t.id AND l.author = 'dispatcher'
            AND l.kind = 'comment' AND json_extract(l.data, '$.left') = 'landed')
        AND e.at >= ?3 AND t.created_at BETWEEN 1 AND e.at
        ORDER BY e.at DESC, t.id LIMIT 2049", project_events()),
        (project, at, at - RETAIN), |r| Ok(Integrated { task: r.get(0)?, slug: r.get(1)?, created: r.get(2)?,
            landed: r.get(3)?, commit: r.get(4)?, kind: r.get(5)?, candidate: r.get(6)?, base: r.get(7)? }))
}

async fn encode(data: impl Serialize + Send + 'static) -> Result<String, Failure> {
    tokio::task::spawn_blocking(move || {
        let text = serde_json::to_string(&data).map_err(|_| Failure::Invalid)?;
        if text.len() > RETAINED_OUTPUT { return Err(Failure::Output); }
        Ok(text)
    }).await.map_err(|_| Failure::Invalid)?
}

async fn collect(source: &Source, dir: Option<&Path>, at: i64, integrated: Vec<Integrated>) -> Result<Observation, Failure> {
    let dir = dir.ok_or(Failure::Unavailable)?;
    if !tokio::fs::metadata(dir).await.is_ok_and(|info| info.is_dir()) { return Err(Failure::Unavailable); }
    if integrated.len() > 2048 { return Err(Failure::Output); }
    let data = match source {
        Source::Git => encode(probe_git::collect(dir, at, integrated).await?).await,
        Source::Architecture(cap) => encode(probe_arch::collect(dir, at, integrated, *cap).await?).await,
        Source::Gate => encode(probe_gate::collect(dir, at).await?).await,
        Source::Custom { command, feedback, .. } => {
            let bytes = probe_command::shell(dir, command, TIMEOUT, LIMIT).await?;
            return tokio::task::spawn_blocking({
                let enabled = feedback.is_some();
                move || custom_output(&bytes, at, enabled)
            }).await.map_err(|_| Failure::Invalid)?;
        }
    }?;
    Ok(Observation { data, reports: Vec::new() })
}

async fn run_job(app: Shared, job: Job) -> Result<(), String> {
    let reader = app.clone();
    let at = now();
    let loaded = off(move || {
        let inputs = reader.tasks.snapshot(SCHEMA, |tx| {
            if !job.eligible(tx)? { return Ok(None); }
            let inputs = if matches!(job.source, Source::Git | Source::Architecture(_)) { integrated(tx, &job.project, at)? }
                else { Vec::new() };
            Ok(Some(inputs))
        })?;
        Ok((job, inputs))
    }).await?;
    let (job, Some(inputs)) = loaded else { return Ok(()); };
    let result = timeout(Duration::from_secs(28), collect(&job.source, job.dir.as_deref(), at, inputs))
        .await.unwrap_or(Err(Failure::Timeout));
    let writer = app.clone();
    if let Some(tasks) = off(move || writer.tasks.with(|tx| publish(tx, &job, at, result))).await? {
        if !tasks.is_empty() { crate::tasks::api::told(&app, ""); }
        app.supervisor.wake();
    }
    Ok(())
}

fn publish(tx: &Transaction, job: &Job, at: i64, result: Result<Observation, Failure>) -> Result<Option<Vec<String>>, String> {
    if !job.eligible(tx)? { return Ok(None); }
    let previous = latest_row(tx, &job.project, job.source.name(), &job.config, at, false)?;
    let (state, data, reports) = match result {
        Ok(observation) => (MEASURED, Some(observation.data), observation.reports),
        Err(error) => (error.code(), None, Vec::new()),
    };
    let id = tx.query_row("INSERT OR IGNORE INTO supervisor_probe_samples (project, source, config, at, state, data)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING id",
        (&job.project, job.source.name(), &job.config, at, state, data), |r| r.get::<_, i64>(0)).optional().map_err(sql)?;
    let Some(id) = id else { return Ok(None); };
    let mut tasks = Vec::new();
    if let Source::Custom { feedback: Some(base), .. } = &job.source {
        for report in reports {
            tasks.extend(outside_feedback::publish(tx, &job.project, base, report, at)?);
        }
    }
    let source = if previous.is_none_or(|row| row.state != state) { "supervisor_probe_status" } else { "supervisor_probe_samples" };
    tx.execute("INSERT INTO supervisor_changes (project, source, source_id) VALUES (?1, ?2, ?3)",
        (&job.project, source, id.to_string())).map_err(sql)?;
    tx.execute("DELETE FROM supervisor_probe_samples WHERE id IN (
        SELECT id FROM supervisor_probe_samples WHERE project = ?1 AND at < ?2 ORDER BY at LIMIT 64)",
        (&job.project, at - RETAIN)).map_err(sql)?;
    Ok(Some(tasks))
}

pub(crate) async fn run(app: Shared) {
    let mut clock = interval(PROBE_BEAT);
    clock.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        clock.tick().await;
        let reader = app.clone();
        match off(move || reader.tasks.snapshot(SCHEMA, |tx| due(tx, now()))).await {
            Ok(jobs) => stream::iter(jobs.into_iter().map(|job| run_job(app.clone(), job))).buffer_unordered(4)
                .for_each(|result| async {
                    if let Err(error) = result { tracing::warn!(event = "supervisor.probe_collection", %error); }
                }).await,
            Err(error) => tracing::warn!(event = "supervisor.probe_schedule", %error),
        }
    }
}

struct Sample {
    id: i64,
    at: i64,
    state: String,
    data: Option<String>,
}

fn latest_row(tx: &Transaction, project: &str, source: &str, config: &str, at: i64, include_data: bool) -> Result<Option<Sample>, String> {
    tx.query_row("SELECT id, at, state, CASE WHEN ?5 THEN data END FROM supervisor_probe_samples
        WHERE project = ?1 AND source = ?2 AND config = ?3 AND at <= ?4 ORDER BY at DESC LIMIT 1",
        (project, source, config, at, include_data), |r| Ok(Sample { id: r.get(0)?, at: r.get(1)?, state: r.get(2)?, data: r.get(3)? }))
        .optional().map_err(sql)
}

pub(super) fn read<T: DeserializeOwned>(tx: &Transaction, project: &str, name: &str, at: i64) -> Result<Option<(i64, T)>, String> {
    let sources = if cfg!(test) && name.starts_with("custom:") { scope(tx, project)?.sources }
        else { standard_sources(tx, project)? };
    let Some(source) = sources.iter().find(|s| s.name() == name) else { return Ok(None); };
    let dir = local_project_dir(tx, project)?;
    let sample = latest_row(tx, project, name, &source.key(dir.as_deref()), at, true)?;
    sample.filter(|s| at - s.at <= FRESH).and_then(|s| s.data.map(|data| (s.id, data))).map(|(id, data)| {
        serde_json::from_str(&data).map(|decoded| (id, decoded)).map_err(|_| "Invalid retained Supervisor probe data.".into())
    }).transpose()
}

pub(super) fn first_sample(tx: &Transaction, project: &str, source: &str, at: i64) -> Result<Option<i64>, String> {
    let Some(active) = standard_sources(tx, project)?.into_iter().find(|s| s.name() == source) else { return Ok(None); };
    let dir = local_project_dir(tx, project)?;
    tx.query_row("SELECT min(at) FROM supervisor_probe_samples WHERE project = ?1 AND source = ?2
        AND config = ?3 AND at <= ?4",
        (project, source, active.key(dir.as_deref()), at), |r| r.get(0)).map_err(sql)
}

#[derive(Serialize)]
pub(super) struct Status {
    pub source: String,
    pub at: Option<i64>,
    pub state: String,
    #[serde(skip)]
    pub id: Option<i64>,
}

pub(super) fn statuses(tx: &Transaction, project: &str, at: i64) -> Result<Vec<Status>, String> {
    let scope = scope(tx, project)?;
    scope.sources.iter().map(|source| {
        let sample = latest_row(tx, project, source.name(), &source.key(scope.dir.as_deref()), at, false)?;
        Ok(Status { source: source.name().into(), at: sample.as_ref().map(|s| s.at), id: sample.as_ref().map(|s| s.id),
            state: sample.map_or_else(|| "waiting for first sample".into(), |row| {
                if row.state == MEASURED && at - row.at > FRESH { OVERDUE.into() } else { row.state }
            }) })
    }).collect()
}

#[cfg(test)]
#[path = "probes_tests.rs"]
pub(super) mod tests;
