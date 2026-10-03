// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Air quality: one best-effort panel on top of a weather run.
//!
//! The air API is **not** a weather backend. It answers one extra question — what is in the air at
//! the location the run already resolved — and it never participates in the provider chain, the
//! cache's day-count keys or the fallback rules: a failed air fetch degrades to `--verbose` note
//! (or the `air quality unavailable` warning of `--aqi`) without touching the forecast's exit
//! code. The [`Report`](crate::model::Report)'s `air` field travels with the report so every
//! renderer can shape its output from the data alone, and the model types live in
//! [`crate::model::air`], because `src/render` reads the model and never this module (the step-12
//! layering rule).
//!
//! One source today, Open-Meteo's keyless Air Quality API; the [`AirSource`] enum and the
//! dispatch in [`fetch`] are where step 19 adds the next one.

pub mod aqi;
pub mod open_meteo;

use crate::error::Result;
use crate::model::Location;
use crate::provider::Env;

pub use crate::model::{AirQuality, AirSource, Pollen};

/// Fetches the air-quality reading for `loc` from the run's selected source.
///
/// The caller owns the failure policy: `--aqi` turns an error into a warning and keeps the weather
/// output, so nothing here catches its own failures.
pub fn fetch(loc: &Location, env: &Env<'_>) -> Result<AirQuality> {
    fetch_source(AirSource::OpenMeteo, loc, env)
}

/// Fetches `source`; the dispatch point a second source joins.
fn fetch_source(source: AirSource, loc: &Location, env: &Env<'_>) -> Result<AirQuality> {
    match source {
        AirSource::OpenMeteo => open_meteo::fetch(loc, env),
    }
}
