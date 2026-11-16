// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Tracing's formatted records go to one bounded blocking writer (AC-196).

use std::cell::Cell;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, mpsc};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::{Log, Policy, EVERY};

#[derive(Clone)]
pub struct Writer(Arc<Inner>);
struct Inner {
    send: Option<mpsc::SyncSender<Vec<u8>>>,
    pending: Arc<AtomicUsize>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.send.take(); // disconnect only after the last record's writer has gone
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}
static WRITER: OnceLock<Writer> = OnceLock::new();
thread_local! { static IN_WORKER: Cell<bool> = const { Cell::new(false) }; }
const FLUSH_LIMIT: Duration = Duration::from_secs(2);

impl Writer {
    pub fn start(path: PathBuf, policy: Policy, home: Option<PathBuf>) -> io::Result<Self> {
        let mut log = Log::open(path, policy)?;
        let (send, recv) = mpsc::sync_channel::<Vec<u8>>(256);
        let pending = Arc::new(AtomicUsize::new(0));
        let completed = pending.clone();
        let worker = std::thread::Builder::new().name("terminum-logs".into()).spawn(move || {
            IN_WORKER.set(true);
            let mut checked = Instant::now() - EVERY;
            let mut sweep = home.map(super::Sweeper::new);
            let mut complained = None;
            loop {
                let result = match recv.recv_timeout(EVERY.saturating_sub(checked.elapsed())) {
                    Ok(bytes) => {
                        let result = log.append(&bytes);
                        completed.fetch_sub(1, Ordering::Release);
                        result
                    }
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
        Ok(Self(Arc::new(Inner { send: Some(send), pending, worker: Some(worker) })))
    }

    /// No thread::current or channel parking here: atexit may run after Rust destroys TLS.
    pub fn flush(&self) {
        if IN_WORKER.try_with(Cell::get).unwrap_or(false) { return }
        let deadline = Instant::now() + FLUSH_LIMIT;
        while self.0.pending.load(Ordering::Acquire) > 0 && Instant::now() < deadline {
            // SAFETY: usleep has no pointer arguments; unlike Rust's channel wait it needs no TLS.
            unsafe { libc::usleep(5000); }
        }
    }

    fn record(&self, bytes: Vec<u8>) {
        self.0.pending.fetch_add(1, Ordering::Relaxed);
        if self.0.send.as_ref().expect("an open writer").send(bytes).is_err() { self.0.pending.fetch_sub(1, Ordering::Release); }
    }

    fn panic_record(&self, bytes: Vec<u8>) {
        self.0.pending.fetch_add(1, Ordering::Relaxed);
        if self.0.send.as_ref().expect("an open writer").try_send(bytes).is_err() { self.0.pending.fetch_sub(1, Ordering::Release); }
    }
}

pub fn install(writer: Writer) {
    if WRITER.set(writer).is_err() { return }
    extern "C" fn at_exit() { flush(); }
    // SAFETY: the callback has C ABI, no captures, and remains valid for the process lifetime.
    unsafe { libc::atexit(at_exit); }
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !IN_WORKER.try_with(Cell::get).unwrap_or(false) && let Some(w) = WRITER.get() {
            w.panic_record(format!("{info}\n").into_bytes());
            w.flush();
        }
        original(info);
    }));
}

pub fn flush() { if let Some(w) = WRITER.get() { w.flush(); } }

pub struct Record { writer: Writer, bytes: Vec<u8> }
impl Write for Record {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl Drop for Record {
    fn drop(&mut self) {
        if !self.bytes.is_empty() { self.writer.record(std::mem::take(&mut self.bytes)); }
    }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Writer {
    type Writer = Record;
    fn make_writer(&'a self) -> Record { Record { writer: self.clone(), bytes: Vec::new() } }
}
