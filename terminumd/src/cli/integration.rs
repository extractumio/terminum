// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! `terminum integration queue|wait|status` (`terminum-integration.md` AC-187): the integration queue's tools from
//! a shell, called as `terminum tool` calls them: by the pane the command runs in, else a token's holder.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::tool::{ask, caller, Caller};
use super::Ctx;

pub const USAGE: &str = "usage: terminum integration queue <ID> --result-file <path> [--commit <sha>] [--wait] | wait <ID> | status [<ID>]";
/// How often a waiting command asks.
const BEAT: Duration = Duration::from_secs(5);
/// How long a waiting command bears terminumd gone: a project's After may restart it.
const DOWN_MOST: Duration = Duration::from_secs(10 * 60);

pub fn run(ctx: &Ctx, rest: &[String]) -> Result<i32, String> {
    match rest {
        [q, id, more @ ..] if q == "queue" => queue(ctx, id, more),
        [w, id] if w == "wait" => wait(ctx, &caller(ctx)?, id),
        [s] if s == "status" => status(ctx, None),
        [s, id] if s == "status" => status(ctx, Some(id)),
        _ => Err(USAGE.into()),
    }
}

/// `queue <ID> --result-file <path> [--commit <sha>] [--wait]`: integrate's answer; with --wait, wait's end.
fn queue(ctx: &Ctx, id: &str, more: &[String]) -> Result<i32, String> {
    let (mut commit, mut file, mut waits) = (None, None, false);
    let mut it = more.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--commit" => commit = Some(it.next().ok_or(USAGE)?.clone()),
            "--result-file" => file = Some(it.next().ok_or(USAGE)?.clone()),
            "--wait" => waits = true,
            _ => return Err(USAGE.into()),
        }
    }
    let path = file.ok_or("--result-file <path>: the result finish_task would take, in its sections.")?;
    let result = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    let who = caller(ctx)?;
    let mut args = json!({"id": id, "result": result});
    if let Some(c) = commit {
        args["commit"] = json!(c);
    }
    ctx.say(ask(ctx, &who, "integrate", &args)?);
    if waits { wait(ctx, &who, id) } else { Ok(0) }
}

/// `status [<ID>]`: integration_status's answer as it gives it.
fn status(ctx: &Ctx, id: Option<&str>) -> Result<i32, String> {
    let args = id.map_or_else(|| json!({}), |i| json!({"id": i}));
    ctx.say(ask(ctx, &caller(ctx)?, "integration_status", &args)?);
    Ok(0)
}

/// What a waiting command exits with, by how its task left the queue (AC-187); none while it stands there.
fn exit_of(left: &Value) -> Result<Option<i32>, String> {
    Ok(match left.as_str() {
        None => None,
        Some("landed") => Some(0),
        Some("red") => Some(1),
        Some("conflict") => Some(2),
        Some("out") => Some(3),
        Some(other) => return Err(format!("terminum says the task left as {other:?}: not known to this command.")),
    })
}

/// `wait <ID>`: each of Dispatcher's notices on the task printed as it comes, until the task leaves INTEGRATION;
/// what was said before the wait is not news, but the words its task left with are.
fn wait(ctx: &Ctx, who: &Caller, id: &str) -> Result<i32, String> {
    let mut seen: Option<HashSet<String>> = None;
    let mut down_since: Option<Instant> = None;
    loop {
        let text = match ask(ctx, who, "integration_status", &json!({"id": id})) {
            Err(why) if why == super::mcp::DOWN => {
                if down_since.get_or_insert_with(Instant::now).elapsed() > DOWN_MOST {
                    return Err(format!("terminum did not come back for {} minutes: {id} may still be queued.", DOWN_MOST.as_secs() / 60));
                }
                std::thread::sleep(BEAT);
                continue;
            }
            other => other?,
        };
        down_since = None;
        let mut task = serde_json::from_str::<Value>(&text).map_err(|e| format!("terminum's answer: {e}"))?["task"].take();
        let said = task["said"].as_array_mut().map(std::mem::take).unwrap_or_default();
        let ended = exit_of(&task["left"])?;
        let text_of = |e: &Value| e["text"].as_str().unwrap_or_default().to_string();
        match &mut seen {
            None => {
                seen = Some(said.iter().filter_map(|e| e["id"].as_str().map(str::to_string)).collect());
                if ended.is_some() && let Some(e) = said.last() {
                    ctx.say(text_of(e));
                }
            }
            Some(known) => said.iter().filter(|e| e["id"].as_str().is_some_and(|i| known.insert(i.to_string()))).for_each(|e| ctx.say(text_of(e))),
        }
        if let Some(code) = ended {
            return Ok(code);
        }
        std::thread::sleep(BEAT);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::cli::{parse, Cmd};

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn ac187_the_command_parses_and_says_its_usage() {
        assert_eq!(parse(&args(&["integration", "wait", "API-12"])), Ok(Cmd::Integration(args(&["wait", "API-12"]))));
        let home = crate::testutil::fresh_dir("integration-cli");
        let ctx = Ctx::new(HashMap::from([("HOME".to_string(), home.display().to_string())]), home.clone()).unwrap();
        for bad in [&[][..], &["queue"], &["wait"], &["wait", "A-1", "B-2"], &["status", "A-1", "B-2"], &["queue", "A-1", "--commit"], &["queue", "A-1", "--fast"]] {
            assert_eq!(run(&ctx, &args(bad)).unwrap_err(), USAGE, "{bad:?}");
        }
        assert!(run(&ctx, &args(&["queue", "A-1"])).unwrap_err().starts_with("--result-file <path>"));
        // outside a pane, with no token, nothing is called
        assert!(run(&ctx, &args(&["status"])).unwrap_err().contains("TERMINUM_TOKEN"));
    }

    #[test]
    fn ac187_a_wait_ends_by_how_its_task_left() {
        assert_eq!([json!("landed"), json!("red"), json!("conflict"), json!("out")].map(|l| exit_of(&l)), [Ok(Some(0)), Ok(Some(1)), Ok(Some(2)), Ok(Some(3))]);
        assert_eq!(exit_of(&Value::Null), Ok(None), "still queued");
        assert!(exit_of(&json!("lost")).unwrap_err().contains("not known"), "a word it does not know is said, never read as taken out");
    }
}
