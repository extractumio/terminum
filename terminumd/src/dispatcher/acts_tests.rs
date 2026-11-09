// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! An End and a Close against a private tmux server (`-L tm-test-*`, never the user's), its "agent" a
//! `sleep` under the pane's shell: what a human does after the walk decided calls the act off or puts it
//! off (terminum-dispatcher.md AC-126, AC-125, AC-131).

use super::*;
use crate::tasks::store::{Store, HUMAN};

/// A run marked in a pane of its own whose shell runs `sleep`, as its agent.
struct Stage {
    tm: crate::testutil::Tmux,
    server: Arc<Server>,
    pane: String,
    app: Shared,
    root: std::path::PathBuf,
    run: Run,
}

impl Stage {
    async fn new(name: &str, phase: Phase, status: &str) -> Option<Stage> {
        let tm = crate::testutil::Tmux::start(name, (60, 12), "/bin/sh")?;
        let pane = tm.read(&["new-window", "-d", "-P", "-F", "#{pane_id}", "/bin/sh -c 'sleep 600; exec /bin/sh'"]).trim().to_string();
        let tmux = crate::config::find_program("tmux", "TM_TMUX").expect("tmux on PATH");
        let server = Server::new("local", &tm.socket, crate::tmux::server::Link::Local { tmux, socket: tm.socket.clone(), conf: "/dev/null".into() },
                                 Arc::new(tokio::sync::Notify::new()));
        server.sync_clients(&["main".to_string()]).unwrap();
        let until = tokio::time::Instant::now() + Duration::from_secs(3);
        while !server.attached() && tokio::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let (root, mut app) = crate::testutil::app(&format!("acts-{name}"));
        Arc::get_mut(&mut app).unwrap().tasks = Arc::new(Store::open(root.join("kanban.sqlite")));
        let key = server.key(&pane);
        let (st, mut s) = (status.to_string(), Stage { tm, server, pane, app, root, run: Run::default() });
        s.run = s.app.tasks.with(|tx| {
            let t = ops::create(tx, HUMAN, ops::NewTask { title: "one".into(), ..Default::default() }, 1)?;
            let like = Run { agent_id: "a".into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: "default".into(), ..Default::default() };
            let id = runs::add(tx, &like, std::slice::from_ref(&t.key), 1)?;
            super::super::move_one(tx, &t.key, &st, "set up")?;
            runs::change(tx, &id, |r| { r.state = phase; r.pane_key = Some(key); r.closing_for = plan::CLOSE; r.turn_ended = true; })
        }).unwrap();
        s.tm.read(&["set-option", "-p", "-t", &s.pane, "@terminum-run", &s.run.id]);
        Some(s)
    }

    /// Whether the pane's shell still has its `sleep`.
    fn agent_runs(&self) -> bool {
        let pid = self.tm.read(&["display", "-p", "-t", &self.pane, "#{pane_pid}"]);
        std::process::Command::new("pgrep").args(["-P", pid.trim()]).output().is_ok_and(|o| !o.stdout.is_empty())
    }

    fn pane_stands(&self) -> bool {
        self.tm.read(&["list-panes", "-a", "-F", "#{pane_id}"]).lines().any(|p| p == self.pane)
    }

    fn now(&self) -> Run {
        self.app.tasks.with(|tx| runs::by_id(tx, &self.run.id)).unwrap()
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        self.server.disconnect();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// TERM-32: an End a human's keys came after waits for the next walk; new work calls it off; else it ends the agent.
#[tokio::test]
async fn ac126_an_end_decided_before_a_humans_input_does_not_end_the_agent() {
    let Some(s) = Stage::new("end", Phase::Ending, "FAILED").await else { return };
    let key = s.server.key(&s.pane);
    let decided = Instant::now();
    s.app.inputs.human(&key);
    end_agent(&s.app, &s.run, &s.server, &s.pane, decided).await;
    assert!(s.agent_runs(), "a human typed after the decision: put off");
    assert_eq!(s.now().state, Phase::Ending, "the next walk decides again");

    let decided = Instant::now();
    s.app.tasks.with(|tx| runs::change(tx, &s.run.id, Run::new_work).map(drop)).unwrap();   // a human's prompt, as its hook does
    end_agent(&s.app, &s.run, &s.server, &s.pane, decided).await;
    assert!(s.agent_runs(), "new work after the decision: called off");
    assert_eq!(s.now().state, Phase::Working);

    s.app.tasks.with(|tx| runs::change(tx, &s.run.id, |r| r.state = Phase::Left).map(drop)).unwrap();   // Give up
    end_agent(&s.app, &s.run, &s.server, &s.pane, Instant::now()).await;
    assert!(s.agent_runs(), "given up after the decision: the window and its agent are the human's");
    assert_eq!(s.now().state, Phase::Left);

    s.app.tasks.with(|tx| runs::change(tx, &s.run.id, |r| r.state = Phase::Ending).map(drop)).unwrap();
    end_agent(&s.app, &s.run, &s.server, &s.pane, Instant::now()).await;
    assert!(!s.agent_runs(), "nothing came after it: the agent is ended");
    assert_eq!(s.now().state, Phase::Gone);
}

/// TERM-32: a Close a human's keys or prompt came after leaves the pane; else it closes it.
#[tokio::test]
async fn ac125_a_close_decided_before_a_humans_input_leaves_the_pane() {
    let Some(s) = Stage::new("close", Phase::Gone, "DONE").await else { return };
    let still = |_: &rusqlite::Transaction, r: &Run| Ok(plan::still_closes(r));
    let decided = Instant::now();
    s.app.inputs.human(&s.server.key(&s.pane));
    assert_eq!(close(&s.app, &s.run, (&s.server, &s.pane), "closed", false, decided, still).await, Ok(Some("a human typed in it".into())));
    assert!(s.pane_stands(), "a human typed after the decision");

    let decided = Instant::now();
    s.app.tasks.with(|tx| runs::change(tx, &s.run.id, Run::new_work).map(drop)).unwrap();
    assert_eq!(close(&s.app, &s.run, (&s.server, &s.pane), "closed", false, decided, still).await, Ok(Some("it is no longer idle".into())));
    assert!(s.pane_stands(), "new work after the decision: its 2 hours start again");

    s.app.tasks.with(|tx| runs::change(tx, &s.run.id, |r| r.closing_for = plan::CLOSE).map(drop)).unwrap();
    assert_eq!(close(&s.app, &s.run, (&s.server, &s.pane), "closed", false, Instant::now(), still).await, Ok(None));
    assert!(!s.pane_stands());
    assert_eq!(s.now().state, Phase::Closed);
}
