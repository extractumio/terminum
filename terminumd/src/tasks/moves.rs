// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A task's move to another project with its session (AC-113): the session follows, else the
//! task goes back, so the two stores never disagree (D02).

use super::api::store;
use super::ops::{self, Task};
use super::store::{DEFAULT_PROJECT, HUMAN};
use crate::app::Shared;
use crate::http::off;

/// Its session could not follow `task` (`why`): the task goes back to project `was`, and what
/// the answer says.
pub(super) async fn move_back(app: &Shared, task: Task, was: String, why: String) -> (Task, Option<String>) {
    let id = task.id.clone();
    let back = store(app, move |tx| {
        ops::change_project(tx, HUMAN, &id, &was, Some("move"), ops::now())?;
        ops::get(tx, &id)
    }).await;
    match back {
        Ok(t) => {
            let note = format!("Its session could not move ({why}): {} stays in its project.", t.id);
            (t, Some(note))
        }
        Err(e) => (task, Some(format!("The task moved, its session did not: {why}; moving the task back failed: {e}"))),
    }
}

/// A session goes where its task went, Default meaning no project (AC-113).
pub(super) async fn follow_session(app: &Shared, session: String, project: String) -> Result<(), String> {
    let a = app.clone();
    let joined = off(move || {
        let mut ps = a.projects.lock();
        if project == DEFAULT_PROJECT { ps.remove(&session) } else { ps.add_to(&session, &project, None) }
    }).await;
    crate::page::projects::tell(app);
    joined
}
