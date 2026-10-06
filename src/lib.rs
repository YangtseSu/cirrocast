// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `cirrocast` — a terminal weather client with pluggable backends.
//!
//! This crate is the library half of the binary: the command line surface ([`cli`]), the error and
//! exit-code contract ([`error`]), XDG directory resolution ([`paths`]), the backend registry and
//! the provider trait ([`provider`]), the canonical data model ([`model`]), location resolution
//! ([`geo`]), the `%`-token engine ([`template`]) and the render layer ([`render`]).
//!
//! It also owns the two mechanics a multi-location run is built from, because they are policy
//! rather than plumbing: [`fetch_reports`] fetches one report per location in argument order, and
//! [`worst_exit_code`] turns the per-location outcomes into the process exit code.

pub mod air;
pub mod alerts;
pub mod astro;
pub mod cache;
pub mod cli;
pub mod config;
pub mod error;
pub mod geo;
pub mod http;
pub mod i18n;
pub mod model;
pub mod normals;
pub mod parallel;
pub mod paths;
pub mod provider;
pub mod render;
pub mod status;
pub mod template;

use crate::error::{Error, Result};
use crate::model::Report;

/// How many locations are fetched concurrently: four, or the machine's parallelism when smaller.
///
/// The cap is a policy, not a tuning knob: `keyless` providers pay for concurrency in upstream
/// load, and the common case (2–4 places, one per city) is covered. A single location never
/// spends a thread.
#[must_use]
pub fn worker_count(locations: usize) -> usize {
    let parallelism = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    locations.min(parallelism.min(4)).max(1)
}

/// Fetches one report per item, in the input's order, with [`worker_count`] workers.
///
/// `fetch` receives the item's index and the item; it does all the per-location work (resolution,
/// provider chain, alerts, panels) and returns the finished report. A failure in one slot does not
/// stop the others: the caller renders the placeholder for it and keeps the rest.
pub fn fetch_reports<T>(
    items: &[T],
    fetch: impl Fn(usize, &T) -> Result<Report> + Sync,
) -> Vec<Result<Report>>
where
    T: Sync,
{
    let workers = worker_count(items.len());
    parallel::par_map_ordered(items, workers, fetch)
}

/// The process exit code of a multi-location run: `0`, or the numerically largest mapped code among
/// the failures.
///
/// "Largest mapped code wins" means a missing key (6) outranks a location miss (5), which is the
/// more actionable problem. The alternative — the first failure in argument order — would make the
/// code depend on the order the user typed, which is not a property of the failure.
#[must_use]
pub fn worst_exit_code(results: &[Result<Report>]) -> u8 {
    results
        .iter()
        .filter_map(|result| result.as_ref().err().map(Error::exit_code))
        .max()
        .unwrap_or(0)
}
