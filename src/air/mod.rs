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
//! Two sources since step 29:
//!
//! * [`open_meteo`] — keyless, global, the floor every install has;
//! * [`qweather`] — the provider-bound one (host + credential from the `qweather` backend), the
//!   only place the Chinese national AQI (`cn-mee`) exists, and therefore what a reader in
//!   mainland China actually wants from `--aqi`.
//!
//! [`AirChoice`] is the selection: `auto` (the default) prefers `QWeather` exactly when the point is
//! in mainland China, the `qweather` provider is on the run's chain *and* a credential resolves —
//! the same "coverage selects the national service, the keyless one is the floor" rule the alert
//! registry uses — and an explicit choice is honoured as given, with its own errors (a missing host
//! or credential) surfacing rather than falling back. `auto` never downgrades a run whose
//! credential is missing, and an explicit `qweather` never silently becomes Open-Meteo.

pub mod aqi;
pub mod open_meteo;
pub mod qweather;

use crate::error::Result;
use crate::model::{AlertSource, Location};
use crate::provider::{Env, ProviderId};

pub use crate::model::{AirQuality, AirSource, Pollen};

/// The `[air] source` / `--air-source` selector: `auto`, or one source by name.
///
/// `auto` is *not* an [`AirSource`]: it is the absence of a choice, which the run resolves from the
/// location, the provider chain and the key store. Naming one source skips that rule entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AirChoice {
    /// Resolve the source from the location, the chain and the credentials.
    #[default]
    Auto,
    /// Use this source wherever the run happens to be.
    Source(AirSource),
}

impl AirChoice {
    /// Every value, in `--help` and `[air] source` order.
    pub const ALL: [&'static str; 3] = ["auto", "open-meteo", "qweather"];

    /// Parses a selector name, case-insensitively.
    ///
    /// The flag parser and the configuration validator share this, so `--air-source` and
    /// `[air] source` cannot disagree about a spelling.
    pub fn from_name(name: &str) -> Result<Self> {
        let name = name.trim();
        if name.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        AirSource::from_name(name).map(Self::Source)
    }
}

impl std::fmt::Display for AirChoice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => formatter.write_str("auto"),
            Self::Source(source) => formatter.write_str(source.as_str()),
        }
    }
}

/// Resolves the source `choice` names for this run.
///
/// `auto` prefers the [`AirSource::QWeather`] reading when all three of its conditions hold — the
/// point is in mainland China, the `qweather` provider is on `chain`, and the key store resolves a
/// credential for it — and answers [`AirSource::OpenMeteo`] otherwise, so a default install keeps
/// answering `--aqi` without a key. A credential the store cannot even read (a malformed
/// `keys.toml`) is a `--verbose` note and the keyless floor, never a failed panel: the air reading
/// is best-effort by contract.
///
/// An explicit choice is returned as given: its source's own errors (the missing host naming the
/// console, the missing credential naming `key set`) surface from [`fetch`], which is what "never
/// a silent fallback" means here.
#[must_use]
pub fn source_for(
    loc: &Location,
    chain: &[ProviderId],
    env: &Env<'_>,
    choice: AirChoice,
) -> AirSource {
    match choice {
        AirChoice::Source(source) => {
            note(env, &format!("air: source `{source}` (explicit)"));
            source
        }
        AirChoice::Auto => {
            let source = auto_source(loc, chain, env);
            note(
                env,
                &format!(
                    "air: source `{source}` (auto{}); `--air-source` overrides",
                    match source {
                        AirSource::QWeather => ": mainland China, qweather on the chain",
                        AirSource::OpenMeteo => ": the keyless source",
                    }
                ),
            );
            source
        }
    }
}

/// [`source_for`]'s `auto` rule.
fn auto_source(loc: &Location, chain: &[ProviderId], env: &Env<'_>) -> AirSource {
    if !in_mainland_china(loc) || !chain.contains(&ProviderId::QWeather) {
        return AirSource::OpenMeteo;
    }
    match env.keys.credential(ProviderId::QWeather) {
        Ok(Some(_)) => AirSource::QWeather,
        Ok(None) => AirSource::OpenMeteo,
        Err(error) => {
            note(
                env,
                &format!("air: the key store could not be read ({error})"),
            );
            AirSource::OpenMeteo
        }
    }
}

/// One `--verbose` line, suppressed by `--quiet`.
fn note(env: &Env<'_>, message: &str) {
    if env.verbose > 0 && !env.quiet {
        eprintln!("{message}");
    }
}

/// Whether `loc` is in mainland China, by the same country code and bounding boxes the alert
/// registry draws for its `qweather` source.
///
/// The rule is shared rather than re-invented: the Chinese national AQI exists for the same
/// territory the warnings do, and a second set of rectangles would be a second answer to "is this
/// point Chinese".
#[must_use]
fn in_mainland_china(loc: &Location) -> bool {
    AlertSource::QWeather.covers(loc)
}

/// Fetches the air-quality reading for `loc` from `source`.
///
/// The caller owns the failure policy: `--aqi` turns an error into a warning and keeps the weather
/// output, so nothing here catches its own failures.
pub fn fetch(loc: &Location, env: &Env<'_>, source: AirSource) -> Result<AirQuality> {
    match source {
        AirSource::OpenMeteo => open_meteo::fetch(loc, env),
        AirSource::QWeather => qweather::fetch(loc, env),
    }
}

#[cfg(test)]
mod tests {
    use super::{AirChoice, AirSource};

    #[test]
    fn every_selector_spelling_parses_to_its_own_choice() {
        for name in AirChoice::ALL {
            let choice = AirChoice::from_name(name).expect("a documented spelling parses");
            match choice {
                AirChoice::Auto => assert_eq!(name, "auto"),
                AirChoice::Source(source) => assert_eq!(source.as_str(), name),
            }
        }
        assert_eq!(
            AirChoice::from_name(" QWEATHER ").expect("qweather parses"),
            AirChoice::Source(AirSource::QWeather)
        );
        assert_eq!(
            AirChoice::from_name("open-meteo").expect("open-meteo parses"),
            AirChoice::Source(AirSource::OpenMeteo)
        );
        let error = AirChoice::from_name("cams").expect_err("never a source");
        assert!(error.to_string().contains("known sources"), "{error}");
    }
}
