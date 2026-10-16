// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Who may use terminumd (AC-09, AC-10): local sign-ins from one-time codes, the web
//! listener's password and cookie, and the checks every TCP request passes.

pub mod checks;
pub mod crypto;
pub mod signin;
pub mod store;
pub mod web;
pub mod websignin;
