// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The first prompt of a human's + Session (`terminum-dispatcher.md` AC-204): the human's words, typed once through
//! the gate (§9) into the pane that request started, then it is AC-142's session; never on the agent's command line.
//! A file the human added goes in as its stored path, on this Mac only.

use std::time::{Duration, Instant};

use super::press::MOST_PROMPT;
use super::runs::{self, Phase};
use super::screen::Screen;
use super::{gate::{self, has_stray}, start, store};
use crate::app::Shared;
use crate::tasks::files;

/// The files one prompt names, at most.
const MOST_FILES: usize = 20;

/// What the human asked to be typed: its words and its files' lines, or none for a session with nothing typed
/// (AC-142); refused before anything starts.
pub(super) async fn asked(app: &Shared, words: Option<&str>, ids: &[String], remote: bool) -> Result<Option<String>, String> {
    let words = words.map(str::trim).filter(|w| !w.is_empty());
    if words.is_some_and(has_stray) {
        return Err("The prompt has control or invisible characters.".into());
    }
    if words.is_some_and(|w| w.chars().count() > MOST_PROMPT) {
        return Err(format!("The prompt is longer than {MOST_PROMPT} characters: add it as a file."));
    }
    if ids.is_empty() {
        return Ok(words.map(str::to_string));
    }
    if ids.len() > MOST_FILES {
        return Err(format!("At most {MOST_FILES} files go with a prompt."));
    }
    if remote {
        return Err("Files go only to a session on this Mac: this project's agents run on another host.".into());
    }
    let (wanted, dir) = (ids.to_vec(), app.cfg.dir.clone());
    // a file not yet any task's (the composer's own upload): its name, its kind, where it is kept
    let found = store(app, move |tx| wanted.iter().map(|id| files::unattached(tx, &dir, id).map_err(|_| "A file to send is gone: add it again.".to_string())).collect::<Result<Vec<_>, _>>()).await?;
    let lines: Vec<String> = found.iter().map(|(f, path)| format!("- {} ({}): {}", crate::termtext::clean(&f.name, false), f.mime, path.display())).collect();
    let files = format!("Files the human added, kept with no extension; read each at its path:\n{}", lines.join("\n"));
    Ok(Some(match words { Some(w) => format!("{w}\n\n{files}"), None => files }))
}

#[derive(Clone, Copy)]
pub(super) enum Scope { Human, Supervisor }

impl Scope {
    pub(super) fn check(self, tx: &rusqlite::Transaction, run: &str) -> Result<(), String> {
        match self {
            Self::Human => Ok(()),
            Self::Supervisor => crate::supervisor::rounds::check_first_prompt(tx, run),
        }
    }
}

