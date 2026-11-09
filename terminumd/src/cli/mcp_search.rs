// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! search_tools' index (terminum-agents.md AC-155): BM25 over each tool's name, keywords and
//! text, where every word is first stemmed and then taken to its synonym group, so "close the
//! ticket" finds finish_task and "reprioritize an issue" finds update_task. Built once a
//! process, in memory: the catalog is small, the search a few microseconds.

use std::collections::{HashMap, HashSet};

/// Words that say nothing of an action.
const STOP: [&str; 44] = [
    "a", "an", "the", "to", "of", "for", "on", "in", "into", "at", "by", "with", "from", "and", "or", "is", "are", "be",
    "it", "its", "this", "that", "these", "i", "me", "we", "you", "your", "our", "can", "how", "do", "what", "which",
    "tool", "tools", "want", "need", "some", "one", "up", "should", "please", "will",
];

/// Synonym groups: each word of a group counts as its first. Verbs say the action, nouns its object.
pub const SYNONYMS: [&[&str]; 17] = [
    &["create", "add", "new", "make", "register", "submit", "draft", "spawn", "introduce"],
    &["list", "search", "find", "browse", "all", "enumerate", "query", "filter", "look", "overview", "board", "backlog"],
    &["read", "get", "view", "fetch", "show", "see", "display", "inspect", "details", "retrieve", "load", "describe"],
    &["update", "change", "edit", "modify", "set", "alter", "adjust", "rename", "retitle", "rewrite", "revise", "groom", "amend", "reword", "correct"],
    &["priority", "severity", "urgency", "importance", "urgent", "critical", "prioritize", "reprioritize", "deprioritize", "rank", "bump", "raise", "lower", "higher", "p0", "p1", "p2", "p3"],
    &["parent", "subtask", "child", "children", "epic", "group", "regroup", "nest", "hierarchy", "umbrella", "under"],
    &["move", "transfer", "column", "state", "block", "wait", "stuck", "handoff", "hand", "fail", "reopen"],
    &["human", "person", "user", "owner", "maintainer", "people", "escalate"],
    &["take", "claim", "assign", "pick", "grab", "start", "begin", "accept", "adopt", "own"],
    &["finish", "done", "complete", "close", "resolve", "result", "wrap", "deliver", "conclude", "ship", "end"],
    &["archive", "remove", "delete", "hide", "drop", "clear", "clean", "cleanup", "retire", "discard", "trash"],
    &["comment", "note", "reply", "remark", "annotate", "discuss", "mention", "message", "feedback"],
    &["attach", "upload", "attachment", "document", "screenshot", "artifact", "image", "path"],
    &["task", "ticket", "issue", "card", "todo", "item", "job"],
    &["plan", "progress", "step", "eta", "checklist", "milestone", "estimate", "doing"],
    &["session", "current"],
    &["text", "description", "body", "title", "words", "content"],
];

/// A word's stem: lowercase, plural and -ing/-ed cut, a final e and a doubled consonant
/// dropped, the same for a query and a document, so "moving", "moved" and "moves" meet "move".
fn stem(word: &str) -> String {
    let mut w = word.to_lowercase();
    let cut = |w: &mut String, suffix: &str, keep: usize| {
        if w.len() > suffix.len() + keep && w.ends_with(suffix) {
            w.truncate(w.len() - suffix.len());
            true
        } else {
            false
        }
    };
    if w.ends_with("ies") && w.len() > 4 {
        w.truncate(w.len() - 3);
        w.push('y');
    } else if ["ses", "xes", "zes", "ches", "shes"].iter().any(|s| w.ends_with(s)) {
        cut(&mut w, "es", 2);
    } else if !(w.ends_with("ss") || w.ends_with("us") || w.ends_with("is")) && !cut(&mut w, "ing", 2) && !cut(&mut w, "ed", 2) {
        cut(&mut w, "s", 2);
    }
    if w.len() > 3 && w.ends_with('e') {
        w.pop();
    }
    let b = w.as_bytes();
    if b.len() > 2 && b[b.len() - 1] == b[b.len() - 2] && !b"aeiou".contains(&b[b.len() - 1]) && b[b.len() - 1].is_ascii_alphabetic() {
        w.pop();
    }
    w
}

/// Each stem's group: the group's first word, stemmed.
fn groups() -> HashMap<String, String> {
    let mut m = HashMap::new();
    for g in SYNONYMS {
        let head = stem(g[0]);
        for w in g {
            m.entry(stem(w)).or_insert_with(|| head.clone());
        }
    }
    m
}

