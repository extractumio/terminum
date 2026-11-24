// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Owner-wide notification settings, delivery and health (AC-227–229, AC-263).

use std::{net::Ipv6Addr, path::Path, time::Duration};
use axum::http::Uri;
use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::{app::Shared, tasks::{api::store, ops::{now, rows, sql}}};
use super::{Evaluation, Evidence, Reading, Rule, SCHEMA, WEEK, Window, impact::OWN_HEALTH,
    incidents, line_command, line_queue, signals};

const TEST: &str = "Terminum critical line test alert.\n";
const CHECK: &str = "Terminum critical line health check.\n";
const CHANNEL_LIMIT: usize = 6;
const PACE: Duration = Duration::from_secs(1);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Settings {
    pub channels: Vec<String>,
    link_base: Option<String>,
    pub quiet: Option<[u16; 2]>,
    pub ladder: [u16; 3],
}

impl Default for Settings {
    fn default() -> Self {
        Self { channels: Vec::new(), link_base: None, quiet: Some([23 * 60, 8 * 60]), ladder: [10, 30, 60] }
    }
}

impl Settings {
    pub fn validate(&mut self) -> Result<(), String> {
        if self.channels.len() > CHANNEL_LIMIT {
            return Err(format!("Use up to {CHANNEL_LIMIT} channel executables."));
        }
        for (i, path) in self.channels.iter().enumerate() {
            if path.len() > 4096 || !Path::new(path).is_absolute() || path.ends_with('/')
                || path.chars().any(crate::termtext::control)
            {
                return Err(format!("Channel {} needs an absolute executable path.", i + 1));
            }
            if self.channels[..i].contains(path) {
                return Err("Each channel executable must be different.".into());
            }
        }
        if let Some(base) = &mut self.link_base {
            *base = origin(base).ok_or("Use an HTTPS origin with no credentials, path, query or fragment.")?;
        }
        if self.quiet.is_some_and(|[from, to]| from >= 1440 || to >= 1440 || from == to) {
            return Err("Choose two different quiet-hour times.".into());
        }
        let [first, second, later] = self.ladder;
        if first == 0 || second <= first || second > 1440 || !(1..=1440).contains(&later) {
            return Err("Enter repeat minutes up to 1440, with the second repeat after the first.".into());
        }
        Ok(())
    }

    fn quiet_at(&self, minute: u16) -> bool {
        self.quiet.is_some_and(|[from, to]| {
            if from < to { (from..to).contains(&minute) } else { minute >= from || minute < to }
        })
    }

    /// The ladder's step in seconds: the first repeat, the second, then every later one.
    pub fn delay(&self, step: usize) -> i64 { i64::from(self.ladder[step]) * 60 }
}

fn origin(text: &str) -> Option<String> {
    if text.len() > 255 || text.contains(['@', '#', '?', '%', '\\']) || !text.starts_with("https://") {
        return None;
    }
    let uri: Uri = text.parse().ok()?;
    let authority = uri.authority()?;
    let host = authority.host();
    let valid = if let Some(ip) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        ip.parse::<Ipv6Addr>().is_ok()
    } else {
        super::dns_labels(host.trim_end_matches('.'))
    };
    if !valid || !matches!(uri.path(), "" | "/")
        || (authority.as_str() != host && authority.port_u16().is_none())
        || crate::sent::redact(host) != host
    {
        return None;
    }
    Some(format!("https://{authority}"))
}

#[derive(Default, Serialize)]
pub(super) struct Saved {
    pub revision: i64,
    pub settings: Settings,
    #[serde(skip)]
    checked_at: i64,
}

impl Saved {
    fn check_revision(&self, revision: i64) -> Result<(), String> {
        if revision == self.revision { Ok(()) }
        else { Err("Critical line settings changed in another tab. Reload them and try again.".into()) }
    }
}

pub(super) fn read(tx: &Transaction) -> Result<Saved, String> {
    tx.query_row("SELECT revision, settings, checked_at FROM supervisor_line", [], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
    }).optional().map_err(sql)?.map_or_else(|| Ok(Saved::default()), |(revision, settings, checked_at)| {
        let mut settings: Settings = serde_json::from_str(&settings).map_err(|_| "Invalid saved critical line settings.")?;
        settings.validate()?;
        Ok(Saved { revision, settings, checked_at })
    })
}

