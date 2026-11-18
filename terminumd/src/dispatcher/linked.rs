// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Whether a task's linked session holds RUN (`terminum-dispatcher.md` AC-121, AC-122; `terminum-kanban.md`
//! AC-113): judged by its server's list read whole and fresh on a live link, never by a list not read now
//! (terminumd just started, a host off or not answering), where a live pane would look closed.

use std::collections::HashSet;

use crate::app::Shared;

pub(super) const NOT_READ: &str = "Its session's server is not read now: run it once that server answers, or unlink the session";

/// Each server's prefix of its pane keys, with the panes its list shows; None: that list is not read now.
#[derive(Default)]
pub(super) struct Linked(pub(super) Vec<(String, Option<HashSet<String>>)>);

impl Linked {
    pub(super) fn read(app: &Shared) -> Linked {
        Linked(app.tmux.all().iter().map(|s| {
            let up = s.connected();
            let keys = s.list.with_told(|t| t.filter(|t| t.ok && up).map(|t| t.keys.iter().cloned().collect()));
            (s.key(""), keys)
        }).collect())
    }

    /// Whether pane `key` is open; a server terminumd no longer has counts closed: its panes cannot come back.
    pub(super) fn open(&self, key: &str) -> Result<bool, String> {
        match self.0.iter().find(|(prefix, _)| key.strip_prefix(prefix.as_str()).is_some_and(|pane| !pane.contains(':'))) {
            None => Ok(false),
            Some((_, Some(keys))) => Ok(keys.contains(key)),
            Some((_, None)) => Err(NOT_READ.to_string()),
        }
    }
}
