// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Who waited for whom (AC-195): the tasks' store's calls, tmux's answers and the async workers'
//! ticks that came late, each said in at most one line a second, the rest counted in the next.

use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// A store's call that waits for its turn, or holds its transaction, this long is said.
pub const STORE_SLOW: Duration = Duration::from_millis(100);
/// A tmux command answered this long after it was queued is said.
pub const TMUX_SLOW: Duration = Duration::from_millis(500);
/// The async workers' tick, and how late it may come before it is said.
const TICK: Duration = Duration::from_millis(100);
const LAG: Duration = Duration::from_millis(200);

/// One line a second; what came between is counted.
#[derive(Default)]
pub struct Throttle(Mutex<(Option<Instant>, u64)>);

impl Throttle {
    /// Whether a line may be written now, with how many were not since the last.
    pub fn pass(&self) -> Option<u64> {
        let mut s = self.0.lock();
        let now = Instant::now();
        if s.0.is_some_and(|last| now.duration_since(last) < Duration::from_secs(1)) {
            s.1 += 1;
            return None;
        }
        s.0 = Some(now);
        Some(std::mem::take(&mut s.1))
    }

    /// The next line may be written at once.
    #[cfg(test)]
    pub fn reset(&self) {
        self.0.lock().0 = None;
    }
}

pub fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

/// How late a tick due at `due` came at `now`, when that is worth saying.
fn late(due: Instant, now: Instant) -> Option<Duration> {
    now.checked_duration_since(due).filter(|d| *d >= LAG)
}

/// A tick every 100 ms on the async workers: one that comes 200 ms late or more says so.
pub async fn watch_runtime() {
    let throttle = Throttle::default();
    let mut due = Instant::now() + TICK;
    loop {
        tokio::time::sleep_until(due.into()).await;
        let now = Instant::now();
        if let Some(d) = late(due, now)
            && let Some(more) = throttle.pass() {
                tracing::warn!(event = "runtime.lag", ms = ms(d), more);
            }
        due = now + TICK;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-195: one line a second; the lines not written are counted in the next.
    #[test]
    fn ac195_one_line_a_second_the_rest_counted() {
        let t = Throttle::default();
        assert_eq!(t.pass(), Some(0));
        assert_eq!(t.pass(), None);
        assert_eq!(t.pass(), None);
        t.0.lock().0 = Some(Instant::now() - Duration::from_secs(2));
        assert_eq!(t.pass(), Some(2));
        assert_eq!(t.pass(), None);
    }

    /// AC-195: a tick is late from 200 ms on, never early.
    #[test]
    fn ac195_a_tick_is_late_from_200_ms() {
        let due = Instant::now();
        assert_eq!(late(due, due + Duration::from_millis(199)), None);
        assert_eq!(late(due, due + Duration::from_millis(250)), Some(Duration::from_millis(250)));
        assert_eq!(late(due + Duration::from_secs(1), due), None);
    }
}
