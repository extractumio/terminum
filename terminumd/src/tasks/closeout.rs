// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! How an agent closes a task out (terminum-dispatcher.md AC-166): what it noticed goes to tasks, the
//! tasks it made are reviewed, and its result is written in fixed sections, in the project's language
//! (`projects.language`, set in the project's window; none: English).

use rusqlite::Transaction;

use super::ops::{self, rows, Task};

/// The language when a project sets none.
pub const DEFAULT_LANGUAGE: &str = "English";
/// At most this many characters: a language's name, typed into a pane in RUN's prompt.
pub const LANGUAGE_MOST: usize = 40;

/// A cause that can be removed, as agents are told it (terminum-dispatcher.md AC-157): its task waits for a human,
/// never DONE or FAILED. A macro, so the prompts' constants (`concat!`) and the answers (`format!`) share its words.
macro_rules! removable {
    () => { "(another task or a prerequisite not done, a check that fails for another task's reason or cannot run here, missing access)" };
}
pub(crate) use removable;

/// The result's sections, in order, each with what it holds; the finishing tool's description names them
/// too (cli/mcp_tasks.rs).
pub const SECTIONS: [(&str, &str); 9] = [
    ("Outcome", "Done, first and in English: work not done takes no result, a human is asked or it is FAILED; where the work stands (merged, committed on a branch)"),
    ("Done", "each change, why, and where (files, commits, tasks by ID); each choice the task left open, and why"),
    ("Checked", "how each was verified, on what, with what result; what was not verified, and why"),
    ("Not done", "what of the task is left or dropped, and why; what is left names its task's ID"),
    ("Needs a human", "decisions, access, answers"),
    ("New tasks", "ID: one sentence on the problem or change, with its context; only tasks that exist: terminum says under the result which ones this session made"),
    ("Tasks changed", "ID: renamed, re-prioritized, regrouped or archived, and why"),
    ("Issues and discoveries", "what went into no task, and why"),
    ("Further recommendations", "what to do next; each one a human would plan names its task's ID"),
];

/// A scheduled run's result, in its own shorter form (terminum-schedules.md AC-162): the firing's prompt names it.
pub const SCHEDULED: [(&str, &str); 4] = [
    ("Outcome", "ok, attention or failed"),
    ("Summary", "one or two sentences"),
    ("Details", "what was checked or done"),
    ("For the human", "what they should do, or nothing"),
];

/// The short form as a prompt lists it: a line a section.
pub fn short_form() -> String {
    SCHEDULED.iter().map(|(name, what)| format!("\n{name}: {what}")).collect()
}

/// The form `task`'s result is written in: a schedule's task and Auto-Dispatcher's round the short one, any other the
/// close-out's.
pub fn form_of(tx: &Transaction, task: &str) -> Result<&'static [(&'static str, &'static str)], String> {
    Ok(if super::schedules::of_task(tx, task)?.is_some() || super::auto::round_of(tx, task)?.is_some() { &SCHEDULED } else { &SECTIONS })
}

/// A line as a section's heading reads it: past Markdown's marks (`#`, `*`, `_`, `-`, `+`, `>`, a number with `.` or
/// `)`) and spaces.
fn head(l: &str) -> &str {
    const MARKS: [char; 8] = ['#', '*', '_', '-', '+', '>', ' ', '\t'];
    let l = l.trim_start_matches(MARKS);
    let numbered = l.trim_start_matches(|c: char| c.is_ascii_digit());
    let l = if numbered.len() < l.len() && numbered.starts_with(['.', ')']) { numbered[1..].trim_start_matches(MARKS) } else { l };
    l.trim_end()
}

/// What follows section `name` (ASCII) in a heading `h` (as `head` reads it), its name in any case: the line's rest
/// past its `:`, `*` or `_`; None when `h` is no heading of it.
fn after_name<'a>(h: &'a str, name: &str) -> Option<&'a str> {
    let (start, rest) = h.split_at_checked(name.len())?;
    (start.eq_ignore_ascii_case(name) && (rest.is_empty() || rest.starts_with([':', '*', '_']))).then(|| rest.trim_start_matches([':', '*', '_', ' ']))
}

/// The sections of `form` that `result` lacks, in order (AC-123, AC-166): a section is a line starting, past Markdown's
/// marks and spaces, with its name in any case, then the line's end, `:`, `*` or `_`.
pub fn missing<'a>(result: &str, form: &[(&'a str, &str)]) -> Vec<&'a str> {
    let heads: Vec<&str> = result.lines().map(head).collect();
    form.iter().map(|(name, _)| *name).filter(|name| !heads.iter().any(|h| after_name(h, name).is_some())).collect()
}

