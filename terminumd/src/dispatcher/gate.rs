// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The typing gate (`terminum-dispatcher.md` AC-121, AC-122, §9): the only way Dispatcher types. All in order, on the
//! pane's typing queue; any miss types nothing, or no Enter, and says which. The paste and the Enter
//! are tmux's own to refuse: it reads the mark and the exit in the same command that types.

use std::time::Duration;

use super::runs::Run;
use super::screen::{self, Screen};
use crate::app::Shared;
use crate::tmux::control::Control;
use crate::tmux::Server;

/// What a press types may hold line ends, nothing else unseen: tmux would refuse it after the run was written.
pub(super) fn has_stray(text: &str) -> bool {
    text.split(['\n', '\t']).any(crate::termtext::has_control)
}

/// How long what was pasted may take to show in the box.
const SHOWN: Duration = Duration::from_millis(1500);
/// A first prompt's: an agent still starting on a busy machine shows it seconds late, and one
/// given up on stands there unsent. Not scaled by a test's clock: the agent's own real time.
const SHOWN_FIRST: Duration = Duration::from_secs(30);

/// What missed.
#[derive(Debug, PartialEq)]
pub enum Miss {
    /// the box holds a draft, or a question stands there
    NotEmpty(&'static str),
    Other(String),
}

impl std::fmt::Display for Miss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Miss::NotEmpty(why) => f.write_str(why),
            Miss::Other(why) => f.write_str(why),
        }
    }
}

fn other(why: impl Into<String>) -> Miss {
    Miss::Other(why.into())
}

/// The screen of `kind`'s agent in `pane`.
pub async fn read(server: &Server, pane: &str, kind: &str) -> Result<Screen, String> {
    let rows = server.run(format!("capture-pane -e -p -t {pane}")).await.map_err(|e| format!("tmux did not give its screen: {e}"))?;
    Ok(screen::read(kind, &rows))
}

/// What the gate's other branch prints: the pane was no more `run`'s, or its agent's exit was recorded.
const REFUSED: &str = "terminum-gate-refused";

/// `line`, if at that moment the pane is still `run`'s with its agent's exit not recorded: tmux judges
/// both in the command that types; the answer is `line`'s own (`Control::run_if`).
pub(super) async fn guarded(client: &Control, run: &str, pane: &str, line: &str, agent_live: bool) -> Result<(), String> {
    guarded_front(client, run, pane, line, agent_live, None).await
}

async fn guarded_front(client: &Control, run: &str, pane: &str, line: &str, agent_live: bool, command: Option<&str>) -> Result<(), String> {
    let marked = format!("#{{==:#{{@terminum-run}},{run}}}");
    let test = if agent_live { format!("#{{&&:{marked},#{{==:#{{@terminum-exit}},}}}}") } else { marked };
    let test = match command {
        Some(name) => {
            let front = format!("#{{==:#{{pane_current_command}},{name}}}");
            let ready = format!("#{{&&:{front},#{{==:#{{pane_in_mode}},0}}}}");
            format!("#{{&&:{test},{ready}}}")
        }
        None => test,
    };
    match client.run_if(pane, &test, line, &format!("display -p {REFUSED}")).await? {
        said if said.iter().any(|line| line == REFUSED) && command.is_some() => Err("its run mark, live agent, foreground program or tmux mode no longer permits typing".into()),
        said if said.iter().any(|l| l == REFUSED) => Err("its pane is no more this run's, or its agent has ended".into()),
        _ => Ok(()),
    }
}

async fn check_scope(app: &Shared, run: &Run, scope: super::firstprompt::Scope) -> Result<(), Miss> {
    let id = run.id.clone();
    super::store(app, move |tx| {
        if super::giveup::was_given_up(tx, &id)? {
            return Err("the human gave up on this run".into());
        }
        scope.check(tx, &id)
    }).await.map_err(other)
}

