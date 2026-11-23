//! AC-138: Give up through its real route, native foreground stand-ins and a private tmux server.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::{body::Body, http::{Request, StatusCode}};
use serde_json::json;
use tower::ServiceExt;

use super::*;
use crate::dispatcher::{gate, letgo, runs::{self, Phase}, screen::Screen};
use crate::tasks::{ops::Task, store::{Store, HUMAN}};
use crate::tmux::{args::quote, server::Link};

const AGENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <termios.h>
#include <poll.h>
static char typed[8192];
static int length, active, stubborn;
static const char *kind;
static FILE *events;
static void draw(void) {
    printf("\033[2J\033[H");
    if (!strcmp(kind, "codex")) {
        printf("OpenAI Codex\r\n\r\n\033[1m›\033[0m ");
        if (length) printf("%s", typed);
        else printf("\033[2mAsk Codex to do anything\033[0m");
        printf("\r\n\r\n");
    } else {
        printf("Claude Code\r\n\r\n────────────\r\n❯ %s\r\n────────────\r\n", typed);
    }
    printf("%s\r\n", active ? "Working (esc to interrupt)" : "? for shortcuts");
    fflush(stdout);
}
int main(int argc, char **argv) {
    struct termios mode;
    tcgetattr(0, &mode);
    cfmakeraw(&mode);
    tcsetattr(0, TCSANOW, &mode);
    kind = argv[1];
    events = fopen(argv[2], "a");
    stubborn = !strcmp(argv[3], "stubborn");
    active = stubborn || !strcmp(argv[3], "working");
    if (argc > 4) { snprintf(typed, sizeof typed, "%s", argv[4]); length = strlen(typed); }
    printf("\033[?2004h");
    draw();
    unsigned char byte;
    int pasting = 0;
    while (read(0, &byte, 1) == 1) {
        if (byte == 27) {
            struct pollfd ready = {0, POLLIN, 0};
            if (poll(&ready, 1, 30) > 0) {
                char marker[5];
                if (read(0, marker, 5) == 5 && !memcmp(marker, "[20", 3)) {
                    pasting = marker[3] == '0';
                    continue;
                }
            }
            fprintf(events, "ESC\n"); fflush(events); if (!stubborn) active = 0;
        } else if (!pasting && (byte == '\r' || byte == '\n')) {
            fprintf(events, "PROMPT:%s\n", typed); fflush(events);
            length = 0; typed[0] = 0;
        } else if (length < sizeof typed - 1) {
            typed[length++] = byte; typed[length] = 0;
        }
        draw();
    }
    return 0;
}
"#;

fn native_agent() -> &'static PathBuf {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let dir = crate::testutil::fresh_dir("giveup-native");
        let source = dir.join("agent.c");
        let binary = dir.join("agent");
        std::fs::write(&source, AGENT).unwrap();
        let built = std::process::Command::new("cc").arg(&source).arg("-o").arg(&binary).output().expect("C compiler");
        assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
        binary
    })
}

pub(super) struct Stage {
    pub(super) tm: crate::testutil::Tmux,
    pub(super) app: Shared,
    pub(super) pane: String,
    pub(super) tasks: Vec<Task>,
    pub(super) run: Run,
    log: PathBuf,
    database: PathBuf,
}

