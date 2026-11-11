// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What the idle agents' exit leaves in the pane (`terminum-agents.md` AC-153): once the agent has
//! given the pane back to its shell, a block of plain lines says which agent ended, why and when,
//! its session, title, command line, folder and plan, and the line that resumes it in that folder.
//! They are the pane's output, not its input: `run-shell` writes them to the pane's tty on the tmux
//! server's own host (this Mac's or a host's), so nothing is typed and tmux's history keeps them.
//! Every word from the agent, the pane or a folder is shown with its control and invisible
//! characters as visible escapes, quoted for the shell, then for tmux's format and command parser.

use crate::app::Shared;
use crate::tmux::args;
use crate::tmux::hookstate::Started;

const DAY: u64 = 86_400;

/// What is known of an agent, read before the first key: its hooks' record goes with it.
#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    pub kind: String,
    /// its hooks' word (None: no hooks spoke for this agent)
    pub started: Option<Started>,
    /// the pane's folder, where the agent's own is not known
    pub pane_path: String,
    pub title: String,
    /// its plan's items done, and all
    pub plan: Option<(usize, usize)>,
    /// how long no one opened the pane, in seconds
    pub unopened: u64,
}

impl Summary {
    /// Pane `key`'s agent of kind `kind` as terminumd knows it at `now`.
    pub fn of(app: &Shared, key: &str, kind: &str, pane_path: &str, title: &str, unopened: u64, now: u64) -> Summary {
        Summary {
            kind: kind.to_string(),
            started: app.hooks.started(key).filter(|s| s.agent == kind),
            pane_path: pane_path.to_string(),
            title: title.to_string(),
            plan: app.plans.row(key, false, now).map(|r| (r.done, r.total)),
            unopened,
        }
    }

    /// The block's lines, a blank one around it; `shell`: the pane's, whose quoting the Resume line
    /// takes; `at`: when the agent ended (unix s).
    pub fn lines(&self, shell: &str, at: u64) -> Vec<String> {
        let st = self.started.clone().unwrap_or_default();
        let id = Some(st.session.as_str()).filter(|id| crate::dispatcher::good_id(id)).unwrap_or("");
        let days = self.unopened / DAY;
        let mut out = vec![String::new(), format!("── terminum ended this idle agent: not opened for {days} day{} · {} ──",
                                                  if days == 1 { "" } else { "s" }, crate::tasks::cron::stamp(at as i64))];
        let label = crate::cli::agentsetup::label(&self.kind);
        out.push(if id.is_empty() { format!("Agent:      {label}, its session not known (its hooks did not say)") }
                 else { format!("Agent:      {label}, session {id}") });
        if !self.title.trim().is_empty() {
            out.push(format!("Title:      {}", visible(self.title.trim())));
        }
        out.push(if st.argv.is_empty() { "Started as: not known (its hooks did not say: `terminum hooks install`, or an older terminum there)".into() }
                 else { format!("Started as: {}", visible(&st.argv)) });
        let (folder, whose) = match (st.folder.as_str(), st.folder_is_start) {
            ("", _) => (self.pane_path.as_str(), " (the pane's; the agent's own is not known)"),
            (f, true) => (f, ""),
            (f, false) => (f, " (where it last worked; where it started is not known)"),
        };
        if !folder.is_empty() {
            out.push(format!("Folder:     {}{whose}", visible(folder)));
        }
        if let Some((done, total)) = self.plan {
            out.push(format!("Plan:       {done} of {total} done"));
        }
        let resume = format!("{} {}", self.kind, crate::dispatcher::resume_words(&self.kind, id));
        let pick = if id.is_empty() { "   (pick it from the list)" } else { "" };
        // a folder the shell cannot take as one word, or one with a control character, is only shown above
        let cd = Some(folder).filter(|f| !f.is_empty() && !crate::termtext::has_control(f)).and_then(|f| crate::termtext::quote(shell, f));
        out.push(match cd {
            Some(q) => format!("Resume:     cd {q} && {resume}{pick}"),
            None => format!("Resume:     {resume}{pick}   (in its folder)"),
        });
        out.push(String::new());
        out
    }
}

/// `s` with each control or invisible character as a visible escape (`\n`, `\u{202e}`): a path
/// shown is the path that exists, and nothing reaches the terminal or a command line as a key.
fn visible(s: &str) -> String {
    s.chars().map(|c| match c {
        '\n' => "\\n".to_string(),
        '\t' => "\\t".to_string(),
        c if crate::termtext::control(c) => format!("\\u{{{:x}}}", c as u32),
        c => c.to_string(),
    }).collect()
}

/// The tmux command that writes `lines` to pane `pane`'s tty: each line one shell word of printf's
/// arguments (never its format), `#` doubled for run-shell's format, the whole quoted for tmux.
pub fn command(pane: &str, lines: &[String]) -> String {
    let words: Vec<String> = lines.iter().map(|l| args::literal_format(&crate::restore_lines::quoted(&visible(l)))).collect();
    let shell = format!("printf '%s\\n' {} > '#{{pane_tty}}'", words.join(" "));
    format!("run-shell -b -t {pane} {}", args::quote(&shell))
}

#[cfg(test)]
#[path = "idle_summary_tests.rs"]
mod tests;