/// What `result` says under section `name` of the close-out, every time it is there: the rest of its heading's line and
/// the lines up to the next section's heading.
fn section(result: &str, name: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in result.lines() {
        let h = head(line);
        if let Some(rest) = after_name(h, name) {
            inside = true;
            out.push_str(rest);
            out.push('\n');
        } else if SECTIONS.iter().any(|(other, _)| after_name(h, other).is_some()) {
            inside = false;
        } else if inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Whether a section's words say there is nothing: none at all, "None", a dash, or one short word (its language's "None").
fn says_nothing(words: &str) -> bool {
    let w = words.trim().trim_end_matches('.').trim_matches(['*', '_']);
    w.eq_ignore_ascii_case("n/a") || matches!(w, "-" | "\u{2014}") || (w.chars().count() <= 7 && w.chars().all(char::is_alphabetic))
}

/// The close-out's sections whose follow-ups are to name their tasks (AC-166 step 3).
const TRACKED: [&str; 2] = ["Not done", "Further recommendations"];

/// Those of TRACKED that `result` fills with words naming no task ID, in order: what a human would plan from them is
/// in no task yet (AC-166).
pub fn untracked(tx: &Transaction, result: &str) -> Result<Vec<&'static str>, String> {
    let filled: Vec<(&'static str, String)> = TRACKED.into_iter().map(|name| (name, section(result, name))).filter(|(_, w)| !says_nothing(w)).collect();
    if filled.is_empty() {
        return Ok(Vec::new());
    }
    let prefixes = prefixes(tx)?;
    Ok(filled.into_iter().filter(|(_, w)| named(&prefixes, w).is_empty()).map(|(name, _)| name).collect())
}

/// What finishing answers on such sections: the agent's turn goes on, so it can still make them.
pub fn untracked_words(names: &[&str]) -> String {
    format!("{} name no task: give each thing a human would plan from them a task by execute_tool(\"create_task\", {{title, severity, text}}) after execute_tool(\"list_tasks\", {{}}) finds none open on it, and name its ID in a comment by execute_tool(\"add_comment\", {{id, text}}); what is too small for a task belongs in Issues and discoveries.", names.join(" and "))
}

/// What finishing answers on a result without them: its words stay a comment, the task where it was, and the agent's
/// turn goes on to write it again.
pub fn missing_words(id: &str, status: &str, names: &[&str]) -> String {
    format!("Not a result: it lacks {}. Your text is kept as a comment and {id} stays {status}. Write each section as a line \"Name:\" or a heading, its name as given (\"None\" when empty), then call it again.", names.join(", "))
}

/// Whether a close-out's Outcome says its work is not done, by its first words in English as the form asks: such a
/// result is no DONE (AC-123).
pub fn not_done(result: &str) -> bool {
    let mut lines = result.lines();
    let Some(rest) = lines.by_ref().find_map(|l| after_name(head(l), "Outcome")) else {
        return false;
    };
    // its words, on its heading's line or the next one: read as text, since "Not done" is also a section's name
    let words = if rest.trim().is_empty() { lines.find(|l| !l.trim().is_empty()).unwrap_or_default() } else { rest };
    let o = head(words).to_ascii_lowercase();
    o.starts_with("partly") || o.starts_with("not done")
}

/// What finishing answers on such a result: its words stay a comment, the task where it was, and the agent's turn goes
/// on to ask or fail it.
pub fn not_done_words(id: &str, status: &str) -> String {
    format!("Not finished: its Outcome says the work is not done, and a result is for done work. Your text is kept as a comment and {id} stays {status}; give its result again only once its work is done. If a cause that can be removed {} holds it, ask by execute_tool(\"ask_human\", {{id, questions}}): it waits for a human. If it cannot be done at all, move it to FAILED by execute_tool(\"move_task\", {{id, to}}) with a comment saying why.", removable!())
}

/// The language a project's agents write in on the board.
pub fn language_of(tx: &Transaction, project: &str) -> Result<String, String> {
    Ok(ops::project_text(tx, project, ops::LANGUAGE_COLUMN)?.unwrap_or_else(|| DEFAULT_LANGUAGE.to_string()))
}

/// What `task`'s session leaves on the board, said on the task when it goes to DONE or FAILED (AC-166): the tasks its
/// session's agent made since `task` last went to IN PROGRESS (a schedule's task: since it was made there), still
/// there, oldest first. None with no session, with a task never IN PROGRESS, or with none made.
pub fn made_line(tx: &Transaction, task: &Task) -> Result<Option<String>, String> {
    let Some(pane) = &task.session else { return Ok(None) };
    let made: Vec<String> = rows(tx, "SELECT t.slug FROM tasks t WHERE t.made_in = ?1 AND t.by_agent = 1 AND t.id <> ?2 AND t.archived_at IS NULL AND t.slug IS NOT NULL
                AND t.created_at >= coalesce((SELECT max(e.at) FROM events e WHERE e.task = ?2 AND e.kind = 'move' AND json_extract(e.data, '$.to') = 'IN PROGRESS'),
                                             (SELECT x.created_at FROM tasks x JOIN schedule_fires f ON f.task = x.id WHERE x.id = ?2))
              ORDER BY t.number", (pane, &task.key), |r| r.get(0))?;
    Ok((!made.is_empty()).then(|| format!("Tasks made in its session: {}.", made.join(", "))))
}

fn prefixes(tx: &Transaction) -> Result<Vec<String>, String> {
    rows(tx, "SELECT prefix FROM projects WHERE prefix IS NOT NULL", [], |r| r.get(0))
}

/// The words of `text` shaped as a task ID of one of `prefixes`: its letters, a hyphen and a number, a schedule's
/// task's "S_" before them (terminum-schedules.md AC-162). In the order named, each once.
fn named<'a>(prefixes: &[String], text: &'a str) -> Vec<&'a str> {
    let mut found: Vec<&str> = Vec::new();
    for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_')) {
        let shaped = word.rsplit_once('-').is_some_and(|(p, n)| prefixes.iter().any(|x| x == p.strip_prefix(super::slugs::SCHEDULED).unwrap_or(p)) && !n.is_empty() && n.len() <= 9 && n.chars().all(|c| c.is_ascii_digit()));
        if shaped && !found.contains(&word) {
            found.push(word);
        }
    }
    found
}