impl Stage {
    pub(super) async fn new(name: &str, kind: &str, state: &str, draft: &str) -> Self {
        let tm = crate::testutil::Tmux::start(name, (120, 30), "/bin/sh").expect("private tmux");
        let binary = tm.dir.join(kind);
        std::fs::copy(native_agent(), &binary).unwrap();
        let log = tm.dir.join("events");
        let command = [binary.to_str().unwrap(), kind, log.to_str().unwrap(), state, draft].map(quote).join(" ");
        let pane = tm.read(&["new-window", "-d", "-P", "-F", "#{pane_id}", &command]).trim().to_string();
        let link = Link::Local { tmux: crate::config::find_program("tmux", "TM_TMUX").unwrap(), socket: tm.socket.clone(), conf: "/dev/null".into() };
        let (root, mut app) = crate::testutil::app_linked(name, link);
        let database = root.join("kanban.sqlite");
        Arc::get_mut(&mut app).unwrap().tasks = Arc::new(Store::open(database.clone()));
        let server = &app.tmux.local;
        server.sync_clients(&["main".into()]).unwrap();
        let (tasks, run) = app.tasks.with(|tx| {
            let mut tasks = Vec::new();
            for title in ["An unrelated title", "Queued work"] {
                tasks.push(ops::create(tx, HUMAN, ops::NewTask { title: title.into(), ..Default::default() }, 1)?);
            }
            let like = Run { kind: kind.into(), agent_id: "a".into(), ..Default::default() };
            let id = runs::add(tx, &like, &tasks.iter().map(|task| task.key.clone()).collect::<Vec<_>>(), Default::default(), 1)?;
            for task in &tasks {
                ops::link(tx, DISPATCHER, &task.id, Some(&server.key(&pane)), 1)?;
                ops::move_to(tx, DISPATCHER, &task.id, "IN PROGRESS", 1)?;
            }
            tx.execute("UPDATE tasks SET status = 'INTEGRATION' WHERE id = ?1", [&tasks[1].key]).map_err(ops::sql)?;
            tx.execute("INSERT INTO integration_entries (task, project, commit_sha, result, author, queued_at) VALUES (?1, ?2, 'abc', 'kept result', 'human', 3)",
                (&tasks[1].key, &tasks[1].project)).map_err(ops::sql)?;
            let run = runs::change(tx, &id, |run| {
                run.state = Phase::Working;
                run.pane_key = Some(server.key(&pane));
                run.turn_ended = state == "idle";
            })?;
            Ok((tasks, run))
        }).unwrap();
        tm.read(&["set-option", "-p", "-t", &pane, "@terminum-run", &run.id]);
        tokio::time::timeout(Duration::from_secs(6), async {
            while !server.connected() { tokio::time::sleep(Duration::from_millis(20)).await; }
            crate::lists::refresh_server(&app, server).await;
            while matches!(gate::read(server, &pane, kind).await, Ok(Screen::Unknown) | Err(_)) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.expect("native stand-in ready");
        if state == "working" {
            let key = server.key(&pane);
            app.hooks.apply(&key, &json!({"agent": kind, "event": "SessionStart", "session": "fixture"}), std::time::Instant::now());
            app.hooks.apply(&key, &json!({"agent": kind, "event": "UserPromptSubmit", "session": "fixture", "prompt": "work"}), std::time::Instant::now());
            assert!(app.hooks.working(&key));
        }
        Self { tm, app, pane, tasks, run, log, database }
    }

    async fn give_up(&self) {
        let api = crate::dispatcher::api::routes().with_state(self.app.clone());
        let response = api.oneshot(Request::post(format!("/api/tasks/{}/give-up", self.tasks[0].id)).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        self.app.tasks.with(|tx| {
            assert_eq!(runs::by_id(tx, &self.run.id)?.state, Phase::Left);
            assert!(runs::holding_agent(tx, "a")?.is_none());
            assert_eq!(ops::get(tx, &self.tasks[0].id)?.status, "FAILED");
            assert_eq!(ops::get(tx, &self.tasks[1].id)?.status, "INTEGRATION");
            let queued: (String, String) = tx.query_row("SELECT commit_sha, result FROM integration_entries WHERE task = ?1", [&self.tasks[1].key], |row| Ok((row.get(0)?, row.get(1)?))).map_err(ops::sql)?;
            assert_eq!(queued, ("abc".into(), "kept result".into()));
            Ok(())
        }).unwrap();
        assert!(self.tm.read(&["list-panes", "-a", "-F", "#{pane_id}"]).lines().any(|pane| pane == self.pane));
    }

    pub(super) fn events(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for Stage {
    fn drop(&mut self) { self.app.tmux.local.disconnect(); }
}

#[tokio::test]
async fn ac138_give_up_interrupts_working_and_idle_claude_and_codex_once() {
    for kind in ["claude", "codex"] {
        for state in ["working", "idle"] {
            let stage = Stage::new(&format!("giveup-{kind}-{state}"), kind, state, "").await;
            stage.give_up().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(stage.events(), format!("ESC\nPROMPT:{}\n", words(&stage.run.ids)));
            assert!(!stage.events().contains("An unrelated title"));
            assert_eq!(stage.tm.read(&["display", "-p", "-t", &stage.pane, "#{pane_current_command}"]).trim(), kind);
            assert!(letgo::gave_up(&stage.app, &stage.tasks[0].id).await.is_err());
            assert!(gate::deliver(&stage.app, &stage.run, &stage.app.tmux.local, &stage.pane, "stale original assignment", true).await.is_err());
            assert!(!stage.events().contains("stale original"));
        }
    }
}

#[tokio::test]
async fn ac138_give_up_does_not_type_into_a_mode_wrong_mark_exited_or_other_program() {
    for condition in ["mode", "mark", "exit", "program", "shell"] {
        let stage = Stage::new(&format!("giveup-{condition}"), "claude", "working", "").await;
        match condition {
            "mode" => { stage.tm.read(&["copy-mode", "-t", &stage.pane]); }
            "mark" => { stage.tm.read(&["set-option", "-p", "-t", &stage.pane, "@terminum-run", "another-run"]); }
            "exit" => { stage.tm.read(&["set-option", "-p", "-t", &stage.pane, "@terminum-exit", "0"]); }
            "program" => { stage.tm.read(&["respawn-pane", "-k", "-t", &stage.pane, "/bin/sleep 300"]); }
            _ => { stage.tm.read(&["respawn-pane", "-k", "-t", &stage.pane, "/bin/sh"]); }
        }
        stage.give_up().await;
        assert_eq!(stage.events(), "", "{condition}");
        let history = serde_json::to_string(&stage.app.tasks.with(|tx| ops::history(tx, &stage.tasks[0].id)).unwrap()).unwrap();
        assert!(history.contains("Give up stop instruction not delivered"), "{condition}: {history}");
    }
}

#[tokio::test]
async fn ac138_give_up_does_not_submit_while_the_turn_stays_working() {
    let stage = Stage::new("giveup-stubborn", "claude", "stubborn", "").await;
    stage.give_up().await;
    assert_eq!(stage.events(), "ESC\n");
    let history = serde_json::to_string(&stage.app.tasks.with(|tx| ops::history(tx, &stage.tasks[0].id)).unwrap()).unwrap();
    assert!(history.contains("within 5 seconds"), "{history}");
}

#[tokio::test]
async fn ac138_give_up_orders_cancellation_after_an_already_accepted_delivery() {
    let stage = Stage::new("giveup-load-order", "claude", "idle", "").await;
    let barrier = "giveup-buffer";
    stage.tm.read(&["wait-for", "-L", barrier]);
    stage.tm.read(&["set-hook", "-g", "after-load-buffer", &format!("wait-for -L {barrier}")]);
    let (app, run, pane) = (stage.app.clone(), stage.run.clone(), stage.pane.clone());
    let delivery = tokio::spawn(async move { gate::deliver(&app, &run, &app.tmux.local, &pane, "original assignment", true).await });
    let loaded = tokio::time::timeout(Duration::from_secs(5), async {
        while stage.tm.read(&["list-buffers", "-F", "#{buffer_name}"]).trim().is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await;
    let (app, id) = (stage.app.clone(), stage.tasks[0].id.clone());
    let typing = stage.app.tmux.local.typing(&stage.pane);
    let owners = Arc::strong_count(&typing);
    let cancellation = tokio::spawn(async move { letgo::gave_up(&app, &id).await });
    let waiting = tokio::time::timeout(Duration::from_secs(4), async {
        while Arc::strong_count(&typing) <= owners {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await;
    let before = stage.app.tasks.with(|tx| ops::get(tx, &stage.tasks[0].id)).unwrap().status;
    stage.tm.read(&["set-hook", "-ug", "after-load-buffer"]);
    stage.tm.read(&["wait-for", "-U", barrier]);
    let delivered = delivery.await.unwrap();
    let cancelled = cancellation.await.unwrap();
    assert!(loaded.is_ok(), "buffer did not reach its barrier");
    assert!(waiting.is_ok(), "Give up did not reach the occupied typing queue");
    assert_eq!(before, "IN PROGRESS", "cancellation waits for the accepted delivery");
    assert_eq!(delivered, Ok(()));
    assert!(cancelled.is_ok(), "{cancelled:?}");
    stage.app.tasks.with(|tx| {
        assert_eq!(runs::by_id(tx, &stage.run.id)?.state, Phase::Left);
        assert_eq!(ops::get(tx, &stage.tasks[0].id)?.status, "FAILED");
        assert!(runs::holding_agent(tx, "a")?.is_none());
        assert!(was_given_up(tx, &stage.run.id)?);
        Ok(())
    }).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(stage.events(), format!("PROMPT:original assignment\nESC\nPROMPT:{}\n", words(&stage.run.ids)));
    assert_eq!(gate::read(&stage.app.tmux.local, &stage.pane, "claude").await, Ok(Screen::Empty));
    assert_eq!(stage.tm.read(&["list-buffers", "-F", "#{buffer_name}"]).trim(), "");
    assert!(gate::deliver(&stage.app, &stage.run, &stage.app.tmux.local, &stage.pane, "late assignment", true).await.is_err());
    assert!(!stage.events().contains("late assignment"));
}

#[tokio::test]
async fn ac138_busy_delivery_refuses_without_cancelling_and_human_input_before_typing_wins() {
    let stage = Stage::new("giveup-typing-busy", "claude", "idle", "").await;
    let held = stage.app.tmux.local.typing(&stage.pane).lock_owned().await;
    let result = letgo::gave_up(&stage.app, &stage.tasks[0].id).await;
    assert_eq!(result.unwrap_err(), "A prompt is being delivered in this session. Give up again once it finishes.");
    assert_eq!(stage.app.tasks.with(|tx| ops::get(tx, &stage.tasks[0].id)).unwrap().status, "IN PROGRESS");
    stage.app.tasks.with(|tx| {
        assert_eq!(runs::by_id(tx, &stage.run.id)?.state, Phase::Working);
        assert!(runs::holding_agent(tx, "a")?.is_some());
        assert!(!was_given_up(tx, &stage.run.id)?);
        Ok(())
    }).unwrap();
    let (app, run, pane) = (stage.app.clone(), stage.run.clone(), stage.pane.clone());
    let (started, saw_start) = tokio::sync::oneshot::channel();
    let delivery = tokio::spawn(async move {
        let decided = std::time::Instant::now();
        let _ = started.send(());
        gate::interrupt_and_deliver(&app, &run, &app.tmux.local, &pane, &words(&run.ids), decided).await
    });
    saw_start.await.unwrap();
    stage.app.inputs.human(&stage.app.tmux.local.key(&stage.pane));
    drop(held);
    assert!(delivery.await.unwrap().is_err());
    assert_eq!(stage.events(), "");
}

#[tokio::test]
async fn ac138_give_up_preserves_a_draft_and_durable_notice_precedes_archive_errors() {
    let stage = Stage::new("giveup-draft", "claude", "idle", "keep my draft").await;
    stage.give_up().await;
    assert_eq!(stage.events(), "ESC\n");
    assert!(matches!(gate::read(&stage.app.tmux.local, &stage.pane, "claude").await, Ok(Screen::Holds(text)) if text == "keep my draft"));
    stage.app.tasks.with(|tx| ops::archive(tx, HUMAN, &stage.tasks[0].id, true, 3)).unwrap();
    let reopened = Store::open(stage.database.clone());
    let key = stage.app.tmux.local.key(&stage.pane);
    let notice = reopened.with(|tx| check_tool(tx, &key, Some(&stage.tasks[0].id))).unwrap_err();
    assert_eq!(notice, words(std::slice::from_ref(&stage.tasks[0].id)));
    let alias = format!("\n {} \t", stage.tasks[0].id.to_lowercase());
    assert_eq!(reopened.with(|tx| check_tool(tx, &key, Some(&alias))).unwrap_err(), notice);
    drop(reopened);
    let who = crate::tasks::tools::Who { key: key.clone(), agent: "claude".into(), title: "Agent".into(), project: None, dir: None, supervisor: false };
    let notice = crate::agentapi::task_tool_of(&stage.app, who, None, false, "finish_task".into(), json!({"id": stage.tasks[0].id, "result": "Done"})).await.unwrap_err();
    assert!(notice.contains("The human gave up on this task"));
    stage.app.tasks.with(|tx| {
        let id = runs::add(tx, &stage.run, std::slice::from_ref(&stage.tasks[0].key), Default::default(), 4)?;
        runs::change(tx, &id, |run| run.pane_key = Some("tmux:local:other:%9".into()))?;
        assert!(check_tool(tx, "tmux:local:other:%9", Some(&stage.tasks[0].id)).is_ok());
        assert!(check_tool(tx, &key, Some(&stage.tasks[0].id)).is_err());
        runs::change(tx, &id, |run| run.pane_key = Some(key.clone()))?;
        assert!(check_tool(tx, &key, Some(&stage.tasks[0].id)).is_ok());
        Ok(())
    }).unwrap();
}

#[tokio::test]
async fn ac138_real_agent_receives_give_up_and_keeps_its_window() {
    if std::env::var_os("TM_TEST_REAL_GIVEUP").is_none() {
        eprintln!("not run: set TM_TEST_REAL_GIVEUP=1 for isolated real CLI checks");
        return;
    }
    for (kind, tui) in [("claude", "default"), ("claude", "fullscreen"), ("codex", "default")] {
        let stage = Stage::new(&format!("giveup-real-{kind}-{tui}"), kind, "idle", "").await;
        let config = stage.tm.dir.join("config");
        let work = stage.tm.dir.join("work");
        std::fs::create_dir(&config).unwrap();
        std::fs::create_dir(&work).unwrap();
        let config_path = quote(config.to_str().unwrap());
        let command = if kind == "claude" {
            std::fs::write(config.join(".claude.json"), json!({"hasCompletedOnboarding": true, "projects": {work.to_str().unwrap(): {"hasTrustDialogAccepted": true}}}).to_string()).unwrap();
            format!("env -i HOME={config_path} PATH=\"$PATH\" TERM=xterm-256color LANG=en_US.UTF-8 SHELL=/bin/sh CLAUDE_CONFIG_DIR={config_path} ANTHROPIC_AUTH_TOKEN=no-model-call ANTHROPIC_BASE_URL=http://127.0.0.1:9 DISABLE_AUTOUPDATER=1 CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 claude --safe-mode --tools '' --settings '{{\"tui\":\"{tui}\"}}'")
        } else {
            format!("env -i HOME={config_path} PATH=\"$PATH\" TERM=xterm-256color LANG=en_US.UTF-8 SHELL=/bin/sh CODEX_HOME={config_path} codex --no-daemon --no-alt-screen -c check_for_update_on_startup=false -c model_provider='\"fixture\"' -c model_providers.fixture.name='\"Fixture\"' -c model_providers.fixture.base_url='\"http://127.0.0.1:9/v1\"' -c model_providers.fixture.wire_api='\"responses\"' -c model_providers.fixture.requires_openai_auth=false")
        };
        let tmux = crate::config::find_program("tmux", "TM_TMUX").unwrap();
        let command = ["/bin/sh", "-c", crate::dispatcher::start::window_wrapper(), "sh", work.to_str().unwrap(), &command, tmux.to_str().unwrap(), ""].map(quote).join(" ");
        stage.tm.read(&["respawn-pane", "-k", "-e", &format!("HOME={}", config.display()), "-e", "SHELL=/bin/sh", "-t", &stage.pane, &command]);
        stage.tm.read(&["set-option", "-p", "-t", &stage.pane, "@terminum-run", &stage.run.id]);
        tokio::time::timeout(Duration::from_secs(40), async {
            let mut trusted = false;
            let mut ready_since = None;
            loop {
                let command = stage.app.tmux.local.run(format!("display -p -t {} '#{{pane_current_command}}'", stage.pane)).await.unwrap();
                let foreground = command.first().is_some_and(|command| crate::tmux::titles::agent_of(command) == Some(kind));
                if foreground && gate::read(&stage.app.tmux.local, &stage.pane, kind).await == Ok(Screen::Empty) {
                    let since = ready_since.get_or_insert_with(tokio::time::Instant::now);
                    if since.elapsed() >= Duration::from_secs(2) { break; }
                } else { ready_since = None; }
                let shown = stage.tm.read(&["capture-pane", "-p", "-t", &stage.pane]);
                if !trusted && (shown.contains("Yes, I trust") || shown.contains("Do you trust")) {
                    if shown.contains("❯ No, exit") {
                        stage.tm.read(&["send-keys", "-t", &stage.pane, "Down"]);
                    }
                    stage.tm.read(&["send-keys", "-t", &stage.pane, "Enter"]);
                    trusted = true;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }).await.unwrap_or_else(|_| panic!("{kind}/{tui} not ready: {}", stage.tm.read(&["capture-pane", "-p", "-t", &stage.pane])));
        stage.give_up().await;
        let history = serde_json::to_string(&stage.app.tasks.with(|tx| ops::history(tx, &stage.tasks[0].id)).unwrap()).unwrap();
        assert!(history.contains("Give up stop instruction delivered."), "{kind}/{tui}: {history}\n{}", stage.tm.read(&["capture-pane", "-p", "-t", &stage.pane]));
        let command = stage.tm.read(&["display", "-p", "-t", &stage.pane, "#{pane_current_command}"]);
        assert_eq!(crate::tmux::titles::agent_of(command.trim()), Some(kind), "{kind}/{tui}: {command}");
        eprintln!("AC-138 real {kind}/{tui}: stop instruction delivered; agent and window remain open; model endpoint is closed loopback port 9");
    }
}
