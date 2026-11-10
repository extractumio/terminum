// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A host's helper gate (AC-83, AC-99): everything that copies the helper to a host or removes it
//! runs in that host's turn, one at a time, in the order asked: Files' enable and removal
//! (consent.rs), agents on and off, their setup by itself and their refresh (agents.rs), the
//! helper's update (tunnel.rs). So no copy meets a removal on the host, and each judges who else
//! uses the helper on a record no other of them is changing. Files' operations and the update are
//! never stopped; a later agents on or off stops the agents' operation running.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::Notify;

/// Said by an agents on or off that a later one for the same host ended.
pub(super) const SUPERSEDED: &str = "a later agents on or off for this host took over; this one was stopped";

#[derive(Default)]
pub struct Turns(Mutex<HashMap<String, Arc<Turn>>>);

#[derive(Default)]
struct Turn {
    running: tokio::sync::Mutex<()>,
    asked: AtomicU64,
    stop: Notify,
}

/// How an operation takes its turn.
#[derive(Clone, Copy, PartialEq)]
enum Way {
    /// stops the agents' operation running or waiting, and is stopped by a later one
    TakeOver,
    /// leaves the one running be; a later [`Way::TakeOver`] stops it
    After,
    /// leaves the one running be and is never stopped
    Hold,
}

impl Turns {
    /// An agents on or off, `work`, in `host`'s turn: the last asked wins. It stops the agents'
    /// operation running (dropped mid-step, its ssh killed) and goes after it, so an off given
    /// during a long on is never written over by that on's late save, nor its removal raced by
    /// that on's install.
    pub(super) async fn run<T>(&self, host: &str, work: impl Future<Output = Result<T, String>>) -> Result<T, String> {
        self.turn(host, Way::TakeOver, work).await
    }

    /// `work` in `host`'s turn after the one running, which it leaves be (the agents' setup by
    /// itself, their refresh); a later [`Turns::run`] stops it all the same.
    pub(super) async fn after<T>(&self, host: &str, work: impl Future<Output = Result<T, String>>) -> Result<T, String> {
        self.turn(host, Way::After, work).await
    }

    /// `work` in `host`'s turn after the one running, and never stopped by a later one, which
    /// waits for it: Files' enable and removal, the helper's update.
    pub(super) async fn hold<T>(&self, host: &str, work: impl Future<Output = Result<T, String>>) -> Result<T, String> {
        self.turn(host, Way::Hold, work).await
    }

    /// Whether an operation of `host` runs or waits now.
    pub(super) fn busy(&self, host: &str) -> bool {
        self.0.lock().get(host).is_some_and(|t| t.running.try_lock().is_err())
    }

    async fn turn<T>(&self, host: &str, way: Way, work: impl Future<Output = Result<T, String>>) -> Result<T, String> {
        let turn = self.0.lock().entry(host.to_string()).or_default().clone();
        let mine = if way == Way::TakeOver {
            let mine = turn.asked.fetch_add(1, Ordering::SeqCst) + 1;
            turn.stop.notify_waiters();
            mine
        } else {
            turn.asked.load(Ordering::SeqCst)
        };
        let _running = turn.running.lock().await;
        if way == Way::Hold {
            return work.await;
        }
        let stopped = async {
            loop {
                let woke = turn.stop.notified();
                tokio::pin!(woke);
                woke.as_mut().enable();
                if turn.asked.load(Ordering::SeqCst) != mine {
                    return;
                }
                woke.await;
            }
        };
        tokio::select! {
            biased;
            _ = stopped => Err(SUPERSEDED.into()),
            r = work => r,
        }
    }
}

#[cfg(test)]
#[path = "turns_tests.rs"]
mod tests;
