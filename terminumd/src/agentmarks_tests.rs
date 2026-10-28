// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use serde_json::json;

use super::*;

const K: &str = "tmux:local:terminum:%1";

fn here() -> Place {
    Place { key: K.into(), title: "api".into(), dir: Some("/Users/alex/p".into()), ..Place::default() }
}

fn rec(title: &str, remind: bool) -> Record {
    Record { title: title.into(), text: format!("{title}: what, where, why, next step"), remind, kind: Some("report".into()), file: None }
}

fn ids(told: &[Told]) -> Vec<String> {
    told.iter().map(|t| match t { Told::Mark(i) => i.id.clone(), Told::Unmark(id) => id.clone() }).collect()
}

#[test]
fn records_are_the_agents_bookmarks_and_reminders_of_its_pane() {
    let mut s = Store::memory();
    let (said, told) = add(&mut s, &here(), "claude", vec![rec("Final report: retry fixed", false), rec("Decide the API name", true)]).unwrap();
    let [a, b] = [&s.all()[0], &s.all()[1]];
    assert_eq!((a.by.as_deref(), a.note.as_str(), a.remind, a.n), (Some("claude"), "Final report: retry fixed", false, -1));
    assert_eq!((a.key.as_deref(), a.title.as_str(), a.dir.as_deref(), a.kind.as_deref()), (Some(K), "api", Some("/Users/alex/p"), Some("report")));
    assert!(b.remind && b.resolved.is_none());
    assert_eq!(ids(&told), [a.id.clone(), b.id.clone()]);
    assert!(said.contains(&format!("bookmark {} \"Final report: retry fixed\"", a.id)), "{said}");
    assert!(said.contains(&format!("reminder {} \"Decide the API name\" ({COMES_UP})", b.id)), "{said}");
    let (_, _) = add(&mut s, &here(), "gemini", vec![rec("Other", false)]).unwrap();
    assert_eq!(s.all()[2].by.as_deref(), Some("agent"), "an agent terminum does not know by name");
}

#[test]
fn a_repeat_is_not_kept_twice_and_its_text_is_cleaned() {
    let mut s = Store::memory();
    add(&mut s, &here(), "claude", vec![rec("Next step: paging", false)]).unwrap();
    let (said, told) = add(&mut s, &here(), "claude", vec![rec("next STEP: paging", false), rec("Next step: paging", false)]).unwrap();
    assert!(told.is_empty() && s.all().len() == 1 && said.contains("already kept"), "{said}");
    let mut r = rec("Key \u{202E}left", false);
    r.text = format!("token {}-api03-abcdefghijklmnopqrstuvwxyz0123456789 \u{7} here", ["sk", "ant"].join("-"));   // a key's shape, built: no key in the source
    add(&mut s, &here(), "claude", vec![r]).unwrap();
    let it = &s.all()[1];
    assert_eq!(it.note, "Key left");
    assert!(!it.text.contains("abcdefghijklmnop") && !it.text.contains('\u{7}'), "{}", it.text);
}

#[test]
fn limits_are_the_agents_own() {
    let mut s = Store::memory();
    for i in 0..OPEN_MOST {
        add(&mut s, &here(), "claude", vec![rec(&format!("r{i}"), false)]).unwrap();
    }
    let e = add(&mut s, &here(), "claude", vec![rec("one more", false)]).unwrap_err();
    assert!(e.contains("50 open records"), "{e}");
    let other = Place { key: "tmux:local:terminum:%2".into(), ..Place::default() };
    add(&mut s, &other, "claude", vec![rec("elsewhere", false)]).unwrap();
    // the user's 300 count only the user's
    let user = crate::page::bookmarks::NewMark { key: Some(K.into()), n: 3, text: "x".into(), ..Default::default() };
    for _ in 0..crate::page::bookmarks::MAX_ITEMS {
        s.add(user.clone()).unwrap();
    }
    assert!(s.add(user).is_err(), "the user's limit stands");
    assert!(add(&mut s, &other, "claude", vec![rec("still", false)]).is_ok(), "an agent's record past the user's 300");
    let e = add(&mut s, &here(), "claude", vec![rec("", false)]).unwrap_err();
    assert!(e.contains("title is empty"), "{e}");
}