enum Delivery<'a> {
    Normal { first: bool, scope: super::firstprompt::Scope },
    GiveUp { command: &'a str, since: std::time::Instant },
}

impl Delivery<'_> {
    fn command(&self) -> Option<&str> {
        match self {
            Self::Normal { .. } => None,
            Self::GiveUp { command, .. } => Some(command),
        }
    }

    async fn check(&self, app: &Shared, run: &Run, key: &str) -> Result<(), Miss> {
        match self {
            Self::Normal { scope, .. } => check_scope(app, run, *scope).await,
            Self::GiveUp { since, .. } if app.inputs.human_since(key, *since) => Err(other("a human typed in its pane")),
            Self::GiveUp { .. } => Ok(()),
        }
    }
}

/// Pastes `text` into `run`'s pane and sends it. `first`: its first prompt, no turn has ended yet:
/// the caller knows the agent has started.
pub async fn deliver(app: &Shared, run: &Run, server: &Server, pane: &str, text: &str, first: bool) -> Result<(), Miss> {
    deliver_scoped(app, run, server, pane, text, first, super::firstprompt::Scope::Human).await
}

pub(super) async fn deliver_scoped(app: &Shared, run: &Run, server: &Server, pane: &str, text: &str, first: bool, scope: super::firstprompt::Scope) -> Result<(), Miss> {
    let _held = server.typing(pane).lock_owned().await;
    deliver_locked(app, run, server, pane, text, Delivery::Normal { first, scope }).await
}

pub(super) async fn interrupt_and_deliver(app: &Shared, run: &Run, server: &Server, pane: &str, text: &str) -> Result<(), Miss> {
    let at = std::time::Instant::now();
    let _held = tokio::time::timeout(Duration::from_secs(5), app.inputs.act_queue(server, pane, &server.key(pane), at)).await
        .map_err(|_| other("its typing queue did not become available within 5 seconds"))?
        .ok_or_else(|| other("a human typed in its pane"))?;
    let rows = server.run(format!("display -p -t {pane} '#{{pane_current_command}}'")).await.map_err(other)?;
    let command = rows.first().ok_or_else(|| other("its foreground program is unknown"))?;
    if crate::tmux::titles::agent_of(command) != Some(run.kind.as_str()) {
        return Err(other("its coding agent no longer runs in front"));
    }
    match read(server, pane, &run.kind).await.map_err(other)? {
        Screen::Empty | Screen::Holds(_) => {}
        Screen::Asks => return Err(Miss::NotEmpty("the agent asks a question")),
        Screen::Unknown => return Err(other(format!("screen not known ({})", run.kind))),
    }
    let client = server.client(pane).ok_or_else(|| other("tmux does not answer"))?;
    let sent = guarded_front(&client, &run.id, pane, &format!("send-keys -t {pane} Escape"), true, Some(command)).await;
    let who = crate::inputs::Who::dispatcher("dispatcher.give_up", &run.id, &run.ids, &run.agent_id);
    if let Some((server, _)) = app.tmux.resolve(&server.key(pane)) {
        crate::inputs::wrote(app, who, "key", &server, pane, "<Esc>", sent.as_ref().err().cloned(), None);
    }
    sent.map_err(other)?;
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if app.inputs.human_since(&server.key(pane), at) {
            return Err(other("a human typed in its pane"));
        }
        let rows = server.run(format!("capture-pane -e -p -t {pane}")).await.map_err(other)?;
        match screen::read(&run.kind, &rows) {
            Screen::Holds(_) => return Err(Miss::NotEmpty("its input box holds a draft")),
            Screen::Asks => return Err(Miss::NotEmpty("the agent asks a question")),
            Screen::Empty if screen::interrupt_ready(&run.kind, &rows) => break,
            _ => {}
        }
        if tokio::time::Instant::now() >= until {
            return Err(other("its agent did not show an idle empty input box within 5 seconds"));
        }
    }
    deliver_locked(app, run, server, pane, text, Delivery::GiveUp { command, since: at }).await
}

