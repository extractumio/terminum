// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Tasks and the board (`docs/specs/terminum-kanban.md`).

pub mod agents;
pub mod api;
pub mod archiving;
pub mod attrs;
pub mod auto;
pub mod closeout;
mod columns;
pub mod comments;
pub mod cron;
pub mod family;
mod fileapi;
pub mod files;
pub mod folder;
pub mod integration;
pub mod integration_queue;
mod groom;
pub mod limits;
pub mod marks;
mod moves;
pub mod ops;
pub mod projectapi;
pub mod projects;
pub mod query;
pub mod schedules;
pub mod schedulesapi;
pub mod slugs;
pub mod notify;
pub mod store;
pub mod tell;
pub mod template;
pub mod tools;
pub mod users;
mod waitfor;
mod windowapi;
