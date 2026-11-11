// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What the idle agents' exit leaves in the pane (`terminum-agents.md` AC-153): once the agent has
//! given the pane back to its shell, a block of plain lines says which agent ended, why and when,
//! its session, title, command line, folder and plan, and the line that resumes it in that folder.
//! They are the pane's output, not its input: `run-shell` writes them to the pane's tty on the tmux
//! server's own host (this Mac's or a host's), so nothing is typed and tmux's history keeps them.
//! Every word from the agent, the pane or a folder is shown with its control and invisible
//! characters as visible escapes (`termtext::escaped`), quoted for the shell, then for tmux's format
//! and command parser.

use crate::app::Shared;
use crate::tmux::args;
use crate::idle_exit::DAY;
use crate::termtext::escaped;
use crate::tmux::hookstate::Started;

/// What is known of an agent, read before the first key: its hooks' record goes with it.
#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    pub kind: String,
    /// its hooks' word (empty: no hooks spoke for this agent)
    pub started: Started,
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
            started: app.hooks.started(key).filter(|s| s.agent == kind).unwrap_or_default(),
            pane_path: pane_path.to_string(),
            title: title.to_string(),
            plan: app.plans.row(key, false, now).map(|r| (r.done, r.total)),
            unopened,
        }
    }

    /// The block's lines, a blank one around it; `shell`: the pane's, whose quoting the Resume line
    /// takes; `at`: when the agent ended (unix s).
    pub fn lines(&self, shell: &str, at: u64) -> Vec<String> {
        let st = &self.started;
        let id = Some(st.session.as_str()).filter(|id| crate::dispatcher::good_id(id)).unwrap_or("");
        let days = self.unopened / DAY;
        let mut out = vec![String::new(), format!("── terminum ended this idle agent: not opened for {days} day{} · {} ──",
                                                  if days == 1 { "" } else { "s" }, crate::tasks::cron::stamp(at as i64))];
        let label = crate::cli::agentsetup::label(&self.kind);
        out.push(if id.is_empty() { format!("Agent:      {label}, its session not known (its hooks did not say)") }
                 else { format!("Agent:      {label}, session {id}") });
        if !self.title.trim().is_empty() {
            out.push(format!("Title:      {}", escaped(self.title.trim())));
        }
        let launch = &st.launch;
        out.push(if launch.argv.is_empty() { "Started as: not known (its hooks did not say: `terminum hooks install`, or an older terminum there)".into() }
                 else { format!("Started as: {}", escaped(&launch.argv)) });
        let (folder, whose) = match (st.folder.as_str(), st.folder_is_start) {
            ("", _) => (self.pane_path.as_str(), " (the pane's; the agent's own is not known)"),
            (f, true) => (f, ""),
            (f, false) => (f, " (where it last worked; where it started is not known)"),
        };
        if !folder.is_empty() {
            out.push(format!("Folder:     {}{whose}", escaped(folder)));
        }
        if let Some((done, total)) = self.plan {
            out.push(format!("Plan:       {done} of {total} done"));
        }
        // the options it started with, each one word of the pane's shell, none with a control character
        let opts = launch.opts.as_ref().and_then(|o| o.iter()
            .map(|w| Some(w).filter(|w| !crate::termtext::has_control(w)).and_then(|w| crate::termtext::quote_word(shell, w)))
            .collect::<Option<Vec<_>>>());
        let with: String = opts.iter().flatten().map(|o| format!(" {o}")).collect();
        let words = crate::dispatcher::resume_words(&self.kind, "");
        let id_word = if id.is_empty() { String::new() } else { format!(" {id}") };
        // Codex's id right after its subcommand, so that no option before it takes it as a value
        let resume = if self.kind == "codex" { format!("codex {words}{id_word}{with}") } else { format!("{}{with} {words}{id_word}", self.kind) };
        let pick = match (id.is_empty(), opts.is_none() && !launch.argv.is_empty()) {
            (true, true) => "   (pick it from the list; add its options from Started as)",
            (true, false) => "   (pick it from the list)",
            (false, true) => "   (add its options from Started as)",
            (false, false) => "",
        };
        // a folder the shell cannot take as one word, or one with a control character, is only shown above
        // nu has no `&&`
        let cd = Some(folder).filter(|f| !f.is_empty() && !crate::termtext::has_control(f) && crate::termtext::shell_name(shell) != "nu")
            .and_then(|f| crate::termtext::quote(shell, f));
        out.push(match cd {
            Some(q) => format!("Resume:     cd {q} && {resume}{pick}"),
            None => format!("Resume:     {resume}{pick}   (in its folder)"),
        });
        out.push(String::new());
        out
    }
}

/// The tmux command that writes `lines` (`lines`' own, their words escaped) to pane `pane`'s tty: each
/// line one shell word of printf's arguments (never its format), `#` doubled for run-shell's format,
/// the whole quoted for tmux.
pub fn command(pane: &str, lines: &[String]) -> String {
    debug_assert!(lines.iter().all(|l| !crate::termtext::has_control(l)), "{lines:?}");
    let words: Vec<String> = lines.iter().map(|l| args::literal_format(&args::quote(l))).collect();
    let shell = format!("printf '%s\\n' {} > '#{{pane_tty}}'", words.join(" "));
    format!("run-shell -b -t {pane} {}", args::quote(&shell))
}

#[cfg(test)]
#[path = "idle_summary_tests.rs"]
mod tests;