fn save(tx: &Transaction, revision: i64, mut settings: Settings, at: i64) -> Result<Saved, String> {
    settings.validate()?;
    let previous = read(tx)?;
    previous.check_revision(revision)?;
    let revision = revision + 1;
    let changed = previous.settings.channels != settings.channels;
    let checked_at = if changed { at } else { previous.checked_at };
    let encoded = serde_json::to_string(&settings).map_err(|_| "Could not encode critical line settings.")?;
    tx.execute("INSERT INTO supervisor_line VALUES (1, ?1, ?2, ?3)
        ON CONFLICT (id) DO UPDATE SET revision = excluded.revision, settings = excluded.settings,
            checked_at = excluded.checked_at", (revision, encoded, checked_at)).map_err(sql)?;
    if changed {
        for (i, old) in previous.settings.channels.iter().enumerate() {
            if settings.channels.get(i) != Some(old) {
                tx.execute("DELETE FROM supervisor_line_failures WHERE channel = ?1", [i as i64 + 1]).map_err(sql)?;
            }
        }
        clear_if_healthy(tx, at)?;
        if settings.channels.is_empty() {
            tx.execute("DELETE FROM supervisor_line_pending", []).map_err(sql)?;
            tx.execute("DELETE FROM supervisor_line_repeats", []).map_err(sql)?;
        } else if previous.settings.channels.is_empty() {
            line_queue::seed(tx)?;
        }
    }
    Ok(Saved { revision, settings, checked_at })
}

pub(super) async fn configure(app: &Shared, project: String, revision: i64, settings: Settings) -> Result<Saved, String> {
    let _serial = app.supervisor.line_serial.lock().await;
    let saved = store(app, move |tx| {
        crate::tasks::ops::check_project(tx, &project)?;
        save(tx, revision, settings, now())
    }).await?;
    app.supervisor.incident_changed();
    Ok(saved)
}

type Attempts = Vec<(usize, Result<(), String>)>;

pub(super) async fn test(app: &Shared, project: String, channel: usize, revision: i64) -> Result<(), String> {
    let _serial = app.supervisor.line_serial.lock().await;
    let (project, path) = store(app, move |tx| {
        crate::tasks::ops::check_project(tx, &project)?;
        let saved = read(tx)?;
        saved.check_revision(revision)?;
        let path = saved.settings.channels.get(channel).ok_or("Save this channel before testing it.")?;
        Ok((project, path.clone()))
    }).await?;
    let result = line_command::send(&path, false, TEST, line_command::DEADLINE).await;
    record(app, project, vec![(channel, result.clone())], false).await?;
    result
}

enum Job {
    Notice(line_queue::Notice),
    Weekly(String),
    Discarded,
}

fn claim(tx: &Transaction, at: i64, minute: Option<u16>) -> Result<Option<(Settings, Job)>, String> {
    let Saved { settings, checked_at, .. } = read(tx)?;
    if settings.channels.is_empty() {
        return Ok(None);
    }
    let quiet = settings.quiet.is_some() && minute.is_none_or(|minute| settings.quiet_at(minute));
    match line_queue::claim(tx, &settings, at, quiet)? {
        line_queue::Claim::Notice(notice) => return Ok(Some((settings, Job::Notice(notice)))),
        line_queue::Claim::Discarded => return Ok(Some((settings, Job::Discarded))),
        line_queue::Claim::Empty => {}
    }
    if at - checked_at >= WEEK {
        let project = tx.query_row("SELECT project FROM project_supervisor WHERE enabled ORDER BY project LIMIT 1",
            [], |r| r.get(0)).optional().map_err(sql)?;
        if let Some(project) = project {
            tx.execute("UPDATE supervisor_line SET checked_at = ?1", [at]).map_err(sql)?;
            return Ok(Some((settings, Job::Weekly(project))));
        }
    }
    Ok(None)
}

async fn deliver(settings: &Settings, text: &str, scheduled: usize, check: bool, limit: Duration) -> Attempts {
    let mut attempts = Vec::new();
    for (i, path) in settings.channels.iter().enumerate() {
        let result = line_command::send(path, check, text, limit).await;
        let done = result.is_ok() && i + 1 >= scheduled;
        attempts.push((i, result));
        if done {
            break;
        }
    }
    attempts
}

pub(crate) async fn run(app: Shared) {
    let mut clock = tokio::time::interval(Duration::from_secs(10));
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut changes = app.supervisor.incident_updates.subscribe();
    loop {
        tokio::select! {
            _ = clock.tick() => {},
            _ = changes.changed() => {},
        }
        // A read first: with no channel, the default, the tick takes no write turn.
        loop {
            let sent = match configured(&app).await {
                Ok(true) => send_next(&app).await,
                other => other,
            };
            match sent {
                // A backlog goes out paced, never back to back (AC-228).
                Ok(true) => tokio::time::sleep(PACE).await,
                Ok(false) => break,
                Err(error) => {
                    tracing::warn!(event = "supervisor.critical_line", %error);
                    break;
                }
            }
        }
        // Our own deliveries' changes are seen: they need no second pass.
        changes.borrow_and_update();
    }
}

async fn configured(app: &Shared) -> Result<bool, String> {
    let app = app.clone();
    crate::http::off(move || app.tasks.snapshot(SCHEMA, |tx| Ok(!read(tx)?.settings.channels.is_empty()))).await
}

async fn send_next(app: &Shared) -> Result<bool, String> {
    let _serial = app.supervisor.line_serial.lock().await;
    let (job, cards) = store(app, |tx| super::breakers::with_new_cards(tx, || {
        let at = now();
        claim(tx, at, crate::tasks::cron::minute_of_day(at))
    })).await?;
    super::breakers::notify(app, cards);
    let Some((settings, job)) = job else { return Ok(false) };
    let (project, text, scheduled, check) = match job {
        Job::Notice(notice) => {
            let text = notice.render(settings.link_base.as_deref());
            (notice.project, text, notice.channels, false)
        }
        Job::Weekly(project) => (project, CHECK.into(), settings.channels.len(), true),
        Job::Discarded => return Ok(true),
    };
    let attempts = deliver(&settings, &text, scheduled, check, line_command::DEADLINE).await;
    record(app, project, attempts, !check).await?;
    Ok(true)
}

async fn record(app: &Shared, project: String, attempts: Attempts, notice: bool) -> Result<(), String> {
    let changed = store(app, move |tx| {
        let at = now();
        if notice && attempts.iter().any(|(_, result)| result.is_ok()) {
            super::measures::page_dispatched(tx, &project, at)?;
        }
        record_attempts(tx, &project, attempts, at)
    }).await?;
    if changed {
        app.supervisor.incident_changed();
    }
    Ok(())
}

fn record_attempts(tx: &Transaction, project: &str, attempts: Attempts, at: i64) -> Result<bool, String> {
    let before = incidents::revision(tx)?;
    let failed = attempts.iter().any(|(_, result)| result.is_err());
    let mut changed = 0;
    for (i, result) in attempts {
        let channel = i as i64 + 1;
        changed += match result {
            Ok(()) => tx.execute("DELETE FROM supervisor_line_failures WHERE channel = ?1", [channel]),
            Err(failure) => tx.execute("INSERT INTO supervisor_line_failures VALUES (?1, ?2)
                ON CONFLICT (channel) DO UPDATE SET failure = excluded.failure WHERE failure <> excluded.failure", (channel, failure)),
        }.map_err(sql)?;
    }
    if !failed && changed == 0 {
        return Ok(false);
    }
    let failures = rows(tx, "SELECT channel, failure FROM supervisor_line_failures ORDER BY channel", [],
        |r| Ok(Evidence::SupervisorLine { channel: r.get(0)?, failure: r.get(1)? }))?;
    if failures.is_empty() {
        clear_if_healthy(tx, at)?;
    } else if failed && crate::tasks::ops::check_project(tx, project).is_ok() {
        report_failure(tx, project, at, failures)?;
    }
    Ok(changed != 0 || before != incidents::revision(tx)?)
}

fn clear_if_healthy(tx: &Transaction, at: i64) -> Result<(), String> {
    let projects = rows(tx, "SELECT DISTINCT project FROM supervisor_signals
        WHERE detector = 'self.line_dead' AND state <> 'resolved'
          AND NOT EXISTS (SELECT 1 FROM supervisor_line_failures)", [], |r| r.get::<_, String>(0))?;
    for project in projects {
        signals::resolve_rule(tx, &project, Rule::LineDead, at)?;
    }
    Ok(())
}

fn report_failure(tx: &Transaction, project: &str, at: i64, failures: Vec<Evidence>) -> Result<(), String> {
    let rule = Rule::LineDead;
    let previous = tx.query_row("SELECT evaluated_at FROM supervisor_evaluations WHERE project = ?1 AND detector = ?2",
        (project, rule.id()), |r| r.get::<_, i64>(0)).optional().map_err(sql)?;
    let at = previous.map_or(at, |last| at.max(last + 1));
    let reading = Reading::new("critical_line", project, "failed_deliveries", failures.len() as f64, failures, OWN_HEALTH);
    let Some(evaluation) = Evaluation::critical(project, rule, reading, Window { before: at, start: at, end: at }) else {
        return Ok(());
    };
    signals::prepare(&evaluation)?.write(tx, false)
}

#[cfg(test)]
#[path = "line_tests.rs"]
mod tests;
