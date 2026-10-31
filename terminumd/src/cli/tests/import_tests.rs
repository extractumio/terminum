// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! terminum.md §9 Compatibility, terminum-remote.md §9: `terminum import` takes iterm-enhancer's
//! workspaces, bookmarks and hosts, keeps tmux panes only and never replaces terminum's own.

use super::*;
use crate::cli::importer;

fn web_key(host: &str, key: &str) -> String {
    format!("web:{}", json!([host, key]))
}

struct T {
    home: Home,
    ctx: Ctx,
    src: PathBuf,
    settings: PathBuf,
}

impl T {
    fn new() -> T {
        let home = Home::new();
        let ctx = home.ctx(&[]);
        let src = home.path(".iterm-enhancer/state");
        std::fs::create_dir_all(&src).unwrap();
        let settings = ctx.root.join("settings.json");
        T { home, ctx, src, settings }
    }
    fn put(&self, name: &str, data: Value) {
        std::fs::write(self.src.join(name), data.to_string()).unwrap();
    }
    fn run(&self) -> Result<Vec<String>, String> {
        importer::run(&self.home.dir, &self.ctx.app_dir, &self.settings)
    }
    fn read(&self, path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }
}

#[test]
fn everything_of_tmux_panes_and_hosts() {
    let t = T::new();
    t.put("agents.json", json!({"alex@devbox.example": {"ssh": ["-p", "2222", "alex@devbox.example"], "platform": "linux-x86_64"},
                                "old-alias": {"host": "vm1"}}));
    let mut panes = serde_json::Map::new();
    panes.insert(web_key("", "tmux:mac:default:%3"), json!({"root": "/Users/alex/work"}));
    panes.insert(web_key("", "tmux-remote:-p 2222 alex@devbox.example:default:%7"), json!({"root": "/home/alex", "tabs": [{"path": "/home/alex/notes.md"}]}));
    panes.insert(web_key("", "w0t0p0:6A1B"), json!({"root": "/Users/alex/iterm"}));
    panes.insert("tmux:mac:default:%3".into(), json!({"root": "/Users/alex/panel"}));
    panes.insert("web:not json".into(), json!({}));
    t.put("workspaces.json", json!({"version": 1, "prefs": {"theme": "dark"}, "panes": panes}));
    t.put("web-bookmarks.json", json!({"version": 1, "items": [
        {"id": "1", "sid": "s1", "key": "tmux:mac:default:%3", "n": 10},
        {"id": "2", "sid": "s2", "key": null, "n": 4},
        {"id": "3", "sid": "s3", "key": "tmux-remote:alex@devbox.example:default:%7", "n": 2}]}));
    let said = t.run().unwrap();
    let hosts = t.read(&t.settings)["hosts"].clone();
    assert_eq!(hosts["devbox"], json!({"ssh": ["-p", "2222", "alex@devbox.example"], "files": "enabled"}));
    assert_eq!(hosts["old-alias"], json!({"ssh": ["old-alias"], "files": "enabled"}), "an alias record is its own destination");
    let ws = t.read(&t.ctx.app_dir.join("workspaces.json"));
    assert_eq!(ws["prefs"], json!({"theme": "dark"}));
    let mut keys: Vec<String> = ws["panes"].as_object().unwrap().keys().cloned().collect();
    keys.sort();
    let remote_key = web_key("devbox", "tmux:devbox:default:%7");
    let mut want = vec![web_key("", "tmux:mac:default:%3"), remote_key.clone()];
    want.sort();
    assert_eq!(keys, want);
    assert_eq!(ws["panes"][&remote_key], json!({"root": "/home/alex", "tabs": [{"path": "/home/alex/notes.md"}]}), "the panel's actual host namespace retains its tree and tabs");
    let marks = t.read(&t.ctx.app_dir.join("web-bookmarks.json"))["items"].clone();
    let got: Vec<(String, String)> = marks.as_array().unwrap().iter().map(|m| (m["id"].as_str().unwrap().into(), m["key"].as_str().unwrap().into())).collect();
    assert_eq!(got, [("1".to_string(), "tmux:mac:default:%3".to_string()), ("3".into(), "tmux:devbox:default:%7".into())]);
    assert_eq!(mode(&t.ctx.app_dir.join("workspaces.json")), 0o600);
    assert!(said.contains(&"workspaces.json: 2 entries of tmux panes imported".to_string()), "{said:?}");
}

