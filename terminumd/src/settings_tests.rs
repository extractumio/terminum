// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! settings.json (AC-19): defaults, the spec's messages, unknown keys, the hosts rewrite.

use super::*;

fn tmp(name: &str) -> std::path::PathBuf {
    crate::testutil::fresh_dir(&format!("settings-{name}")).join("settings.json")
}

#[test]
fn missing_or_empty_gives_every_default_and_no_message() {
    let (s, m) = parse("");
    assert_eq!((s.clone(), m.is_empty()), (Settings::default(), true));
    assert_eq!((s.port, s.tmux_socket.as_str(), s.tmux_history, s.size, s.restore), (47900, "terminum", 12000, Size::Latest, true));
    assert_eq!(s.themes.default, "local");
    assert_eq!(s.themes.hosts.get("*").map(String::as_str), Some("remote"));
    assert_eq!(s.page_json(), json!({"textSize": 13, "lines": "wrap", "screenMode": "auto", "widgets": true, "folds": true, "messages": []}));
    let (s, m) = load(Path::new("/nonexistent/terminum/settings.json"));
    assert_eq!((s, m), (Settings::default(), vec![]));
}

#[test]
fn the_spec_example_is_read_whole() {
    let text = r#"{"port": 47901, "tmux": {"socket": "tm-test", "history": 20000},
 "size": "largest", "restore": false,
 "themes": {"default": "local", "hosts": {"*": "remote", "devbox": "work"}, "sessions": {"notes": "paper"}},
 "page": {"textSize": 15, "lines": "grid", "screenMode": "never", "widgets": false, "folds": false},
 "hosts": {"devbox": {"ssh": ["-p", "2222", "alex@devbox.example"], "files": "enabled"}}}"#;
    let (s, m) = parse(text);
    assert!(m.is_empty(), "{m:?}");
    assert_eq!((s.port, s.tmux_socket.as_str(), s.tmux_history, s.size, s.restore), (47901, "tm-test", 20000, Size::Largest, false));
    assert_eq!(s.themes.hosts.get("devbox").map(String::as_str), Some("work"));
    assert_eq!(s.themes.sessions.get("notes").map(String::as_str), Some("paper"));
    assert_eq!(s.page, PageSettings { text_size: 15, lines: "grid".into(), screen_mode: "never".into(), widgets: false, folds: false });
    assert_eq!(s.hosts["devbox"]["files"], "enabled");
}

