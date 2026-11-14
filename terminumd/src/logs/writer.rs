// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Tracing's formatted records go to one bounded blocking writer (AC-196).

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};

use super::{Log, Policy, EVERY};

enum Message {
    Record(Vec<u8>),
    Flush(mpsc::Sender<()>),
}

#[derive(Clone)]
pub struct Writer(mpsc::SyncSender<Message>);
static WRITER: OnceLock<Writer> = OnceLock::new();
const FLUSH_LIMIT: Duration = Duration::from_secs(2);

impl Writer {
    pub fn start(path: PathBuf, policy: Policy, home: Option<PathBuf>) -> io::Result<Self> {
        let mut log = Log::open(path, policy)?;
        let (send, recv) = mpsc::sync_channel(256);
        std::thread::Builder::new().name("terminum-logs".into()).spawn(move || {
            let mut checked = Instant::now() - EVERY;
            let mut sweep = home.map(super::Sweeper::new);
            let mut complained = None;
            loop {
                let result = match recv.recv_timeout(EVERY.saturating_sub(checked.elapsed())) {
                    Ok(Message::Record(bytes)) => log.append(&bytes),
                    Ok(Message::Flush(done)) => { let _ = done.send(()); Ok(()) }
                    Err(mpsc::RecvTimeoutError::Timeout) => log.maintain(),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                if let Err(e) = result && complained.is_none_or(|at: Instant| at.elapsed() >= EVERY) {
                    eprintln!("terminumd: log write/rotation failed: {e}");
                    complained = Some(Instant::now());
                }
                if checked.elapsed() >= EVERY {
                    checked = Instant::now();
                    if let Some(sweep) = &mut sweep && let Err(e) = sweep.tick() {
                        eprintln!("terminumd: integration log retention failed: {e}");
                    }
                }
            }
        })?;
        Ok(Self(send))
    }

    /// A barrier after all preceding records; no unbounded exit wait for a blocked disk.
    pub fn flush(&self) {
        if std::thread::current().name() == Some("terminum-logs") { return }
        let deadline = Instant::now() + FLUSH_LIMIT;
        let (done, wait) = mpsc::channel();
        let mut message = Message::Flush(done);
        loop {
            match self.0.try_send(message) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(m)) if Instant::now() < deadline => {
                    message = m;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return,
            }
        }
        let _ = wait.recv_timeout(deadline.saturating_duration_since(Instant::now()));
    }
}

pub fn install(writer: Writer) {
    if WRITER.set(writer).is_err() { return }
    extern "C" fn at_exit() { flush(); }
    // SAFETY: the callback has C ABI, no captures, and remains valid for the process lifetime.
    unsafe { libc::atexit(at_exit); }
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // A failing worker cannot log to itself or wait for itself to drain.
        if std::thread::current().name() != Some("terminum-logs") && let Some(w) = WRITER.get() {
                let _ = w.0.try_send(Message::Record(format!("{info}\n").into_bytes()));
                w.flush();
        }
        original(info);
    }));
}

pub fn flush() {
    if let Some(w) = WRITER.get() { w.flush(); }
}

pub struct Record {
    writer: Writer,
    bytes: Vec<u8>,
}

impl Write for Record {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

impl Drop for Record {
    fn drop(&mut self) {
        if !self.bytes.is_empty() {
            let _ = self.writer.0.send(Message::Record(std::mem::take(&mut self.bytes)));
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Writer {
    type Writer = Record;
    fn make_writer(&'a self) -> Record { Record { writer: self.clone(), bytes: Vec::new() } }
}
