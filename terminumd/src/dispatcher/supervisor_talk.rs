// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-244: the page's Talk press opens one marked pane and sends its case ID once.

use serde_json::Value;

use crate::app::Shared;
use crate::supervisor::talk;
use crate::tasks::{agents, ops};

use super::{firstprompt, places, pool, runs::{self, Run}, store};

pub(crate) async fn open(app: &Shared, incident: i64) -> Result<Value, String> {
    let (project, chosen) = store(app, move |tx| talk::chosen(tx, incident)).await?;
    let (agent, host, folder, server) = places::place(app, &project, Some(chosen)).await?;
    let (scoped, located, pool_agent) = (project.clone(), folder.clone(), agent.clone());
    let (run, case) = store(app, move |tx| {
        let (project, agent) = talk::chosen(tx, incident)?;
        if project != scoped || agent != pool_agent.id || host.is_some() || ops::project_place(tx, &project)?.2 != located
            || agents::in_project(tx, &agent, &project)?.is_none_or(|a| a.line != pool_agent.line || a.at_once != pool_agent.at_once)
        {
            return Err("The Supervisor's agent or project changed. Press Talk again.".into());
        }
        if let Some(full) = pool::Slots::read(tx)?.check_in(tx, &pool_agent, &project, Some(pool::Slot::Supervisor), None)? {
            return Err(full.words);
        }
        let at = ops::now();
        let like = Run::of(&pool_agent, host, located, project.clone(), None);
        let run = runs::add(tx, &like, &[], runs::Started::of(runs::How::Session), at)?;
        let case = talk::save(tx, incident, &project, &run, at)?;
        Ok((run, case))
    }).await?;
    let text = format!("Supervisor incident case {case}. Call get_case_file with its ID and discuss it with the owner.");
    places::Prompted { run, server, project, dir: folder, agent }.open(app, text, firstprompt::Scope::Supervisor).await
}
