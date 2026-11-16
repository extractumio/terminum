// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A task's attributes (terminum-kanban.md AC-199).

use super::*;
use crate::tasks::ops::{self, Edit, NewTask};
use crate::tasks::store::{Store, HUMAN};

fn fresh(name: &str) -> Store {
    let s = Store::open(crate::testutil::fresh_dir(name).join("kanban.sqlite"));
    s.with(|tx| { tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('api', 'api', 1, '', 0), ('web', 'web', 2, '', 1)", []).map_err(ops::sql)?; crate::tasks::slugs::fill(tx) }).unwrap();
    s
}

fn made(s: &Store, title: &str, project: &str) -> ops::Task {
    s.with(|tx| ops::create(tx, HUMAN, NewTask { title: title.into(), project: Some(project.into()), ..Default::default() }, 1)).unwrap()
}

fn edit(s: &Store, id: &str, change: Edit) -> Result<ops::Task, String> {
    s.with(|tx| ops::edit(tx, HUMAN, id, change, 2).and_then(|()| ops::get(tx, id)))
}

fn said(s: &Store, id: &str) -> Vec<String> {
    s.with(|tx| ops::history(tx, id)).unwrap().as_array().unwrap().iter().map(|e| e["text"].as_str().unwrap_or_default().to_string()).collect()
}

#[test]
fn ac199_a_title_s_type_word_is_its_type_and_it_is_shown_from_it() {
    let s = fresh("attrs-type");
    let t = made(&s, "bug:  the list loses its scroll", "api");
    assert_eq!((t.kind.as_deref(), t.name.as_str(), t.title.as_str()), (Some("Bug"), "the list loses its scroll", "Bug: the list loses its scroll"));
    let plain = made(&s, "Note: words stay", "api");
    assert_eq!((plain.kind, plain.title.as_str()), (None, "Note: words stay"), "a word that is no type stays in the title");
    // a title without a word keeps the type; one with a word sets it; Type alone sets it; empty takes it off
    let t = edit(&s, &t.id, Edit { title: Some("the list keeps its scroll".into()), ..Default::default() }).unwrap();
    assert_eq!(t.title, "Bug: the list keeps its scroll");
    let t = edit(&s, &t.id, Edit { title: Some("Feature: keep the scroll".into()), ..Default::default() }).unwrap();
    assert_eq!((t.kind.as_deref(), t.title.as_str()), (Some("Feature"), "Feat: keep the scroll"));
    let t = edit(&s, &t.id, Edit { kind: Some("story".into()), ..Default::default() }).unwrap();
    assert_eq!(t.title, "Story: keep the scroll");
    let t = edit(&s, &t.id, Edit { kind: Some(String::new()), ..Default::default() }).unwrap();
    assert_eq!((t.kind.as_deref(), t.title.as_str()), (None, "keep the scroll"));
    assert!(said(&s, &t.id).iter().any(|l| l.ends_with("set its type to Story")) && said(&s, &t.id).iter().any(|l| l.ends_with("took its type off")));
    // what reading the title and writing it back does: nothing
    let before = t.clone();
    let t = edit(&s, &t.id, Edit { title: Some(t.title.clone()), ..Default::default() }).unwrap();
    assert_eq!((t.title, t.kind), (before.title, before.kind));
}

#[test]
fn ac199_a_type_that_disagrees_with_the_title_or_is_unknown_is_refused() {
    let s = fresh("attrs-type-no");
    let t = made(&s, "one", "api");
    assert_eq!(edit(&s, &t.id, Edit { title: Some("Bug: two".into()), kind: Some("Debt".into()), ..Default::default() }).unwrap_err(), "The title says Bug: and Type says Debt: give one.");
    assert_eq!(edit(&s, &t.id, Edit { kind: Some("Epic".into()), ..Default::default() }).unwrap_err(), "Type: one of Feature, Bug, Debt, Story");
    assert_eq!(edit(&s, &t.id, Edit { title: Some("Debt:   ".into()), ..Default::default() }).unwrap_err(), "Title: 1–200 characters");
    let long = format!("Feat: {}", "x".repeat(ops::TITLE_MOST));
    assert_eq!(edit(&s, &t.id, Edit { title: Some(long), ..Default::default() }).unwrap().name.len(), ops::TITLE_MOST, "the limit is the title's, the word aside");
}

#[test]
fn ac199_an_external_link_is_a_web_address_and_a_target_any_short_words() {
    let s = fresh("attrs-link");
    let t = made(&s, "one", "api");
    let set = |link: &str, target: &str| edit(&s, &t.id, Edit { attrs: Attrs { external_link: Some(link.into()), target: Some(target.into()), ..Default::default() }, ..Default::default() });
    let t2 = set(" https://tracker.example/browse/API-7 ", "v0.32.0").unwrap();
    assert_eq!((t2.external_link.as_deref(), t2.target.as_deref()), (Some("https://tracker.example/browse/API-7"), Some("v0.32.0")));
    for bad in ["javascript:alert(1)", "tracker.example/API-7", "https://a b", "https://"] {
        assert!(set(bad, "").unwrap_err().starts_with("External link:"), "{bad}");
    }
    assert_eq!(set("", &"v".repeat(TARGET_MOST + 1)).unwrap_err(), format!("Target: at most {TARGET_MOST} characters"));
    let t3 = set("", "").unwrap();
    assert_eq!((t3.external_link, t3.target), (None, None), "empty takes them off");
    let lines = said(&s, &t.id);
    for l in ["set its external link", "set its target to v0.32.0", "took its external link off", "took its target off"] {
        assert!(lines.iter().any(|x| x.ends_with(l)), "{l}: {lines:?}");
    }
}

#[test]
fn ac199_blocked_by_and_follow_ups_hold_tasks_of_the_project_with_their_reasons() {
    let s = fresh("attrs-lists");
    let (a, b, c, far) = (made(&s, "a", "api"), made(&s, "b", "api"), made(&s, "c", "api"), made(&s, "far", "web"));
    let link = |of: &ops::Task, list: &str, other: &str, reason: Option<&str>, remove: bool| s.with(|tx| super::link(tx, HUMAN, &of.key, &Link { list, task: other, reason, remove }, 3));
    link(&a, "blocked_by", &b.id, Some("needs b's column"), false).unwrap();
    link(&a, "blocked_by", &c.id, None, false).unwrap();
    link(&a, "follow_ups", &c.id, Some("then c"), false).unwrap();
    let lists = s.with(|tx| lists_of(tx, &a.key)).unwrap();
    assert_eq!(lists["blocked_by"][0], serde_json::json!({"id": b.id, "title": "b", "status": "TODO", "reason": "needs b's column"}));
    assert_eq!(lists["blocked_by"][1]["id"], c.id.as_str());
    assert_eq!(lists["follow_ups"][0]["reason"], "then c");
    link(&a, "blocked_by", &b.id, Some("b first"), false).unwrap();
    link(&a, "blocked_by", &c.id, None, true).unwrap();
    let lists = s.with(|tx| lists_of(tx, &a.key)).unwrap();
    assert_eq!((lists["blocked_by"].as_array().unwrap().len(), lists["blocked_by"][0]["reason"].as_str()), (1, Some("b first")));
    let lines = said(&s, &a.id);
    for l in [format!("added {} to Blocked by", b.id), format!("changed why {} is in Blocked by", b.id), format!("took {} out of Blocked by", c.id), format!("added {} to Follow-ups", c.id)] {
        assert!(lines.iter().any(|x| x.ends_with(&l)), "{l}: {lines:?}");
    }
    // refused: itself, another project's, a ring, one not there, an unknown list
    assert_eq!(link(&a, "blocked_by", &a.id, None, false).unwrap_err(), "Blocked by: a task cannot name itself");
    assert_eq!(link(&a, "follow_ups", &far.id, None, false).unwrap_err(), format!("Follow-ups: {} is in another project", far.id));
    assert_eq!(link(&b, "blocked_by", &a.id, None, false).unwrap_err(), format!("{} waits for this task already: a task cannot wait for one that waits for it", a.id));
    link(&c, "blocked_by", &a.id, None, false).unwrap();
    assert!(link(&b, "blocked_by", &c.id, None, false).is_err(), "a longer ring too: c waits for a, a for b");
    assert_eq!(link(&a, "blocked_by", &c.id, None, true).unwrap_err(), format!("{} is not in Blocked by.", c.id));
    assert_eq!(link(&a, "parents", &c.id, None, false).unwrap_err(), "The lists are blocked_by and follow_ups.");
    let many: Vec<(String, String)> = (0..=LINKS_MOST).map(|n| (made(&s, &format!("n{n}"), "api").id, String::new())).collect();
    assert_eq!(s.with(|tx| set_list(tx, HUMAN, &b.key, "follow_ups", &many, 4)).unwrap_err(), format!("Follow-ups: at most {LINKS_MOST} tasks"));
}

#[test]
fn ac199_a_task_moved_to_another_project_leaves_the_lists_across_projects() {
    let s = fresh("attrs-move");
    let (a, b) = (made(&s, "a", "api"), made(&s, "b", "api"));
    s.with(|tx| super::link(tx, HUMAN, &a.key, &Link { list: "blocked_by", task: &b.id, reason: Some("why"), remove: false }, 3)).unwrap();
    s.with(|tx| ops::change_project(tx, HUMAN, &b.id, "web", None, 4)).unwrap();
    assert_eq!(s.with(|tx| lists_of(tx, &a.key)).unwrap()["blocked_by"], serde_json::json!([]));
    assert!(said(&s, &a.id).iter().any(|l| l.contains("to another project: it left Blocked by")));
}

#[test]
fn ac199_each_type_s_word_reads_back_as_that_type() {
    for (name, word) in TYPES {
        assert_eq!(split_title(&shown(Some(name), "x")), (Some(name), "x"));
        assert_eq!(check_type(word), Ok(Some(name)));
    }
}
