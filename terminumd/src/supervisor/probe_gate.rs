// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-260: read the closed fields of the local check gate record.

use std::{collections::BTreeMap, fs::OpenOptions, io::{Read, Seek, SeekFrom}, os::unix::fs::OpenOptionsExt, path::Path};

use serde::{Deserialize, Serialize};

use super::{probe_command::Failure, probe_git::common_dir};

const LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Deserialize)]
struct GateLine {
    kind: String,
    at: String,
    at_unix: Option<i64>,
    by: Option<String>,
    result: Option<String>,
    #[serde(default)]
    steps: BTreeMap<String, Step>,
}

#[derive(Deserialize)]
struct Step {
    #[serde(default)]
    retried: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Gate {
    pub since: i64,
    pub nightly_since: Option<i64>,
    pub events: Vec<GateEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct GateEvent {
    pub at: i64,
    pub retried: usize,
    pub nightly_red: bool,
}

fn legacy_time(text: &str) -> Option<i64> {
    let [date, time] = text.split(' ').collect::<Vec<_>>().try_into().ok()?;
    let [year, month, day] = date.split('-').map(str::parse::<i32>).collect::<Result<Vec<_>, _>>().ok()?.try_into().ok()?;
    let [hours, minutes] = time.split(':').map(str::parse::<i32>).collect::<Result<Vec<_>, _>>().ok()?.try_into().ok()?;
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day)
        || !(0..24).contains(&hours) || !(0..60).contains(&minutes) {
        return None;
    }
    // SAFETY: initialize a civil timestamp in the local timezone used by the legacy writer.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = year - 1900;
    tm.tm_mon = month - 1;
    tm.tm_mday = day;
    tm.tm_hour = hours;
    tm.tm_min = minutes;
    tm.tm_isdst = -1;
    let at = unsafe { libc::mktime(&mut tm) };
    (at > 0).then_some(at)
}

fn read(path: &Path, through: i64) -> Result<Gate, Failure> {
    let mut file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path).map_err(|err| if err.kind() == std::io::ErrorKind::NotFound { Failure::Unavailable } else { Failure::Invalid })?;
    let attrs = file.metadata().map_err(|_| Failure::Invalid)?;
    if !attrs.is_file() { return Err(Failure::Invalid); }
    let offset = attrs.len().saturating_sub(LIMIT);
    file.seek(SeekFrom::Start(offset)).map_err(|_| Failure::Invalid)?;
    let mut bytes = Vec::new();
    file.take(LIMIT).read_to_end(&mut bytes).map_err(|_| Failure::Invalid)?;
    // Ignore the clipped first record and a writer's incomplete final line.
    let mut lines = bytes.split_inclusive(|b| *b == b'\n').skip(usize::from(offset > 0));
    let mut since = None;
    let mut nightly_since = None;
    let mut events = Vec::new();
    for line in lines.by_ref().filter(|line| line.ends_with(b"\n")) {
        let row: GateLine = serde_json::from_slice(line).map_err(|_| Failure::Invalid)?;
        if row.kind != "gate" { continue; }
        let at = row.at_unix.or_else(|| legacy_time(&row.at)).filter(|t| *t > 0).ok_or(Failure::Invalid)?;
        if at > through { continue; }
        since = Some(since.map_or(at, |old: i64| old.min(at)));
        let nightly = row.by.as_deref() == Some("rehearse");
        if nightly { nightly_since = Some(nightly_since.map_or(at, |old: i64| old.min(at))); }
        let nightly_red = nightly && row.result.as_deref() == Some("fail");
        let retried = row.steps.into_values().map(|step| step.retried.len()).sum();
        if nightly_red || retried > 0 {
            events.push(GateEvent { at, retried, nightly_red });
        }
    }
    let since = since.ok_or(Failure::Unavailable)?;
    Ok(Gate { since, nightly_since, events })
}

pub(super) async fn collect(dir: &Path, at: i64) -> Result<Gate, Failure> {
    let path = common_dir(dir).await?.join("terminum-gate.jsonl");
    tokio::task::spawn_blocking(move || read(&path, at)).await.map_err(|_| Failure::Invalid)?
}
