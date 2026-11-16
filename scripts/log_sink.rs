// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-196: the checks' output uses the daemon's rotation core, without a Cargo build.

#[allow(dead_code)]
#[path = "../terminumd/src/logs/rotation.rs"]
mod rotation;
#[path = "../terminumd/src/fslock.rs"]
mod fslock;

use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

fn run(path: PathBuf) -> io::Result<()> {
    let mut log = rotation::Log::open(path, rotation::CHECKS)?;
    let (send, recv) = mpsc::sync_channel(4);
    // Fixed chunks and a bounded channel even for a command printing one enormous line.
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        let mut bytes = [0; 8192];
        loop {
            match input.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => { if send.send(Ok(bytes[..n].to_vec())).is_err() { break } }
                Err(e) => { let _ = send.send(Err(e)); break }
            }
        }
    });
    let mut checked = Instant::now();
    loop {
        match recv.recv_timeout(rotation::EVERY.saturating_sub(checked.elapsed())) {
            Ok(bytes) => log.append(&bytes?)?,
            Err(mpsc::RecvTimeoutError::Timeout) => { log.maintain()?; checked = Instant::now(); }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn main() {
    let Some(path) = std::env::args_os().nth(1) else { eprintln!("usage: log-sink <file>"); std::process::exit(2) };
    if let Err(e) = run(PathBuf::from(path)) {
        eprintln!("check log failed: {e}");
        std::process::exit(1);
    }
}
