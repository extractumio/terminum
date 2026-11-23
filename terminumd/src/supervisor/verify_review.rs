// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-251: the opted-in Verify command checks the queue's pinned task and writes a typed failure.

use std::{path::PathBuf, time::Duration};

use axum::{Json, extract::{ConnectInfo, State}, http::StatusCode, response::{IntoResponse, Response}};
use rusqlite::{OptionalExtension, Transaction};
use serde::Deserialize;
use serde_json::json;

use crate::app::Shared;
use crate::integrate::git;
use crate::roles::peer::{self, Peer};
use crate::tasks::{api, integration, ops::{self, sql}};
use super::{design_gates as gates, settings, surfaces::{self, Tier}};

pub(crate) const MISSING_EXIT: i32 = 73;
const NO_VERIFY: &str = "This command runs inside its project's active integration Verify.";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request { task: String, pane: String, sha: String }

struct Verify {
    task: ops::Task,
    repo: PathBuf,
    reference: String,
    sha: String,
    attempt: String,
    handle: String,
    target_rerun: bool,
    revision: i64,
    map: surfaces::Surfaces,
}

pub(super) fn reference(target: &str) -> Result<String, String> {
    let (remote, branch) = integration::target_of(target)?;
    Ok(format!("refs/remotes/{remote}/{branch}"))
}

fn read(tx: &Transaction, request: &Request) -> Result<Option<Verify>, String> {
    let task = ops::get(tx, &request.task)?;
    let row: Option<(String, String, Option<String>, String)> = tx.query_row("SELECT v.commit_sha,
        v.attempt, v.target_sha, v.pane FROM integration_verifies v JOIN project_integration i USING (project)
        WHERE v.task = ?1 AND v.project = ?2 AND i.is_on AND v.pane IS NOT NULL",
        (&task.key, &task.project), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional().map_err(sql)?;
    let Some((sha, attempt, target, handle)) = row else { return Ok(None); };
    if target.as_deref().unwrap_or(&sha) != request.sha || crate::integrate::runner::pane_of(&handle) != request.pane {
        return Ok(None);
    }
    let (_, host, dir) = ops::project_place(tx, &task.project)?;
    if host.is_some() { return Err(NO_VERIFY.into()); }
    let cfg = integration::read(tx, &task.project)?;
    let saved = settings::read(tx, &task.project)?;
    let map = if super::register::advisor_enabled(tx, &task.project)? { saved.settings.charter.surfaces } else { Default::default() };
    Ok(Some(Verify { task, repo: integration::local_dir(&dir), reference: reference(&cfg.target)?, sha, attempt,
        handle, target_rerun: target.is_some(), revision: saved.revision, map }))
}

fn check(app: &Shared, request: &Request, verify: Verify) -> Result<Option<String>, String> {
    if verify.target_rerun || !verify.map.enabled() { return Ok(None); }
    let target = git::local_output(&verify.repo, &["rev-parse", "--verify", &verify.reference])?;
    let base = surfaces::fork(&verify.repo, target.trim(), &verify.sha)?;
    let diff = surfaces::changes(&verify.repo, &base, &verify.sha)?;
    let hit = verify.map.classify(&diff).filter(|hit| hit.tier >= Tier::T2);
    app.tasks.with(|tx| {
        let current = read(tx, request)?.ok_or(NO_VERIFY)?;
        if current.attempt != verify.attempt || current.revision != verify.revision || current.repo != verify.repo
            || current.reference != verify.reference || !current.map.enabled()
        {
            return Err("The project or surface map changed during Verify. Run Verify again.".into());
        }
        check_evidence(tx, &current.task, hit, &verify.attempt, ops::now())
    })
}

pub(super) fn check_evidence(tx: &Transaction, task: &ops::Task, hit: Option<surfaces::Match>,
    attempt: &str, at: i64) -> Result<Option<String>, String> {
    let Some(hit) = hit else { return Ok(None); };
    let tier = hit.tier.number();
    let evidence = hit.evidence.clone();
    let paths = hit.paths.clone();
    gates::observe(tx, task, hit, true, at)?;
    tx.execute("UPDATE supervisor_design_gates SET deadline = ?2 WHERE task = ?1 AND deadline = 0",
        (&task.key, at + gates::TIME_LIMIT)).map_err(sql)?;
    let gate = gates::get(tx, &task.key)?.ok_or("Missing design gate.")?;
    let missing = if gates::covers(tx, &gate, tier, &paths)? {
        gates::missing(tx, &gate, tier)?
    } else {
        Some("a review on a separate task for the expanded design")
    };
    let Some(missing) = missing else { return Ok(None); };
    let words = format!("T{tier} Verify needs {missing} on {}.\n\n{evidence}", task.id);
    let by = gates::author(tx, &task.project)?;
    let condition = format!("T{tier}: {missing}");
    if !ops::noticed(tx, &by, &task.key, "review_condition", &condition)? {
        ops::notice(tx, &by, &task.id, &words, json!({"design_gate":"review_missing","review_condition":condition}), at)?;
    }
    tx.execute("INSERT OR IGNORE INTO supervisor_design_verify_misses (task, attempt) VALUES (?1, ?2)",
        (&task.key, attempt)).map_err(sql)?;
    Ok(Some(words))
}

pub(crate) fn missed(tx: &Transaction, task: &str, attempt: &str) -> Result<bool, String> {
    tx.execute("DELETE FROM supervisor_design_verify_misses WHERE task = ?1 AND attempt = ?2",
        (task, attempt)).map(|count| count != 0).map_err(sql)
}

pub(crate) async fn run(State(app): State<Shared>, ConnectInfo(caller): ConnectInfo<Peer>, Json(request): Json<Request>) -> Response {
    let server = &app.tmux.local;
    if request.pane.len() > 17 || !request.pane.strip_prefix('%')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        || !git::is_commit(&request.sha) || caller.0.is_none()
    {
        return (StatusCode::FORBIDDEN, Json(json!({"error":NO_VERIFY}))).into_response();
    }

    // The runner opens the window before publishing the pane and its mark in the store.
    let verified = async {
        for _ in 0..40 {
            let verify = crate::http::off({
                let (app, request) = (app.clone(), request.clone());
                move || app.tasks.snapshot(super::SCHEMA, |tx| read(tx, &request))
            }).await?;
            if let Some(verify) = verify {
                let (_, mark) = crate::integrate::runner::parts(&verify.handle);
                let mark = mark.ok_or(NO_VERIFY)?;
                if peer::in_integration(caller, server, &request.pane, mark).await {
                    return Ok(verify);
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Err::<Verify,String>(NO_VERIFY.into())
    }.await;

    let id = request.task.clone();
    let result = match verified {
        Ok(verify) => {
            let a = app.clone();
            crate::http::off(move || check(&a, &request, verify)).await
        }
        Err(why) => Err(why),
    };
    if matches!(&result, Ok(Some(_))) { api::told(&app, &id); }
    match result {
        Ok(None) => Json(json!({"pass":true})).into_response(),
        Ok(Some(words)) => Json(json!({"pass":false,"message":words})).into_response(),
        Err(why) => (StatusCode::BAD_REQUEST, Json(json!({"error":why}))).into_response(),
    }
}

pub(super) fn project_dir(tx: &Transaction, project: &str) -> Result<PathBuf, String> {
    let (_, host, dir) = ops::project_place(tx, project)?;
    if host.is_some() || dir.is_empty() { return Err("This project needs a local Git checkout for surface history.".into()); }
    Ok(integration::local_dir(&dir))
}

