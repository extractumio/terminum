// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-220: signed-in project pack transfer and separate, one-command approval.

use std::collections::BTreeMap;

use axum::{Json, Router, extract::{Path, State}, http::header, response::{IntoResponse, Response}, routing::{get, post}};
use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::Shared;
use crate::auth::crypto::random_hex;
use crate::http::off;
use crate::tasks::{api::{answer, store}, ops::{check_project, rows, sql}};

use super::{Mode, Rule, SCHEMA, Threshold, settings::{self, Settings}};

const VERSION: u32 = 1;
const MAX_FILE: usize = 768 * 1024;
const MAX_COMMANDS: usize = 32;

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/task-projects/{id}/supervisor/settings", get(shown).put(save))
        .route("/api/task-projects/{id}/supervisor/export", post(export))
        .route("/api/task-projects/{id}/supervisor/import", post(import))
        .route("/api/task-projects/{id}/supervisor/probe-candidates/{probe}", post(stage).delete(discard))
        .route("/api/task-projects/{id}/supervisor/probe-candidates/{probe}/preview", post(preview))
        .route("/api/task-projects/{id}/supervisor/probe-candidates/{probe}/confirm", post(confirm))
        .route("/api/task-projects/{id}/supervisor/probe-commands/{probe}", axum::routing::delete(clear))
}

#[derive(Serialize)]
struct View {
    #[serde(flatten)]
    saved: settings::Saved,
    detector_defaults: BTreeMap<&'static str, DetectorDefaults>,
    confirmed_probes: Vec<String>,
    unset_probes: Vec<String>,
}

#[derive(Serialize)]
struct DetectorDefaults {
    mode: Mode,
    window_days: u16,
    threshold: Threshold,
}

fn view(tx: &Transaction, id: &str, saved: settings::Saved) -> Result<View, String> {
    let confirmed_probes = rows(tx, "SELECT probe FROM supervisor_probe_commands WHERE project = ?1 ORDER BY probe",
        [id], |row| row.get(0))?;
    let unset_probes = rows(tx, "SELECT probe FROM supervisor_probe_candidates WHERE project = ?1 ORDER BY probe",
        [id], |row| row.get(0))?;
    let defaults = Settings::default();
    let detector_defaults = Rule::ALL.iter().copied().filter(|rule| rule.configurable()).map(|rule| {
        let d = defaults.detector(rule);
        (rule.id(), DetectorDefaults { mode: d.mode, window_days: d.window_days, threshold: d.threshold })
    }).collect();
    Ok(View { saved, detector_defaults, confirmed_probes, unset_probes })
}

async fn shown(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    answer(off(move || app.tasks.snapshot(SCHEMA, |tx| {
        check_project(tx, &id)?;
        view(tx, &id, settings::read(tx, &id)?)
    })).await)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    revision: i64,
    settings: Settings,
}

