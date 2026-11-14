// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Bounded file logs (AC-196): shared stream rotation and completed-file retention.

mod completed;
mod reader;
mod rotation;
pub mod writer;
pub use completed::{Sweeper, snapshot};
pub use reader::{Mark, follow, mark, recent};
pub use rotation::{AGENT, DAEMON, EVERY, INTEGRATION, Log, Policy, adopt_legacy, segment};
use rotation::{TURN, lock_dir, remove};

#[cfg(test)]
mod tests;
