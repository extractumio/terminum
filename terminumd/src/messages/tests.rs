// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The message history in the store (AC-90, AC-147), its search (AC-151).

use super::search::{fts_query, page, Filter};
use super::*;
use crate::tasks::store::{Store, DISPATCHER, HUMAN};

fn store(tag: &str) -> Store {
    Store::open(crate::testutil::fresh_dir(tag).join("kanban.sqlite"))
}

fn msg(at: i64, pane: &str, text: &str) -> Message {
    Message { at, sender: HUMAN.into(), via: "page".into(), pane: pane.into(), host: "local".into(), cwd: "/Users/alex/api".into(), text: text.into(), ..Default::default() }
}

fn put(s: &Store, ms: &[Message]) -> Vec<i64> {
    s.with(|tx| ms.iter().map(|m| insert(tx, m)).collect()).unwrap()
}

fn texts(v: &serde_json::Value) -> Vec<String> {
    v["items"].as_array().unwrap().iter().map(|i| i["text"].as_str().unwrap().to_string()).collect()
}

#[test]
fn ac90_the_folder_s_history_by_age_its_pane_s_places_and_quote_numbers() {
    let s = store("messages-folder");
    let mut first = msg(1_000, "k1", "fix the tests\nand the lint");
    first.agent = Some("claude".into());
    first.line = Some(120);
    let mut other = msg(2_000, "k2", "make");
    other.host = "devbox".into();
    let mut third = msg(3_000, "k1", "see Quote #7 and Quote #0041");
    third.line = Some(180);
    put(&s, &[first, other, third, msg(4_000, "k1", "ls -la")]);
    let (n, last) = s.with(|tx| nth_of_folder(tx, "local", "/Users/alex/api", 0)).unwrap();
    assert_eq!((n, last.unwrap()["text"].as_str().map(str::to_string)), (3, Some("ls -la".into())), "another host is another folder");
    let oldest = s.with(|tx| nth_of_folder(tx, "local", "/Users/alex/api", 2)).unwrap().1.unwrap();
    assert_eq!((oldest["agent"].as_str(), oldest["line"].as_i64(), oldest["at"].as_i64(), oldest["key"].as_str()), (Some("claude"), Some(120), Some(1), Some("k1")));
    assert_eq!(s.with(|tx| nth_of_folder(tx, "local", "/Users/alex/api", usize::MAX)).unwrap(), (3, None));
    let places = s.with(|tx| of_pane(tx, "k1", 10)).unwrap();
    assert_eq!(places, [("see Quote #7 and Quote #0041".to_string(), 180, false), ("fix the tests\nand the lint".to_string(), 120, true)], "only those with a line");
    assert_eq!(s.with(|tx| highest_quote(tx, "k1")).unwrap(), 41);
    assert_eq!(s.with(|tx| highest_quote(tx, "k2")).unwrap(), 0);
    let mut older = msg(500, "k1", "kept before, added later");
    older.line = Some(5);
    put(&s, &[older]);
    assert_eq!(s.with(|tx| nth_of_folder(tx, "local", "/Users/alex/api", 0)).unwrap().1.unwrap()["text"], "ls -la", "newest by time, not by id");
    assert_eq!(s.with(|tx| of_pane(tx, "k1", 10)).unwrap().last().unwrap().0, "kept before, added later");
}

#[test]
fn ac147_old_messages_go_with_the_input_log_s_rows() {
    let s = store("messages-trim");
    let now = 400 * 86_400_000;
    put(&s, &[msg(now - 181 * 86_400_000, "k", "old one"), msg(now, "k", "a"), msg(now, "k", "b"), msg(now, "k", "c"), msg(now - 5, "k", "older, added last")]);
    s.with(|tx| trim(tx, now, 180, 2)).unwrap();
    let left = s.with(|tx| page(tx, &Filter::default(), false, None)).unwrap();
    assert_eq!(texts(&left), ["c", "b"]);
    let f = Filter { q: "old".into(), ..Default::default() };
    assert_eq!(s.with(|tx| page(tx, &f, true, None)).unwrap()["total"], 0, "its words left the index too");
}