async fn save(State(app): State<Shared>, Path(id): Path<String>, Json(body): Json<Save>) -> Response {
    let result = store(&app, move |tx| {
        check_project(tx, &id)?;
        let saved = settings::write(tx, &id, body.revision, body.settings, false)?;
        view(tx, &id, saved)
    }).await;
    if result.is_ok() {
        app.supervisor.wake();
    }
    answer(result)
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Export {
    include_commands: bool,
    include_secrets: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pack<T> {
    version: u32,
    settings: T,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    probe_commands: BTreeMap<String, String>,
}

fn redact(value: &mut Value) {
    match value {
        Value::String(text) => *text = crate::sent::redact(text),
        Value::Array(items) => items.iter_mut().for_each(redact),
        Value::Object(object) => {
            object.retain(|key, _| crate::sent::redact(key) == *key);
            object.values_mut().for_each(redact);
        }
        _ => {}
    }
}

fn pack(tx: &Transaction, id: &str, options: Export) -> Result<String, String> {
    check_project(tx, id)?;
    let mut settings = serde_json::to_value(settings::read(tx, id)?.settings).map_err(|_| "Could not encode Supervisor settings.")?;
    if !options.include_secrets {
        redact(&mut settings);
    }
    let mut probe_commands = BTreeMap::new();
    if options.include_commands {
        for (probe, command) in rows(tx, "SELECT probe, command FROM supervisor_probe_commands WHERE project = ?1",
            [id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        {
            if options.include_secrets || (crate::sent::redact(&command) == command && crate::sent::redact(&probe) == probe) {
                probe_commands.insert(probe, command);
            }
        }
    }
    let text = serde_json::to_string_pretty(&Pack { version: VERSION, settings, probe_commands })
        .map_err(|_| "Could not encode the Supervisor settings file.")?;
    if text.len() >= MAX_FILE {
        return Err("The Supervisor settings file would be larger than 768 KiB.".into());
    }
    Ok(text + "\n")
}

async fn export(State(app): State<Shared>, Path(id): Path<String>, Json(options): Json<Export>) -> Response {
    let result = off(move || app.tasks.snapshot(SCHEMA, |tx| pack(tx, &id, options))).await;
    match result {
        Ok(text) => ([(header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"supervisor-settings.json\"")], text).into_response(),
        Err(e) => answer::<()>(Err(e)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Import {
    revision: i64,
    file: String,
}

fn check_command(probe: &str, command: &str) -> Result<(), String> {
    if probe.is_empty() || probe.len() > 80 || !probe.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) {
        return Err("Use letters, numbers, dots, underscores or hyphens in a probe ID.".into());
    }
    if command.trim().is_empty() || command.len() > 8192 ||
        command.chars().any(|c| !matches!(c, '\n' | '\t') && crate::termtext::control(c))
    {
        return Err("A probe command needs 1–8192 bytes and cannot contain hidden control characters.".into());
    }
    Ok(())
}

fn stage_command(tx: &Transaction, id: &str, probe: &str, command: &str) -> Result<(), String> {
    let can_stage = tx.query_row("SELECT count(*) < ?3 OR EXISTS (
        SELECT 1 FROM supervisor_probe_candidates WHERE project = ?1 AND probe = ?2
        ) FROM supervisor_probe_candidates WHERE project = ?1", (id, probe, MAX_COMMANDS as i64), |r| r.get::<_, bool>(0)).map_err(sql)?;
    if !can_stage {
        return Err("A project can have up to 32 pending Supervisor probe commands.".into());
    }
    tx.execute("INSERT INTO supervisor_probe_candidates (project, probe, command) VALUES (?1, ?2, ?3)
        ON CONFLICT (project, probe) DO UPDATE SET command = excluded.command, approval = NULL",
        (id, probe, command)).map(drop).map_err(sql)
}

fn read_pack(file: &str) -> Result<Pack<Settings>, String> {
    if file.len() > MAX_FILE {
        return Err("The Supervisor settings file is larger than 768 KiB.".into());
    }
    let pack: Pack<Settings> = serde_json::from_str(file)
        .map_err(|_| "The file is not a valid Supervisor settings pack.")?;
    if pack.version != VERSION {
        return Err("This Supervisor settings file version is unsupported.".into());
    }
    if pack.probe_commands.len() > MAX_COMMANDS {
        return Err("A Supervisor settings file can contain up to 32 probe commands.".into());
    }
    for (probe, command) in &pack.probe_commands {
        check_command(probe, command)?;
    }
    Ok(pack)
}

async fn import(State(app): State<Shared>, Path(id): Path<String>, Json(body): Json<Import>) -> Response {
    let pack = match read_pack(&body.file) {
        Ok(pack) => pack,
        Err(error) => return answer::<View>(Err(error)),
    };
    let result = store(&app, move |tx| {
        check_project(tx, &id)?;
        let saved = settings::write(tx, &id, body.revision, pack.settings, true)?;
        tx.execute("DELETE FROM supervisor_probe_candidates WHERE project = ?1", [&id]).map_err(sql)?;
        let mut insert = tx.prepare_cached("INSERT INTO supervisor_probe_candidates (project, probe, command)
            VALUES (?1, ?2, ?3)").map_err(sql)?;
        for (probe, command) in pack.probe_commands {
            insert.execute((&id, probe, command)).map_err(sql)?;
        }
        view(tx, &id, saved)
    }).await;
    if result.is_ok() {
        app.supervisor.wake();
    }
    answer(result)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    command: String,
}

async fn stage(State(app): State<Shared>, Path((id, probe)): Path<(String, String)>, Json(body): Json<Candidate>) -> Response {
    answer(store(&app, move |tx| {
        check_project(tx, &id)?;
        check_command(&probe, &body.command)?;
        stage_command(tx, &id, &probe, &body.command)?;
        view(tx, &id, settings::read(tx, &id)?)
    }).await)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preview {}

async fn preview(State(app): State<Shared>, Path((id, probe)): Path<(String, String)>, Json(_): Json<Preview>) -> Response {
    answer(store(&app, move |tx| {
        check_project(tx, &id)?;
        let approval = random_hex::<16>();
        let command: String = tx.query_row("UPDATE supervisor_probe_candidates SET approval = ?3
            WHERE project = ?1 AND probe = ?2 RETURNING command", (&id, &probe, &approval), |row| row.get(0))
            .optional().map_err(sql)?.ok_or("That probe command is no longer pending.")?;
        Ok(serde_json::json!({ "probe": probe, "command": command, "approval": approval }))
    }).await)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approval {
    approval: String,
}

async fn confirm(State(app): State<Shared>, Path((id, probe)): Path<(String, String)>, Json(body): Json<Approval>) -> Response {
    answer(store(&app, move |tx| {
        check_project(tx, &id)?;
        let command: String = tx.query_row("DELETE FROM supervisor_probe_candidates
            WHERE project = ?1 AND probe = ?2 AND approval = ?3 RETURNING command",
            (&id, &probe, &body.approval), |row| row.get(0))
            .optional().map_err(sql)?.ok_or("View the current probe command in full before confirming it.")?;
        let can_confirm = tx.query_row("SELECT count(*) < ?3 OR EXISTS (
            SELECT 1 FROM supervisor_probe_commands WHERE project = ?1 AND probe = ?2
            ) FROM supervisor_probe_commands WHERE project = ?1", (&id, &probe, MAX_COMMANDS as i64), |r| r.get::<_, bool>(0)).map_err(sql)?;
        if !can_confirm {
            return Err("A project can have up to 32 confirmed Supervisor probe commands.".into());
        }
        tx.execute("INSERT INTO supervisor_probe_commands (project, probe, command) VALUES (?1, ?2, ?3)
            ON CONFLICT (project, probe) DO UPDATE SET command = excluded.command", (&id, &probe, command)).map_err(sql)?;
        view(tx, &id, settings::read(tx, &id)?)
    }).await)
}

async fn discard(State(app): State<Shared>, Path((id, probe)): Path<(String, String)>) -> Response {
    answer(store(&app, move |tx| {
        check_project(tx, &id)?;
        tx.execute("DELETE FROM supervisor_probe_candidates WHERE project = ?1 AND probe = ?2", (&id, &probe)).map_err(sql)?;
        view(tx, &id, settings::read(tx, &id)?)
    }).await)
}

async fn clear(State(app): State<Shared>, Path((id, probe)): Path<(String, String)>) -> Response {
    answer(store(&app, move |tx| {
        check_project(tx, &id)?;
        tx.execute("DELETE FROM supervisor_probe_commands WHERE project = ?1 AND probe = ?2", (&id, &probe)).map_err(sql)?;
        view(tx, &id, settings::read(tx, &id)?)
    }).await)
}