#[test]
fn the_ac19_table_messages_word_for_word() {
    let cases = [
        (r#"{"size": "biggest"}"#, "settings.json: size must be latest, largest or manual"),
        (r#"{"page": {"textSize": 300}}"#, "settings.json: page.textSize must be 8–40"),
        ("{not json", "settings.json: line 1: expected a value"),
    ];
    for (text, msg) in cases {
        let (s, m) = parse(text);
        assert_eq!(m, vec![msg.to_string()], "{text}");
        assert_eq!(s.size, Size::Latest);
        assert_eq!(s.page.text_size, 13, "the default is kept");
        assert_eq!(s.page_json()["messages"], json!([msg]));
    }
}

#[test]
fn a_wrong_value_keeps_only_that_default() {
    let (s, m) = parse(r#"{"page": {"textSize": 15, "lines": "spiral", "widgets": "yes"}, "port": 0, "restore": 1}"#);
    assert_eq!(s.page.text_size, 15);
    assert_eq!((s.page.lines.as_str(), s.page.widgets, s.port, s.restore), ("wrap", true, 47900, true));
    for want in ["page.lines must be wrap, grid or fit", "page.widgets must be true or false", "port must be 1–65535", "restore must be true or false"] {
        assert!(m.contains(&format!("settings.json: {want}")), "{want}: {m:?}");
    }
    let (_, m) = parse(r#"{"page": {"textSize": 12.5}, "tmux": {"socket": "../x", "history": 5}, "themes": {"hosts": {"a": "../evil"}, "default": 3}}"#);
    assert_eq!(m.len(), 5, "{m:?}");
    assert!(m.contains(&"settings.json: themes.hosts.a must be a theme name".to_string()));
}

#[test]
fn unknown_keys_are_reported_once_each_at_their_place() {
    let (s, m) = parse(r#"{"foo": 1, "page": {"bar": 2, "textSize": 20}, "tmux": {"baz": 3}, "themes": {"qux": 1}}"#);
    assert_eq!(s.page.text_size, 20);
    assert_eq!(m, vec![
        "settings.json: unknown key foo".to_string(),
        "settings.json: unknown key page.bar".into(),
        "settings.json: unknown key themes.qux".into(),
        "settings.json: unknown key tmux.baz".into(),
    ]);
    let (_, m) = parse(r#"{"page": 5, "themes": {"sessions": []}}"#);
    assert_eq!(m, vec!["settings.json: page must be an object".to_string(), "settings.json: themes.sessions must be an object".into()]);
}

#[test]
fn host_rules_merge_over_the_remote_default() {
    let (s, m) = parse(r#"{"themes": {"hosts": {"devbox": "work"}}}"#);
    assert!(m.is_empty());
    assert_eq!(s.themes.hosts.len(), 2, "* stays remote unless named");
    let (s, _) = parse(r#"{"themes": {"hosts": {"*": "local"}}}"#);
    assert_eq!(s.themes.hosts["*"], "local");
}

#[test]
fn syntax_errors_name_their_line_and_other_shapes_are_refused() {
    let (_, m) = parse("{\n  \"size\": \"latest\",\n  \"page\": {\"textSize\": 14,}\n}");
    assert_eq!(m, vec!["settings.json: line 3: trailing comma".to_string()]);
    let (_, m) = parse("{\"size\": ");
    assert_eq!(m, vec!["settings.json: line 1: the file ends too early".to_string()]);
    let (_, m) = parse("[1, 2]");
    assert_eq!(m, vec!["settings.json: expected an object".to_string()]);
    let (_, m) = parse(r#"{"hosts": [1]}"#);
    assert_eq!(m, vec!["settings.json: hosts must be an object".to_string()]);
}

#[test]
fn a_file_over_64_kb_is_refused_with_a_message() {
    let big = format!("{{\"page\": {{\"textSize\": 20}}, \"x\": \"{}\"}}", "a".repeat(MAX_BYTES));
    let (s, m) = parse(&big);
    assert_eq!((s.page.text_size, m), (13, vec!["settings.json: larger than 64 KB, not read".to_string()]));
    let p = tmp("big");
    fs::write(&p, &big).unwrap();
    assert_eq!(load(&p).1, vec!["settings.json: larger than 64 KB, not read".to_string()]);
    fs::write(&p, r#"{"size": "manual"}"#).unwrap();
    assert_eq!(load(&p).0.size, Size::Manual);
}

#[test]
fn save_hosts_rewrites_only_the_hosts_value() {
    let p = tmp("hosts");
    let text = "{\n  // not json\n}";
    fs::write(&p, text).unwrap();
    let err = save_hosts(&p, |h| {
        h.insert("x".into(), json!({}));
    })
    .unwrap_err();
    assert!(err.starts_with("settings.json: line 2:") && err.ends_with("fix it first"), "{err}");
    assert_eq!(fs::read_to_string(&p).unwrap(), text, "a broken file is left alone");

    let text = "{\n  \"size\":   \"largest\",\n  \"hosts\": {\"old\": {\"ssh\": [\"old.example\"]}},\n  \"page\": {\"textSize\": 15}\n}\n";
    fs::write(&p, text).unwrap();
    save_hosts(&p, |h| {
        h.remove("old");
        h.insert("devbox".into(), json!({"ssh": ["alex@devbox.example"], "files": null}));
    })
    .unwrap();
    let out = fs::read_to_string(&p).unwrap();
    assert!(out.starts_with("{\n  \"size\":   \"largest\",\n  \"hosts\": {\n"), "{out}");
    assert!(out.ends_with("},\n  \"page\": {\"textSize\": 15}\n}\n"), "{out}");
    let (s, m) = parse(&out);
    assert!(m.is_empty(), "{m:?}");
    assert_eq!((s.size, s.page.text_size), (Size::Largest, 15));
    assert_eq!(s.hosts.keys().collect::<Vec<_>>(), vec!["devbox"]);
}

#[test]
fn save_hosts_adds_the_key_or_the_file() {
    let p = tmp("add");
    save_hosts(&p, |h| {
        h.insert("devbox".into(), json!({"ssh": ["devbox.example"]}));
    })
    .unwrap();
    let (s, m) = load(&p);
    assert!(m.is_empty());
    assert_eq!(s.hosts["devbox"]["ssh"], json!(["devbox.example"]));

    fs::write(&p, "{\"size\": \"manual\"}  \n").unwrap();
    save_hosts(&p, |h| {
        h.insert("b".into(), json!({}));
    })
    .unwrap();
    let out = fs::read_to_string(&p).unwrap();
    assert!(out.starts_with("{\"size\": \"manual\",\n  \"hosts\": {"), "{out}");
    let (s, m) = parse(&out);
    assert!(m.is_empty());
    assert_eq!((s.size, s.hosts.len()), (Size::Manual, 1));
    fs::write(&p, "{}").unwrap();
    save_hosts(&p, |h| {
        h.insert("c".into(), json!({}));
    })
    .unwrap();
    assert!(parse(&fs::read_to_string(&p).unwrap()).1.is_empty());
    assert!(fs::read_dir(p.parent().unwrap()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().contains(".tmp-")));
}

#[test]
fn the_locator_finds_values_and_their_lines() {
    let text = "{\n \"a\": {\"b\": [1, \"x,]}\", {\"c\": true}]},\n \"d\":\n   \"#12\"\n}";
    assert_eq!(locate(text, &[Seg::Key("d")]).map(|s| (s.line, &text[s.start..s.end])), Some((4, "\"#12\"")));
    let c = locate(text, &[Seg::Key("a"), Seg::Key("b"), Seg::Index(2), Seg::Key("c")]).unwrap();
    assert_eq!((c.line, &text[c.start..c.end]), (2, "true"));
    let x = locate(text, &[Seg::Key("a"), Seg::Key("b"), Seg::Index(1)]).unwrap();
    assert_eq!(&text[x.start..x.end], "\"x,]}\"");
    assert_eq!(locate(text, &[Seg::Key("zz")]), None);
    assert_eq!(locate(text, &[Seg::Key("a"), Seg::Index(0)]), None);
    assert_eq!(line_of(text, &[Seg::Key("a"), Seg::Key("b"), Seg::Index(9)]), 1);
    assert!(is_name("work") && is_name("tm-test.2") && !is_name(".hidden") && !is_name("a/b") && !is_name(""));
}

/// AC-89: sessions of the user's own tmux, by socket; terminum's own socket is refused.
#[test]
fn attach_names_sessions_by_socket() {
    let (s, m) = parse(r#"{"attach": {"default": ["work", "notes"], "x y": ["a"], "other": ["a:b"]}}"#);
    assert_eq!(s.attach, BTreeMap::from([("default".to_string(), vec!["work".to_string(), "notes".to_string()])]));
    assert_eq!(m, vec!["settings.json: attach.other must be a list of tmux session names, under a socket name".to_string(),
        "settings.json: attach.x y must be a list of tmux session names, under a socket name".to_string()]);
    let (s, m) = parse(r#"{"attach": {"terminum": ["main"]}}"#);
    assert!(s.attach.is_empty());
    assert_eq!(m, vec!["settings.json: attach.terminum is terminum's own tmux server: its sessions are listed anyway".to_string()]);
    let p = tmp("attach");
    std::fs::write(&p, "{\n  \"port\": 47901\n}\n").unwrap();
    save_object(&p, "attach", |a| { a.insert("default".into(), json!(["work"])); }).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.starts_with("{\n  \"port\": 47901,\n  \"attach\": {"), "{text}");
    assert_eq!(load(&p).0.attach["default"], vec!["work".to_string()]);
}

/// AC-89: a host's attached sessions are kept in its record; a bad list is said and dropped.
#[test]
fn a_hosts_attached_sessions_are_checked() {
    let (s, m) = parse(r#"{"hosts": {"devbox": {"ssh": ["devbox"], "attach": ["work", "api"]}, "b": {"ssh": ["b"], "attach": ["terminum"]}, "c": {"ssh": ["c"], "attach": "x"}}}"#);
    assert_eq!(crate::attach::names_of(&s.hosts["devbox"]), vec!["work".to_string(), "api".to_string()]);
    assert!(s.hosts["b"].get("attach").is_none() && s.hosts["c"].get("attach").is_none() && s.hosts["c"]["ssh"] == json!(["c"]));
    assert_eq!(m, vec!["settings.json: hosts.b.attach must be a list of tmux session names (not terminum)".to_string(),
        "settings.json: hosts.c.attach must be a list of tmux session names (not terminum)".to_string()]);
}
