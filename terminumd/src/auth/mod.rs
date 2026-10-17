// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Who may use terminumd (AC-09, AC-10): the password, the sign-ins it gets, and the checks
//! every TCP request passes.

pub mod checks;
pub mod crypto;
pub mod signin;
pub mod store;
pub mod web;
pub mod websignin;
