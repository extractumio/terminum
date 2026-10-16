// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Themes (AC-12): defaults, rules, the light variant, parse errors with lines, last good.

use super::*;
use crate::settings::parse;
use serde_json::json;

fn dir(name: &str) -> PathBuf {
    crate::testutil::fresh_dir(&format!("themes-{name}"))
}

const WORK: &str = r##"{
  "name": "work",
  "font": "Menlo",
  "size": 13,
  "bg": "#000010",
  "fg": "#e0e0e0",
  "cursor": "#ffcc00",
  "light": {"bg": "#ffffff", "fg": "#111111"}
}"##;

#[test]
fn the_shipped_themes_are_written_once_and_valid() {
    let d = dir("defaults");
    ensure_defaults(&d).unwrap();
    assert_eq!(fs::read_to_string(d.join("local.json")).unwrap(), LOCAL);
    fs::write(d.join("local.json"), WORK).unwrap();
    ensure_defaults(&d).unwrap();
    assert_eq!(fs::read_to_string(d.join("local.json")).unwrap(), WORK, "a user's file is never overwritten");
    for (text, file) in [(LOCAL, "local.json"), (REMOTE, "remote.json")] {
        let (t, w) = validate(text, file).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(t["font"], "JetBrainsMonoNFM-Regular");
        assert_eq!(t["size"], 14);
        assert_eq!(t["ansi"].as_array().unwrap().len(), 16);
    }
}

#[test]
fn a_fresh_install_shows_local_here_and_remote_on_hosts() {
    let d = dir("fresh");
    ensure_defaults(&d).unwrap();
    let (themes, m) = Themes::load(&d);
    assert!(m.is_empty(), "{m:?}");
    assert_eq!(themes.names(), vec!["local", "remote"]);
    let s = Settings::default();
    let local = themes.resolve(&s, None, "main", true);
    assert_eq!((local.name.as_str(), local.message.clone()), ("local", None));
    assert_eq!((local.theme["bg"].as_str(), local.theme["dark"].as_bool()), (Some("#322925"), Some(true)));
    assert!(local.theme.get("light").is_none(), "the page gets one resolved variant");
    let remote = themes.resolve(&s, Some("devbox"), "terminum", true);
    assert_eq!((remote.name.as_str(), remote.theme["bg"].as_str()), ("remote", Some("#311d30")));
}