/// The index terms of `text`: words split at anything not a letter or digit (`_` too), stop
/// words left out, stemmed, taken to their group.
fn terms(text: &str, groups: &HashMap<String, String>) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !STOP.contains(&w.to_lowercase().as_str()))
        .map(|w| { let s = stem(w); groups.get(&s).cloned().unwrap_or(s) })
        .collect()
}

/// One tool to index: its name, then its fields each with a weight.
pub struct Doc<'a> {
    pub name: &'a str,
    pub fields: Vec<(&'a str, f64)>,
}

/// The index of a catalog.
pub struct Index {
    groups: HashMap<String, String>,
    /// per tool: its name, its terms' weighted counts, its weighted length
    docs: Vec<(String, HashMap<String, f64>, f64)>,
    /// per term: in how many tools
    df: HashMap<String, usize>,
    avg_len: f64,
}

const K1: f64 = 1.2;
const B: f64 = 0.75;
/// A hit is kept only within this part of the best one's score: the rest is noise.
const KEEP: f64 = 0.2;

impl Index {
    pub fn new(catalog: &[Doc]) -> Index {
        let groups = groups();
        let mut docs = Vec::new();
        let mut df: HashMap<String, usize> = HashMap::new();
        for d in catalog {
            let mut tf: HashMap<String, f64> = HashMap::new();
            let mut len = 0.0;
            for (text, weight) in std::iter::once((d.name, 3.0)).chain(d.fields.iter().copied()) {
                for t in terms(text, &groups) {
                    *tf.entry(t).or_default() += weight;
                    len += weight;
                }
            }
            for t in tf.keys() {
                *df.entry(t.clone()).or_default() += 1;
            }
            docs.push((d.name.to_string(), tf, len));
        }
        let avg_len = docs.iter().map(|d| d.2).sum::<f64>() / docs.len().max(1) as f64;
        Index { groups, docs, df, avg_len }
    }

    /// The tools that match `query`, best first, at most `limit`: a tool's exact name in the
    /// query comes first; then BM25 over the query's distinct terms.
    pub fn search(&self, query: &str, limit: usize) -> Vec<String> {
        let named: Vec<&String> = self.docs.iter().map(|d| &d.0).filter(|n| query.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == n.as_str())).collect();
        let wanted: HashSet<String> = terms(query, &self.groups).into_iter().collect();
        let n = self.docs.len() as f64;
        let mut scored: Vec<(f64, &String)> = self.docs.iter().map(|(name, tf, len)| {
            let s = wanted.iter().filter_map(|t| tf.get(t).map(|f| (t, f))).map(|(t, f)| {
                let df = *self.df.get(t).unwrap_or(&0) as f64;
                let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
                idf * f * (K1 + 1.0) / (f + K1 * (1.0 - B + B * len / self.avg_len))
            }).sum::<f64>();
            (s, name)
        }).filter(|(s, _)| *s > 0.0).collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
        let best = scored.first().map_or(0.0, |s| s.0);
        let mut out: Vec<String> = named.into_iter().cloned().collect();
        for (s, name) in scored {
            if s >= best * KEEP && !out.contains(name) {
                out.push(name.clone());
            }
        }
        out.truncate(limit);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac155_stems_meet_across_forms() {
        for (a, b) in [("moving", "move"), ("moved", "moves"), ("tasks", "task"), ("priorities", "priority"), ("planned", "plan"),
                       ("archived", "archive"), ("added", "add"), ("statuses", "status"), ("boxes", "box"), ("comments", "commenting")] {
            assert_eq!(stem(a), stem(b), "{a} {b}");
        }
        assert_ne!(stem("status"), stem("stat"));
    }

    #[test]
    fn ac155_no_word_sits_in_two_groups() {
        let mut seen = HashMap::new();
        for g in SYNONYMS {
            for w in g {
                if let Some(other) = seen.insert(stem(w), g[0]) {
                    panic!("{w} is in {other}'s group and {}'s", g[0]);
                }
            }
        }
    }

    #[test]
    fn ac155_an_exact_name_comes_first_and_nothing_matches_nothing() {
        let idx = Index::new(&[Doc { name: "get_task", fields: vec![("Read a task", 1.0)] }, Doc { name: "create_task", fields: vec![("Make a task", 1.0)] }]);
        assert_eq!(idx.search("create_task please", 5)[0], "create_task");
        assert!(idx.search("the of a", 5).is_empty());
        assert!(idx.search("", 5).is_empty());
        assert_eq!(idx.search("open a ticket", 1).len(), 1);
    }
}