async fn deliver_locked(app: &Shared, run: &Run, server: &Server, pane: &str, text: &str, delivery: Delivery<'_>) -> Result<(), Miss> {
    let command = delivery.command();
    let first = matches!(delivery, Delivery::Normal { first: true, .. });
    let said = server.run(format!("display -p -t {pane} '#{{@terminum-run}}\t#{{pane_in_mode}}\t#{{@terminum-exit}}\t#{{pane_current_command}}'")).await.map_err(|e| other(format!("tmux does not answer: {e}")))?;
    let fields: Vec<&str> = said.first().map_or(Vec::new(), |l| l.split('\t').collect());
    let field = |i: usize| fields.get(i).copied().unwrap_or("");
    if field(0) != run.id {
        return Err(other("its pane is not this run's"));
    }
    // its last screen still shows a box: a shell under it is told by the exit, or by its name
    if !field(2).is_empty() || crate::tmux::titles::SHELLS.contains(&crate::termtext::shell_name(field(3))) {
        return Err(other("the agent has ended: only its shell remains"));
    }
    if field(1) != "0" {
        return Err(other("the pane is in a tmux mode"));
    }
    if command.is_some_and(|name| field(3) != name) {
        return Err(other("its coding agent no longer runs in front"));
    }
    let key = server.key(pane);
    if !first && command.is_none() && (!run.turn_ended || app.hooks.working(&key)) {
        return Err(other("the agent is working"));
    }
    match read(server, pane, &run.kind).await.map_err(other)? {
        Screen::Empty => {}
        Screen::Holds(_) => return Err(Miss::NotEmpty("its input box holds a draft")),
        Screen::Asks => return Err(Miss::NotEmpty("the agent asks a question")),
        Screen::Unknown => return Err(other(format!("screen not known ({})", run.kind))),
    }
    delivery.check(app, run, &key).await?;
    let client = server.client(pane).ok_or_else(|| other("tmux does not answer"))?;
    let buffer = crate::page::typing::load_buffer(app, &client, server, text).await.map_err(other)?;
    let pasted = match delivery.check(app, run, &key).await {
        Ok(()) => guarded_front(&client, &run.id, pane, &format!("paste-buffer -p -b {buffer} -t {pane}"), true, command).await
            .map_err(|error| other(format!("tmux did not take the paste: {error}"))),
        Err(miss) => Err(miss),
    };
    let shown = async {
        pasted?;
        let until = tokio::time::Instant::now() + if first { SHOWN_FIRST } else { SHOWN };
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if super::start::exit_of(server, pane).await.is_some() {
                return Err(other("the agent ended before what was pasted showed: no Enter"));
            }
            if screen::shows(&read(server, pane, &run.kind).await.map_err(other)?, text) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= until {
                return Err(other("what was pasted did not show in its box: no Enter"));
            }
        }
    }.await;
    let _ = client.run(format!("delete-buffer -b {buffer}")).await;
    let entered = match shown {
        Ok(()) => async {
            delivery.check(app, run, &key).await?;
            app.dispatcher.remember_typed(&key, text);
            guarded_front(&client, &run.id, pane, &format!("send-keys -t {pane} Enter"), true, command).await.map_err(|e| other(format!("tmux did not take Enter: {e}")))
        }.await,
        Err(m) => Err(m),
    };
    // what went in, and what tmux refused of it (AC-147)
    if let Some((server, _)) = app.tmux.resolve(&key) {
        let words = if entered.is_ok() { format!("{text}<Enter>") } else { text.to_string() };
        let who = crate::inputs::Who::dispatcher("dispatcher.gate", &run.id, &run.ids, &run.agent_id);
        crate::inputs::wrote(app, who, "prompt", &server, pane, &words, entered.as_ref().err().map(Miss::to_string), None);
    }
    entered
}