#[test]
fn ac151_the_words_typed_never_become_syntax() {
    assert_eq!(fts_query("page history").as_deref(), Some(r#""page"* AND "history"*"#));
    assert_eq!(fts_query(r#""jump to message" -codex"#).as_deref(), Some(r#""jump to message" NOT "codex""#));
    assert_eq!(fts_query(r#"a AND (b* OR c:d) ^x"#).as_deref(), Some(r#""AND"* AND "OR"* AND "c:d)"*"#), "one letter dropped, operators are words");
    assert_eq!(fts_query(r#"say "it"#).as_deref(), Some(r#""say"* AND "it""#), "an open quote closes at the end");
    assert_eq!(fts_query("-codex"), None, "nothing to rank");
    assert_eq!(fts_query("   "), None);
    assert_eq!(fts_query(r#"he said "a""b""#).as_deref(), Some(r#""he"* AND "said"* AND "a" AND "b""#), "two phrases side by side");
}

#[test]
fn ac151_ranked_by_relevance_newest_by_a_cursor_and_filtered() {
    let s = store("messages-search");
    let mut ms = vec![msg(1_000, "k1", "deploy the staging build"), msg(2_000, "k1", "fix the board test"),
                      msg(3_000, "k2", "board board board: make the board wait for its selector"), msg(4_000, "k2", "Исправь поиск по сообщениям")];
    ms[1].agent = Some("claude".into());
    ms[2].sender = DISPATCHER.into();
    ms[2].project = Some("p1".into());
    put(&s, &ms);
    let f = |q: &str| Filter { q: q.into(), ..Default::default() };
    let best = s.with(|tx| page(tx, &f("boa"), true, None)).unwrap();
    assert_eq!(texts(&best), ["board board board: make the board wait for its selector", "fix the board test"], "BM25: the denser match first; a word is a prefix");
    assert_eq!(best["items"][1]["hl"], "fix the \u{2}board\u{3} test");
    assert_eq!(texts(&s.with(|tx| page(tx, &f("поиск"), true, None)).unwrap()), ["Исправь поиск по сообщениям"], "Cyrillic words, any case");
    assert_eq!(texts(&s.with(|tx| page(tx, &f("ИСПРАВЬ"), true, None)).unwrap()).len(), 1);
    let human = Filter { q: "board".into(), sender: Some(HUMAN.into()), ..Default::default() };
    assert_eq!(texts(&s.with(|tx| page(tx, &human, true, None)).unwrap()), ["fix the board test"], "Dispatcher's left out on asking");
    let agents = Filter { kind: Some("agent".into()), ..Default::default() };
    assert_eq!(texts(&s.with(|tx| page(tx, &agents, false, None)).unwrap()), ["fix the board test"]);
    let project = Filter { projects: vec!["p1".into()], ..Default::default() };
    assert_eq!(s.with(|tx| page(tx, &project, false, None)).unwrap()["total"], 1);
    let none = Filter { projects: vec![search::NONE.into()], ..Default::default() };
    assert_eq!(s.with(|tx| page(tx, &none, false, None)).unwrap()["total"], 3, "no project");
    // newest first, a page at a time by its cursor, steady while more come
    let many: Vec<Message> = (0..(search::PAGE as i64 + 5)).map(|i| msg(10_000 + i, "k3", &format!("step {i}"))).collect();
    put(&s, &many);
    let one = s.with(|tx| page(tx, &Filter::default(), false, None)).unwrap();
    assert_eq!((one["items"].as_array().unwrap().len(), one["total"].as_i64()), (search::PAGE, Some(search::PAGE as i64 + 9)));
    put(&s, &[msg(99_000, "k3", "a newer one")]);
    let two = s.with(|tx| page(tx, &Filter::default(), false, one["next"].as_str())).unwrap();
    assert_eq!(texts(&two)[0], "step 4", "the cursor goes on where the page ended");
    assert_eq!((two["items"].as_array().unwrap().len(), two["total"].is_null()), (9, true), "counted with the first page only");
    assert!(two["next"].is_null());
    let ranked = s.with(|tx| page(tx, &f("step"), true, None)).unwrap();
    let rest = s.with(|tx| page(tx, &f("step"), true, ranked["next"].as_str())).unwrap();
    assert_eq!(ranked["next"], "50");
    assert_eq!(rest["items"].as_array().unwrap().len(), 5);
}

#[test]
fn ac151_days_in_local_time_and_a_message_s_neighbours() {
    let s = store("messages-days");
    let local_noon: i64 = s.with(|tx| tx.query_row("SELECT CAST(strftime('%s', '2026-11-02 12:00', 'utc') AS INTEGER) * 1000", [], |r| r.get(0)).map_err(sql)).unwrap();
    let ids = put(&s, &[msg(local_noon, "k1", "monday one"), msg(local_noon + 1, "k2", "elsewhere"), msg(local_noon + 2, "k1", "monday two"),
                        msg(local_noon + 86_400_000, "k1", "tuesday")]);
    let days = s.with(|tx| search::days(tx, &Filter { from: Some("2026-11-03".into()), ..Default::default() })).unwrap();
    assert_eq!(serde_json::json!(days), serde_json::json!([{"day": "2026-11-02", "n": 3}, {"day": "2026-11-03", "n": 1}]), "the strip leaves the dates out");
    let tuesday = Filter { from: Some("2026-11-03".into()), to: Some("2026-11-03".into()), ..Default::default() };
    let got = s.with(|tx| page(tx, &tuesday, false, None)).unwrap();
    assert_eq!((texts(&got), got["items"][0]["day"].as_str()), (vec!["tuesday".to_string()], Some("2026-11-03")), "its local day, as the strip counts");
    let bad = Filter { from: Some("2026-11-03'; DROP".into()), ..Default::default() };
    assert_eq!(s.with(|tx| page(tx, &bad, false, None)).unwrap()["total"], 4, "a day that is not one is no filter");
    let c = s.with(|tx| search::context(tx, ids[2], 3, 3)).unwrap().unwrap();
    let side = |k: &str| c[k].as_array().unwrap().iter().map(|i| i["text"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!((side("before"), side("after")), (vec!["monday one".to_string()], vec!["tuesday".to_string()]), "its own pane only");
    assert!(s.with(|tx| search::context(tx, 999, 3, 3)).unwrap().is_none());
}

#[test]
fn ac90_an_agents_own_turns_are_told_apart_and_the_trim_takes_those_kept_before() {
    for t in ["<task-notification>\n<task-id>b1</task-id>", " \n<cross-session-message from=\"x\">", "<agent-message from=\"a\">ok", "[Cross-session delivery notice] sent", "[status timer] x"] {
        assert!(injected(t), "{t}");
    }
    for t in ["fix the <task-notification> parser", "[Image #1] the strip", "<pasted_content id=\"1\">text", "[terminum Dispatcher] T-1: its time budget is off."] {
        assert!(!injected(t), "{t}");
    }
    let s = store("messages-injected");
    let hook = |at, text: &str| Message { via: "hook".into(), ..msg(at, "k1", text) };
    put(&s, &[hook(1_000, "<task-notification>\n<task-id>b1</task-id>"), hook(2_000, "\n <agent-message from=\"a\">ok"), hook(3_000, "[Cross-session idle notice] x"), hook(3_500, "[status timer] x"),
              hook(4_000, "<pasted_content id=\"1\">mine"), hook(5_000, "fix it"), msg(6_000, "k1", "<task-notification> as a shell's text")]);
    s.with(|tx| trim(tx, 7_000, 30, 2000)).unwrap();
    let left = s.with(|tx| page(tx, &Filter::default(), false, None)).unwrap();
    assert_eq!(texts(&left), ["<task-notification> as a shell's text", "fix it", "<pasted_content id=\"1\">mine"]);
    let f = Filter { q: "agent".into(), ..Default::default() };
    assert_eq!(s.with(|tx| page(tx, &f, true, None)).unwrap()["total"], 0, "the search forgets them too");
}

#[test]
fn ac151_a_message_s_tasks_its_agent_and_the_quick_filters_counts() {
    let s = store("messages-facets");
    let mut ms = vec![msg(1_000, "k1", "ls"), msg(2_000, "k1", "fix the board"), msg(3_000, "k2", "TERM-12: do it"), msg(4_000, "k2", "go on"), msg(5_000, "k3", "review it")];
    ms[1].agent = Some("claude".into());
    (ms[2].agent, ms[2].sender, ms[2].tasks) = (Some("codex".into()), DISPATCHER.into(), Some("TERM-12,S_TERM-3".into()));
    (ms[3].agent, ms[3].tasks) = (Some("codex".into()), Some("TERM-12".into()));
    (ms[4].agent, ms[4].sender) = (Some("claude".into()), crate::inputs::MANAGER.into());
    s.with(|tx| crate::tasks::users::agent_named(tx, crate::inputs::MANAGER.into(), "Manager")).unwrap();
    put(&s, &ms);
    let got = |f: Filter| s.with(|tx| page(tx, &f, false, None)).unwrap();
    let all = got(Filter::default());
    assert_eq!((all["items"][2]["tasks"].clone(), all["items"][4].get("tasks")), (serde_json::json!(["TERM-12", "S_TERM-3"]), None), "the tasks it was sent for; none: no field");
    assert_eq!(texts(&got(Filter { task: Some("TERM-12".into()), ..Default::default() })), ["go on", "TERM-12: do it"]);
    assert_eq!(texts(&got(Filter { task: Some("TERM-3".into()), ..Default::default() })).len(), 0, "a whole ID, never a part of one");
    assert_eq!(texts(&got(Filter { task: Some("S_TERM-3".into()), ..Default::default() })), ["TERM-12: do it"], "an ID's _ is no wildcard");
    assert_eq!(texts(&got(Filter { kind: Some("codex".into()), ..Default::default() })), ["go on", "TERM-12: do it"], "one agent's prompts");
    assert_eq!(texts(&got(Filter { sender: Some(crate::inputs::MANAGER.into()), ..Default::default() })), ["review it"]);
    let facets = |f: Filter| s.with(|tx| search::facets(tx, &f)).unwrap();
    let project = serde_json::json!({"": 5});
    assert_eq!(facets(Filter::default()), serde_json::json!({"sender": {"human": 3, "dispatcher": 1, "manager": 1}, "agent": {"": 1, "claude": 2, "codex": 2}, "project": project}));
    // a filter's own choice does not narrow its own counts; the others' do
    assert_eq!(facets(Filter { sender: Some(HUMAN.into()), ..Default::default() }), serde_json::json!({"sender": {"human": 3, "dispatcher": 1, "manager": 1}, "agent": {"": 1, "claude": 1, "codex": 1}, "project": {"": 3}}));
    assert_eq!(facets(Filter { kind: Some("codex".into()), projects: vec!["gone".into()], ..Default::default() }), serde_json::json!({"sender": {}, "agent": {}, "project": {"": 2}}));
    assert_eq!(facets(Filter { kind: Some("shell".into()), q: "board".into(), ..Default::default() }), serde_json::json!({"sender": {}, "agent": {"claude": 1}, "project": {}}), "the words narrow them too");
}