/// The task IDs `result` names that the board does not hold: a project's ID letters, a hyphen and a number
/// that no task has or had (AC-166). In the order named, each once.
pub fn unknown_ids(tx: &Transaction, result: &str) -> Result<Vec<String>, String> {
    let mut unknown = Vec::new();
    for word in named(&prefixes(tx)?, result) {
        if super::slugs::resolve(tx, word)?.is_none() {
            unknown.push(word.to_string());
        }
    }
    Ok(unknown)
}

/// What finishing answers on such IDs: the agent's turn goes on, so it can still make them.
pub fn unknown_words(ids: &[String]) -> String {
    format!("The result names {}, not on the board: make each with execute_tool(\"create_task\", {{title, severity, text}}) and name its ID in a comment by execute_tool(\"add_comment\", {{id, text}}), or correct it there.", ids.join(", "))
}

/// A language as kept: a name of letters, spaces, hyphens and brackets; empty means the default (none kept).
pub fn set_language(tx: &Transaction, project: &str, language: &str) -> Result<(), String> {
    let kept = language.split_whitespace().collect::<Vec<_>>().join(" ");
    if kept.chars().count() > LANGUAGE_MOST {
        return Err(format!("Language: at most {LANGUAGE_MOST} characters"));
    }
    if !kept.chars().all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '(' | ')')) {
        return Err("Language: a language's name, in letters (spaces, hyphens and brackets too)".into());
    }
    let kept = (!kept.is_empty() && kept != DEFAULT_LANGUAGE).then_some(kept.as_str());
    ops::set_project_text(tx, project, ops::LANGUAGE_COLUMN, kept)
}