#[test]
fn rules_sessions_over_hosts_over_star_over_default() {
    let d = dir("rules");
    ensure_defaults(&d).unwrap();
    fs::write(d.join("work.json"), WORK).unwrap();
    fs::write(d.join("paper.json"), r##"{"bg": "#fdf6e3"}"##).unwrap();
    let (themes, m) = Themes::load(&d);
    assert!(m.is_empty(), "{m:?}");
    let (s, _) = parse(r#"{"themes": {"hosts": {"devbox": "work"}, "sessions": {"notes": "paper"}},
        "hosts": {"other": {"ssh": ["other.example"], "theme": "paper"}}}"#);
    let name = |host: Option<&str>, session: &str| {
        let n = themes.resolve(&s, host, session, true).name;
        assert_eq!(themes.name_of(&s, host, session), n, "name_of agrees with resolve");
        n
    };
    assert_eq!(name(Some("devbox"), "x"), "work");
    assert_eq!(name(Some("devbox"), "notes"), "paper", "sessions win");
    assert_eq!(name(None, "notes"), "paper");
    assert_eq!(name(Some("elsewhere"), "x"), "remote", "* for any other remote host");
    assert_eq!(name(None, "x"), "local", "* never applies on this Mac");
    assert_eq!(name(Some("other"), "x"), "paper", "a host record's own theme");
    let paper = themes.resolve(&s, None, "notes", true).theme;
    assert_eq!((paper["bg"].as_str(), paper["fg"].as_str()), (Some("#fdf6e3"), Some("#e2ded9")), "missing fields from local");
}

#[test]
fn the_light_variant_follows_the_system_and_others_stay() {
    let d = dir("light");
    fs::write(d.join("work.json"), WORK).unwrap();
    let (themes, _) = Themes::load(&d);
    let (s, _) = parse(r#"{"themes": {"default": "work"}}"#);
    let dark = themes.resolve(&s, None, "x", true).theme;
    let light = themes.resolve(&s, None, "x", false).theme;
    assert_eq!((dark["bg"].as_str(), dark["dark"].as_bool()), (Some("#000010"), Some(true)));
    assert_eq!((light["bg"].as_str(), light["fg"].as_str(), light["cursor"].as_str(), light["dark"].as_bool()),
        (Some("#ffffff"), Some("#111111"), Some("#ffcc00"), Some(false)));
    let local = themes.resolve(&Settings::default(), None, "x", false).theme;
    assert_eq!((local["bg"].as_str(), local["dark"].as_bool()), (Some("#322925"), Some(true)), "no light variant: stays");
    let remote = themes.resolve(&Settings::default(), Some("h"), "x", false).theme;
    assert_eq!((remote["bg"].as_str(), remote["dark"].as_bool()), (Some("#311d30"), Some(true)), "the shipped remote theme stays violet");
}

#[test]
fn a_bad_color_names_its_line() {
    let text = "{\n  \"name\": \"work\",\n  \"font\": \"Menlo\",\n  \"size\": 13,\n  \"bg\": \"#000010\",\n  \"fg\": \"#e0e0e0\",\n  \"cursor\": \"yellow\"\n}";
    assert_eq!(validate(text, "work.json").unwrap_err(), "Theme work.json: line 7: expected a color");
    let ansi = format!("{{\"ansi\": [\n{}\n\"#12345\"]}}", (0..15).map(|_| "\"#000000\",").collect::<Vec<_>>().join("\n"));
    assert_eq!(validate(&ansi, "a.json").unwrap_err(), "Theme a.json: line 17: expected a color");
    assert_eq!(validate(r##"{"ansi": ["#000000"]}"##, "a.json").unwrap_err(), "Theme a.json: line 1: expected 16 colors");
    let cases = [
        ("{\n\"size\": \"big\"}", "line 2: expected a size from 6 to 72"),
        (r#"{"font": "x'); background: url(evil"}"#, "line 1: expected a font name"),
        (r#"{"useBold": 1}"#, "line 1: expected true or false"),
        ("{\"light\": {\n\"bg\": \"#fff\"}}", "line 2: expected a color"),
        (r#"{"light": {"font": "Menlo"}}"#, "line 1: unknown key light.font"),
        (r#"{"light": 3}"#, "line 1: expected an object or null"),
        ("{\n\"bg\": \"#000000\",\n}", "line 3: trailing comma"),
        ("[]", "expected an object"),
    ];
    for (text, want) in cases {
        assert_eq!(validate(text, "t.json").unwrap_err(), format!("Theme t.json: {want}"), "{text}");
    }
    let (_, w) = validate("{\n\"bg\": \"#000000\",\n\"glow\": 1}", "t.json").unwrap();
    assert_eq!(w, vec!["Theme t.json: line 3: unknown key glow".to_string()]);
}

#[test]
fn a_broken_file_keeps_its_last_good_version_and_is_told_once() {
    let d = dir("lastgood");
    fs::write(d.join("work.json"), WORK).unwrap();
    let (mut themes, m) = Themes::load(&d);
    assert!(m.is_empty());
    fs::write(d.join("work.json"), "{\n\"bg\": \"#00001\"}").unwrap();
    assert_eq!(themes.reload(), vec!["Theme work.json: line 2: expected a color".to_string()]);
    assert_eq!(themes.get("work").unwrap()["bg"], "#000010", "the last good version");
    assert!(themes.reload().is_empty(), "told once");
    fs::write(d.join("work.json"), "{\"bg\": ").unwrap();
    assert_eq!(themes.reload(), vec!["Theme work.json: line 1: the file ends too early".to_string()]);
    fs::write(d.join("work.json"), r##"{"bg": "#010101"}"##).unwrap();
    assert!(themes.reload().is_empty());
    assert_eq!(themes.get("work").unwrap()["bg"], "#010101");
    fs::write(d.join("work.json"), "nope").unwrap();
    assert_eq!(themes.reload().len(), 1, "told again after it was good");
    fs::remove_file(d.join("work.json")).unwrap();
    assert!(themes.reload().is_empty());
    assert!(themes.get("work").is_none(), "a removed file is gone");
    fs::write(d.join("new.json"), "{oops").unwrap();
    assert_eq!(themes.reload(), vec!["Theme new.json: line 1: expected a value".to_string()]);
    assert!(themes.get("new").is_none(), "never good: no theme");
}

#[test]
fn a_missing_theme_uses_the_default_and_says_so() {
    let d = dir("missing");
    ensure_defaults(&d).unwrap();
    fs::write(d.join("work.json"), WORK).unwrap();
    let (themes, _) = Themes::load(&d);
    let (s, _) = parse(r#"{"themes": {"default": "work", "sessions": {"notes": "paper"}}}"#);
    let r = themes.resolve(&s, None, "notes", true);
    assert_eq!(r.name, "work");
    assert_eq!(themes.name_of(&s, None, "notes"), "work", "a missing theme: the default");
    assert_eq!(r.message.as_deref(), Some("Theme paper (the session notes rule) does not exist: using work"));
    let (s, _) = parse(r#"{"themes": {"default": "gone"}}"#);
    let r = themes.resolve(&s, None, "x", true);
    assert_eq!((r.name.as_str(), r.message.as_deref()), ("local", Some("Theme gone (the default rule) does not exist: using local")));
    assert_eq!(themes.name_of(&s, None, "x"), "local");
    let empty = dir("empty");
    let (themes, _) = Themes::load(&empty.join("absent"));
    let r = themes.resolve(&Settings::default(), Some("h"), "x", true);
    assert_eq!((r.name.as_str(), r.message), ("remote", None), "shipped themes stand in for missing files");
}

#[test]
fn big_and_odd_files_are_refused_or_skipped() {
    let d = dir("odd");
    fs::write(d.join("big.json"), format!("{{\"x\": \"{}\"}}", "a".repeat(MAX_BYTES))).unwrap();
    fs::write(d.join(".hidden.json"), WORK).unwrap();
    fs::write(d.join("notes.txt"), "x").unwrap();
    fs::create_dir(d.join("dir.json")).unwrap();
    let (themes, m) = Themes::load(&d);
    assert_eq!(m, vec!["Theme big.json: larger than 64 KB, not read".to_string()]);
    assert!(themes.names().is_empty());
    assert_eq!(variant(&json!({"bg": "#000000", "light": null}), "x", false)["dark"], true);
}

#[test]
fn system_dark_answers_without_failing() {
    let _ = system_dark();
}