/// Types `text` once into run `id`'s pane: when its box stands empty, the run is made Working first, so no watch or
/// walk can type it again, then the gate pastes it and presses Enter. What missed, in words for the page.
pub(super) async fn type_once(app: &Shared, id: &str, text: &str, scope: Scope) -> Result<(), String> {
    let began = Instant::now();
    let mut empty_since: Option<Instant> = None;
    loop {
        let wanted = id.to_string();
        let r = store(app, move |tx| {
            scope.check(tx, &wanted)?;
            runs::by_id(tx, &wanted)
        }).await?;
        if !r.is_starting() {
            return Err("the session no longer waits for its first prompt".into());
        }
        if let Some((server, pane)) = r.place(app) {
            if let Some(code) = start::exit_of(&server, &pane).await {
                return Err(format!("the program ended with exit {code}"));
            }
            match gate::read(&server, &pane, &r.kind).await.unwrap_or(Screen::Unknown) {
                Screen::Empty => {
                    if start::ready(app, &server, &pane, &mut empty_since) {
                        let wanted = id.to_string();
                        let now = store(app, move |tx| {
                            scope.check(tx, &wanted)?;
                            let mut run = runs::by_id(tx, &wanted)?;
                            if !run.is_starting() {
                                return Err("the session no longer waits for its first prompt".into());
                            }
                            run.state = Phase::Working;
                            runs::save(tx, &run)?;
                            Ok(run)
                        }).await?;
                        return gate::deliver_scoped(app, &now, &server, &pane, text, true, scope).await.map_err(|m| m.to_string());
                    }
                }
                Screen::Asks => return Err("the agent asks a question first: answer it, then paste the prompt".into()),
                _ => empty_since = None,
            }
        }
        if began.elapsed() >= start::start_scaled(start::BOX_WITHIN) {
            return Err("its input box did not come in 60 s".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::ops::{create, NewTask};
    use crate::tasks::store::HUMAN;

    #[tokio::test]
    async fn ac204_a_prompt_is_vetted_before_anything_starts() {
        let (_root, app) = crate::testutil::app_with_tasks("firstprompt-vet");
        assert_eq!(asked(&app, None, &[], false).await, Ok(None), "no prompt: nothing is typed (AC-142)");
        assert_eq!(asked(&app, Some(" \n "), &[], false).await, Ok(None), "a blank one neither");
        assert_eq!(asked(&app, Some("  Fix TERM-12 TERM-15\nthen test  "), &[], false).await, Ok(Some("Fix TERM-12 TERM-15\nthen test".into())), "its words, trimmed, line ends kept");
        for bad in ["ring \u{7}", "\u{1b}[2J", "zero\u{200b}width"] {
            assert!(asked(&app, Some(bad), &[], false).await.unwrap_err().contains("control or invisible"), "{bad:?}");
        }
        let long = "x".repeat(MOST_PROMPT + 1);
        assert!(asked(&app, Some(&long), &[], false).await.unwrap_err().contains("add it as a file"));
    }

    #[tokio::test]
    async fn ac204_no_watch_takes_a_start_the_request_holds() {
        let (_root, app) = crate::testutil::app_with_tasks("firstprompt-hold");
        let run = app.tasks.with(|tx| runs::add(tx, &runs::Run { agent_id: "a".into(), project: "default".into(), ..Default::default() }, &[], runs::Started::of(runs::How::Session), 1)).unwrap();
        let held = |app: &Shared| app.dispatcher.starting.lock().contains(&run);
        let hold = super::super::Held::take(&app, |d| &d.starting, &run).unwrap();
        start::watch(&app, &run);                     // a walk's beat while the request types
        drop(hold);
        assert!(!held(&app), "the watch left it to the request: nothing marks its empty box AC-142's meanwhile");
        start::watch(&app, &run);
        assert!(held(&app), "once the request let go, the watch takes it");
    }

    #[tokio::test]
    async fn ac204_files_go_as_their_kept_paths_on_this_mac_only() {
        let (_root, app) = crate::testutil::app_with_tasks("firstprompt-files");
        let shot = files::keep(&app.tasks, &app.cfg.dir, "shot.png", b"\x89PNG\r\n\x1a\nrest", 1).unwrap();
        let blob = files::keep(&app.tasks, &app.cfg.dir, "dump.bin", &[0, 159, 255], 1).unwrap();
        let text = asked(&app, Some("Look at these"), &[shot.id.clone(), blob.id.clone()], false).await.unwrap().unwrap();
        let path = |id: &str| files::folder(&app.cfg.dir).join(id).display().to_string();
        assert!(text.starts_with("Look at these\n\nFiles the human added"), "{text}");
        assert!(text.contains(&format!("- shot.png (image/png): {}", path(&shot.id))) && text.contains(&format!("- dump.bin (application/octet-stream): {}", path(&blob.id))), "{text}");
        assert!(asked(&app, None, std::slice::from_ref(&shot.id), false).await.unwrap().unwrap().starts_with("Files the human added"), "files alone make a prompt");
        assert!(asked(&app, None, std::slice::from_ref(&shot.id), true).await.unwrap_err().contains("only to a session on this Mac"), "a remote project's agent cannot read a Mac's path");
        assert!(asked(&app, None, &["0".repeat(32)], false).await.unwrap_err().contains("gone"), "an id terminumd does not keep");
        assert!(asked(&app, None, &vec![shot.id.clone(); MOST_FILES + 1], false).await.unwrap_err().contains("At most"), "a bounded list");
        // a task's file is that task's: + Session sends only its own uploads
        app.tasks.with(|tx| {
            let t = create(tx, HUMAN, NewTask { title: "has a file".into(), ..Default::default() }, 2)?;
            files::attach(tx, std::slice::from_ref(&blob.id), &t.key, None)
        }).unwrap();
        assert!(asked(&app, None, &[blob.id], false).await.unwrap_err().contains("gone"), "attached elsewhere");
    }
}