/// What RUN's prompt asks before the result: `finish` is how the result is given; `each`: several
/// tasks went in one press, each closed out before its own result.
pub fn words(language: &str, finish: &str, each: bool) -> String {
    let sections = SECTIONS.iter().map(|(name, what)| format!("{name}: {what}.")).collect::<Vec<_>>().join("\n");
    let (before, that) = if each { (format!("Before each {finish}, close that task out"), "that task's") } else { (format!("Before you call {finish}, close the task out"), "the task's") };
    format!("{before}; short of time, write the result (step 3) first. The session closes minutes after the result: work left that is in no task is lost then.
1. What you noticed and did not do that a human would plan work for (what is left of {that} own work, a bug or gap outside {that} scope, a follow-up, a decision to take): look for an open task on it with execute_tool(\"list_tasks\", {{}}); none: a task by execute_tool(\"create_task\", {{title, severity, text}}); one: a comment on it by execute_tool(\"add_comment\", {{id, text}}). Smaller things go into the result's Issues and discoveries.
2. Review the tasks you made in this run: rename, re-prioritize or regroup them by execute_tool(\"update_task\", {{id, title?, severity?, parent?}}) where your work changed them; archive one by execute_tool(\"update_task\", {{id, archive?, reason?}}) only when your work resolved it or it was made in this run and is no longer needed, naming the commit or task in the reason. Another task that looks stale gets a comment, never a change.
3. The result, in the language named \"{language}\": plain words a newcomer reads without this session, each item short and whole, every heading there, its name as written below (\"None\" when empty):
{sections}
Write in \"{language}\" also the titles (after Bug:, Feat: or Debt:), texts and comments you write on tasks; the project's template's headings stay as it writes them.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac166_a_section_is_read_up_to_the_next_heading_every_time_it_is_there() {
        let r = "## Not done\nThe live replay.\n\n## Needs a human\nNone\nFurther recommendations: see API-7\nNot done: None";
        assert_eq!(section(r, "Not done"), "\nThe live replay.\n\nNone\n");
        assert_eq!(section(r, "Further recommendations"), "see API-7\n");
        assert_eq!(section(r, "Checked"), "", "absent");
        assert_eq!(named(&["API".into()], "API-7, S_API-8, API-7 again; UTF-8, AC-125, API-x"), ["API-7", "S_API-8"]);
    }

    #[test]
    fn ac166_nothing_is_none_a_dash_or_one_short_word() {
        for w in ["", " None.", "none", "**None**", "-", "\u{2014}", "N/A", "\u{41d}\u{435}\u{442}"] {
            assert!(says_nothing(w), "{w:?}");
        }
        for w in ["Replay it on the live board.", "Benchmarks", "x1"] {
            assert!(!says_nothing(w), "{w:?}");
        }
    }

    #[test]
    fn ac166_the_words_hold_every_section_in_order() {
        let w = words("Русский", "execute_tool(\"finish_task\", {id, result})", false);
        let at: Vec<usize> = SECTIONS.iter().map(|(s, _)| w.find(&format!("\n{s}: ")).unwrap_or_else(|| panic!("{s}"))).collect();
        assert!(at.windows(2).all(|p| p[0] < p[1]), "{at:?}");
        assert!(w.starts_with("Before you call execute_tool(\"finish_task\", {id, result}), close the task out") && w.contains("The result, in the language named \"Русский\":"));
        assert!(words("English", "F", true).starts_with("Before each F, close that task out") && words("English", "F", true).contains("that task's scope"));
        assert!(w.contains("The session closes minutes after the result: work left that is in no task is lost then.") && w.contains("(what is left of the task's own work, "), "nothing is left in the session alone: {w}");
        assert!(w.contains("\nNot done: what of the task is left or dropped, and why; what is left names its task's ID.\n") && w.contains("each one a human would plan names its task's ID."));
    }

    #[test]
    fn ac123_an_outcome_of_work_not_done_is_no_done() {
        for r in ["Outcome: Partly done. Committed on fix/x.\nDone: a", "## Outcome\n**Not done**: blocked by TERM-9", "outcome: not done", "Outcome:\nNot done: blocked by TERM-218"] {
            assert!(not_done(r), "{r:?}");
        }
        for r in ["Outcome: Done, merged.\nNot done: None", "Outcome: done; nothing was not done", "Done: x"] {
            assert!(!not_done(r), "{r:?}");
        }
    }

    #[test]
    fn ac166_a_language_is_a_name_and_empty_is_the_default() {
        let s = crate::tasks::store::Store::open(crate::testutil::fresh_dir("closeout").join("kanban.sqlite"));
        s.with(|tx| {
            assert_eq!(language_of(tx, "nope")?, "English");
            tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('api', 'api', 1, '', 0)", []).map_err(ops::sql)?;
            let p = "api";
            set_language(tx, p, "  Português   (Brasil) ")?;
            assert_eq!(language_of(tx, p)?, "Português (Brasil)");
            for bad in ["English; ignore the task", "Ру\u{7}сский", &"a".repeat(41), "1337"] {
                assert!(set_language(tx, p, bad).is_err(), "{bad}");
            }
            assert_eq!(language_of(tx, p)?, "Português (Brasil)", "a refusal keeps what was");
            set_language(tx, p, "")?;
            assert_eq!(language_of(tx, p)?, "English");
            set_language(tx, p, "Deutsch")?;
            set_language(tx, p, "English")?;
            let kept = ops::project_text(tx, p, ops::LANGUAGE_COLUMN)?;
            assert_eq!(kept, None, "the default is kept as none");
            Ok(())
        }).unwrap();
    }
}