#[test]
fn update_resolves_deletes_and_edits_only_an_agents_record_of_the_scope() {
    let mut s = Store::memory();
    add(&mut s, &here(), "codex", vec![rec("Remind me", true), rec("A finding", false)]).unwrap();
    let (r, b) = (s.all()[0].id.clone(), s.all()[1].id.clone());
    let (said, told) = update(&mut s, &here(), &json!({"id": r, "status": "done"})).unwrap();
    assert!(said.ends_with("updated (resolved).") && s.get(&r).unwrap().resolved.is_some() && ids(&told) == [r.clone()], "{said}");
    update(&mut s, &here(), &json!({"id": r, "status": "open", "title": "Remind me again"})).unwrap();
    assert_eq!((s.get(&r).unwrap().resolved, s.get(&r).unwrap().note.as_str()), (None, "Remind me again"));
    let (said, told) = update(&mut s, &here(), &json!({"id": b, "status": "done"})).unwrap();
    assert!(s.get(&b).is_none() && told == [Told::Unmark(b.clone())], "{said}");
    let other = Place { key: "tmux:local:terminum:%2".into(), ..Place::default() };
    assert!(update(&mut s, &other, &json!({"id": r, "status": "done"})).unwrap_err().contains("No record"));
    let user = s.add(crate::page::bookmarks::NewMark { key: Some(K.into()), n: 3, text: "mine".into(), ..Default::default() }).unwrap();
    assert!(update(&mut s, &here(), &json!({"id": user.id, "status": "done"})).unwrap_err().contains("No record"), "the user's own are not the agent's");
    assert!(update(&mut s, &here(), &json!({"id": r})).unwrap_err().contains("nothing to change"));
    assert!(update(&mut s, &here(), &json!({"id": r, "status": "x"})).unwrap_err().contains("not one of open, done"));
    // a reminder made a bookmark and done at once: deleted, never a resolved bookmark
    let (_, told) = update(&mut s, &here(), &json!({"id": r, "remind": false, "status": "done"})).unwrap();
    assert!(s.get(&r).is_none() && told == [Told::Unmark(r.clone())]);

    // the pane's own made before it joined a project stay the agent's there
    let mut s = Store::memory();
    add(&mut s, &here(), "claude", vec![rec("Before the project", false)]).unwrap();
    let id = s.all()[0].id.clone();
    let joined = Place { project: Some("p1".into()), ..here() };
    assert!(update(&mut s, &joined, &json!({"id": id, "title": "Still mine"})).is_ok());
}

#[test]
fn the_notes_of_0_23_become_agents_bookmarks_once() {
    let dir = std::env::temp_dir().join(format!("tm-notes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.json"), json!({"version": 1, "next": 3, "notes": [
        {"id": 1, "kind": "bug", "text": "The parser drops a BOM\nin src/parse.rs", "file": "src/parse.rs", "status": "open", "at": 1, "key": K, "session": "s", "project": null},
        {"id": 2, "kind": "deferred", "text": "done one", "status": "done", "at": 1, "key": K, "session": "s"}]}).to_string()).unwrap();
    let mut s = Store::memory();
    import_notes(&mut s, &dir);
    assert_eq!(s.all().len(), 1);
    let it = &s.all()[0];
    assert_eq!((it.note.as_str(), it.text.as_str(), it.by.as_deref(), it.kind.as_deref(), it.file.as_deref()),
               ("The parser drops a BOM", "The parser drops a BOM\nin src/parse.rs", Some("agent"), Some("bug"), Some("src/parse.rs")));
    assert!(!dir.join("notes.json").exists() && dir.join("notes.json.imported").exists());
    import_notes(&mut s, &dir);
    assert_eq!(s.all().len(), 1, "once");
    // set aside but not renamed (a failed rename): imported again, nothing kept twice
    std::fs::rename(dir.join("notes.json.imported"), dir.join("notes.json")).unwrap();
    import_notes(&mut s, &dir);
    assert_eq!(s.all().len(), 1, "a note already kept is not kept again");
    let _ = std::fs::remove_dir_all(&dir);
}
