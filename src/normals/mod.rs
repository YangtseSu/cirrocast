// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Climate normals: one best-effort comparison on top of a weather run.
//!
//! A normal is not a weather backend reading. It answers one extra question — what is usual for
//! this calendar month at the place the run already resolved — and it never participates in the
//! provider chain, the day-count cache keys or the fallback rules: a failed normals fetch degrades
//! to a note without touching the forecast's exit code. The
//! [`Report`](crate::model::Report)'s `normals` field travels with the report so every renderer can
//! shape its output from the data alone, and the model type lives in [`crate::model::normals`],
//! because `src/render` reads the model and never this module (the step-12 layering rule).
//!
//! One source today, NOAA NCEI's Global Summary of the Month (see [`ncei`]); the dispatch in
//! [`fetch`] is where a second source would join.

pub mod ncei;

use crate::error::Result;
use crate::model::{Location, Normals};
use crate::provider::Env;

/// Fetches the normal for `month` (`1`–`12`) at `loc` from the run's source.
///
/// `Ok(None)` is a first-class answer: no station inside the configured radius, a record too thin
/// to be a normal, or a month the station's rows do not cover. The reason travels on the `-v`
/// stream; the caller owns the failure policy for a real [`Err`].
pub fn fetch(loc: &Location, month: u8, env: &Env<'_>) -> Result<Option<Normals>> {
    ncei::normals(loc, month, env)
}