#[test]
fn terminums_own_files_and_hosts_are_kept() {
    let t = T::new();
    std::fs::create_dir_all(&t.ctx.app_dir).unwrap();
    std::fs::write(&t.settings, json!({"port": 47901, "hosts": {"devbox": {"ssh": ["devbox.example"]}, "work": {"ssh": ["alex@devbox.example"]}}}).to_string()).unwrap();
    std::fs::write(t.ctx.app_dir.join("workspaces.json"), r#"{"mine": true}"#).unwrap();
    t.put("workspaces.json", json!({"panes": {}}));
    t.put("agents.json", json!({"a": {"ssh": ["alex@devbox.example"]}, "b": {"ssh": ["alex@devbox.example.org"]}}));
    let said = t.run().unwrap();
    let s = t.read(&t.settings);
    assert_eq!(s["port"], 47901);
    let mut names: Vec<&String> = s["hosts"].as_object().unwrap().keys().collect();
    names.sort();
    assert_eq!(names, ["devbox", "devbox-2", "work"], "same recipe kept, a taken name numbered");
    assert_eq!(std::fs::read_to_string(t.ctx.app_dir.join("workspaces.json")).unwrap(), r#"{"mine": true}"#);
    assert!(said.contains(&"workspaces.json: kept terminum's own (it exists already)".to_string()));
}

#[test]
fn a_second_import_changes_nothing() {
    let t = T::new();
    t.put("agents.json", json!({"a": {"ssh": ["alex@devbox.example"]}}));
    t.run().unwrap();
    let before = std::fs::read_to_string(&t.settings).unwrap();
    assert!(!t.run().unwrap().join(" ").contains("hosts:"));
    assert_eq!(std::fs::read_to_string(&t.settings).unwrap(), before);
}

#[test]
fn broken_files_are_refused_with_their_name() {
    let t = T::new();
    std::fs::write(t.src.join("agents.json"), "{not json").unwrap();
    let err = t.run().unwrap_err();
    assert!(err.contains("agents.json: not JSON"), "{err}");
    assert!(!t.settings.exists());
}

#[test]
fn existing_non_object_settings_are_refused_without_overwriting_them() {
    for text in ["[]\n", "\"salvageable settings\"\n", "null\n", "42\n"] {
        let t = T::new();
        t.put("agents.json", json!({"a": {"ssh": ["alex@devbox.example"]}}));
        std::fs::create_dir_all(t.settings.parent().unwrap()).unwrap();
        std::fs::write(&t.settings, text).unwrap();
        let error = t.run().unwrap_err();
        assert!(error.contains("settings.json") && error.contains("object"), "{error}");
        assert_eq!(std::fs::read_to_string(&t.settings).unwrap(), text);
        assert!(!t.ctx.app_dir.join("workspaces.json").exists());
    }
}

#[test]
fn imported_remote_workspace_replaces_a_legacy_outer_host_hint() {
    let t = T::new();
    t.put("agents.json", json!({"old-destination": {"ssh": ["alex@devbox.example"]}}));
    let pane = json!({"root": "/home/alex/work", "tabs": [{"path": "/home/alex/work/notes.md"}], "expanded": ["/home/alex/work/src"]});
    let old_key = web_key("old-destination", "tmux-remote:old-destination:default:%8");
    t.put("workspaces.json", json!({"version": 1, "panes": {old_key: pane}}));
    t.run().unwrap();
    let data = t.read(&t.ctx.app_dir.join("workspaces.json"));
    assert_eq!(data["panes"][web_key("devbox", "tmux:devbox:default:%8")], pane);
    assert_eq!(data["panes"].as_object().unwrap().len(), 1);
}

#[test]
fn nothing_to_import() {
    let t = T::new();
    std::fs::remove_dir_all(&t.src).unwrap();
    assert!(t.run().unwrap()[0].contains("Nothing to import"));
}

#[test]
fn import_stops_and_starts_a_running_terminumd() {
    let t = T::new();
    let ctx = t.home.ctx(&[("TM_NO_LAUNCHD", "1"), ("TM_NO_OPEN", "1")]);
    crate::cli::install::install(&ctx, &package(&t.home, "a", None, false)).unwrap();
    t.put("agents.json", json!({"a": {"ssh": ["alex@devbox.example"]}}));
    let before = crate::cli::launch::health(&ctx).unwrap()["pid"].clone();
    crate::cli::commands::import_state(&ctx).unwrap();
    let after = crate::cli::launch::health(&ctx).unwrap();
    assert_eq!(after["build"], "a");
    assert_ne!(after["pid"], before, "stopped while the files were written");
    assert!(said(&ctx).iter().any(|l| l.starts_with("hosts: devbox")));
    crate::cli::direct::stop(&ctx);
}
