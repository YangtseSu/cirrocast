// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Open-Meteo's Marine Weather API: waves, swell and sea-surface temperature.
//!
//! This is a **supplementary** source, not a forecast backend: the registry row carries
//! `marine: true`, [`select`](super::select) refuses the id as a chain entry, and `--marine`
//! requests it beside the weather answer. A run that never asked for the panel pays nothing.
//!
//! One request per fetch, keyless and metric. What this module knows that the payload does not
//! spell out:
//!
//! 1. **The answer is for the nearest sea cell, not the requested coordinate.** `cell_selection=sea`
//!    makes the API snap to water, so a coastal point lands within a kilometre or two while an
//!    inland point gets the nearest sea hundreds of kilometres away. The response's own
//!    `latitude`/`longitude` are the sampled cell; the great-circle distance to the requested point
//!    travels with the reading, and a `-v` note names it once it exceeds
//!    [`FAR_CELL_KM`](crate::model::FAR_CELL_KM).
//! 2. **The `current` block is required.** A response without one, or with every reading `null`,
//!    is an upstream error naming the place — never an all-`None` panel. A calm sea still reports
//!    a height, so an absent field means "not measured", which the model spells `None`.
//! 3. **`current.time` is a local wall clock.** `timezone=auto` makes the API answer in the
//!    location's own zone and echo its name; the instant is resolved in that zone (not in a
//!    provisional `UTC` the caller happened to carry) and stored as a fixed offset.
//! 4. **The daily block is a wave summary, not day parts.** The panel is current conditions plus
//!    the daily extremes, so the request asks for neither `hourly` nor `forecast_days`; the cache
//!    key carries a day count of `0` to say so.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use chrono_tz::Tz;
use serde::Deserialize;

use super::{
    Capabilities, Env, FetchRequest, JsonFetch, Provider, ProviderId, attribution, fetch_json,
    local_today,
};
use crate::cache::CacheKey;
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::{Location, Marine, MarineDay, MarineSource, Report, ReportMode, resolve_local};

/// The provider id, as the registry and every error message spell it.
const PROVIDER: &str = "open-meteo-marine";

/// The marine endpoint.
pub const BASE: &str = "https://marine-api.open-meteo.com/v1/marine";

/// Current-condition variables, in the fixed order the request uses.
pub const CURRENT_VARIABLES: &str =
    "wave_height,wave_direction,wave_period,swell_wave_height,sea_surface_temperature";

/// Daily variables, in the fixed order the request uses.
pub const DAILY_VARIABLES: &str = "wave_height_max,wave_period_max,wave_direction_dominant";

/// The Open-Meteo Marine backend. Stateless: one value serves every fetch.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenMeteoMarine;

impl Provider for OpenMeteoMarine {
    fn id(&self) -> ProviderId {
        ProviderId::OpenMeteoMarine
    }

    fn capabilities(&self) -> Capabilities {
        ProviderId::OpenMeteoMarine.metadata().capabilities()
    }

    fn fetch_report(&self, loc: &Location, _req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
        let outcome = fetch_outcome(loc, env)?;
        Ok(Report {
            location: loc.clone(),
            current: None,
            days: Vec::new(),
            alerts: Vec::new(),
            air: None,
            astro: None,
            marine: Some(outcome.marine),
            mode: ReportMode::Forecast,
            attribution: attribution(
                ProviderId::OpenMeteoMarine,
                outcome.url,
                env.cache.clock().now().into(),
                outcome.raw,
            ),
        })
    }
}

/// The reusable entry point for the CLI's `--marine` path: the sea state for `loc`.
///
/// This is the one function the supplementary fetch calls; [`Provider::fetch_report`] wraps the
/// same answer in a [`Report`] for the hand-built-chain path the tests exercise.
pub fn fetch(loc: &Location, env: &Env<'_>) -> Result<Marine> {
    Ok(fetch_outcome(loc, env)?.marine)
}

/// One decoded answer plus the request text and the `-v` raw summary [`Report::attribution`]
/// carries.
struct Outcome {
    marine: Marine,
    url: String,
    raw: Option<String>,
}

/// Fetches and decodes one marine answer, printing the far-cell `-v` note.
fn fetch_outcome(loc: &Location, env: &Env<'_>) -> Result<Outcome> {
    let request = marine_request(loc);
    let key = CacheKey::weather_part(
        PROVIDER,
        "marine",
        loc.lat,
        loc.lon,
        0,
        local_today(env, loc.tz),
    );
    let ttl = Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs));

    let response: MarineResponse = fetch_json(
        env,
        loc,
        &JsonFetch {
            provider: ProviderId::OpenMeteoMarine,
            request: request.clone(),
            key,
            ttl,
            what: "marine",
        },
    )?;

    let marine = decode(&response, loc)?;
    note_far_cell(&marine, env);

    Ok(Outcome {
        marine,
        url: request.redacted_url(),
        raw: (env.verbose > 0).then(|| raw_summary(&response)),
    })
}

/// The request, assembled in a fixed parameter order (the tests assert the URL verbatim).
///
/// No `hourly` and no `forecast_days`: the panel is current conditions plus the daily wave
/// summary, so the API's own default span is what it should be.
fn marine_request(loc: &Location) -> HttpRequest {
    HttpRequest::get(BASE)
        .query("latitude", format!("{:.4}", loc.lat))
        .query("longitude", format!("{:.4}", loc.lon))
        .query("current", CURRENT_VARIABLES)
        .query("daily", DAILY_VARIABLES)
        .query("cell_selection", "sea")
        .query("timezone", "auto")
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// The marine response, in the subset `cirrocast` consumes.
///
/// Unknown fields are ignored on purpose: upstream adds variables and blocks regularly, and a new
/// one never invalidates a cached body.
#[derive(Debug, Clone, Deserialize)]
pub struct MarineResponse {
    /// Latitude of the sea cell upstream answered for.
    pub latitude: f64,
    /// Longitude of the sea cell upstream answered for.
    pub longitude: f64,
    /// IANA zone name the timestamps are expressed in.
    #[serde(default)]
    pub timezone: String,
    /// Current sea state; a response without one is an upstream error.
    #[serde(default)]
    pub current: Option<CurrentBlock>,
    /// Daily wave summary.
    #[serde(default)]
    pub daily: Option<DailyBlock>,
}

/// The `current` object.
#[derive(Debug, Clone, Deserialize)]
pub struct CurrentBlock {
    /// Observation time, local wall clock.
    pub time: String,
    /// Significant wave height in metres.
    #[serde(default)]
    pub wave_height: Option<f64>,
    /// Direction the waves travel *from*, degrees clockwise from north.
    #[serde(default)]
    pub wave_direction: Option<f64>,
    /// Peak wave period in seconds.
    #[serde(default)]
    pub wave_period: Option<f64>,
    /// Swell wave height in metres.
    #[serde(default)]
    pub swell_wave_height: Option<f64>,
    /// Sea surface temperature in °C.
    #[serde(default)]
    pub sea_surface_temperature: Option<f64>,
}

/// The `daily` object: one array per variable, all as long as `time`.
#[derive(Debug, Clone, Deserialize)]
pub struct DailyBlock {
    /// Local calendar dates.
    #[serde(default)]
    pub time: Vec<String>,
    /// Highest significant wave height in metres.
    #[serde(default)]
    pub wave_height_max: Vec<Option<f64>>,
    /// Longest wave period in seconds.
    #[serde(default)]
    pub wave_period_max: Vec<Option<f64>>,
    /// Dominant wave direction, degrees clockwise from north.
    #[serde(default)]
    pub wave_direction_dominant: Vec<Option<f64>>,
}

// ---------------------------------------------------------------------------------------------
// Response → canonical model
// ---------------------------------------------------------------------------------------------

/// Turns one response into a [`Marine`], refusing a response without a usable reading.
fn decode(response: &MarineResponse, loc: &Location) -> Result<Marine> {
    let current = response.current.as_ref().ok_or_else(|| no_reading(loc))?;
    let tz = response_zone(response)?;
    let time = parse_local(&current.time, tz, "current")?;

    let marine = Marine {
        time: time.fixed_offset(),
        wave_height_m: current.wave_height,
        wave_direction_deg: current.wave_direction.map(degrees),
        wave_period_s: current.wave_period,
        swell_wave_height_m: current.swell_wave_height,
        sea_surface_temp_c: current.sea_surface_temperature,
        days: marine_days(response.daily.as_ref())?,
        sampled_lat: response.latitude,
        sampled_lon: response.longitude,
        distance_km: haversine_km(loc.lat, loc.lon, response.latitude, response.longitude),
        source: MarineSource::OpenMeteoMarine,
    };
    if !marine.has_readings() {
        return Err(no_reading(loc));
    }
    Ok(marine)
}

/// The zone the response is expressed in.
///
/// The API echoes the name it resolved `timezone=auto` to; the reading's instant is resolved in
/// that zone, not in a provisional `UTC` a raw-coordinate caller carried in.
fn response_zone(response: &MarineResponse) -> Result<Tz> {
    response
        .timezone
        .parse::<Tz>()
        .map_err(|_| Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!("`{}` is not a known time zone", response.timezone),
        })
}

/// The daily wave summary; empty when the source answered with no daily block.
fn marine_days(daily: Option<&DailyBlock>) -> Result<Vec<MarineDay>> {
    let Some(daily) = daily else {
        return Ok(Vec::new());
    };
    let mut days = Vec::with_capacity(daily.time.len());
    for (index, text) in daily.time.iter().enumerate() {
        days.push(MarineDay {
            date: parse_date(text)?,
            wave_height_max_m: at(&daily.wave_height_max, index),
            wave_period_max_s: at(&daily.wave_period_max, index),
            wave_direction_dominant_deg: at(&daily.wave_direction_dominant, index).map(degrees),
        });
    }
    Ok(days)
}

/// One value of a parallel array; a short array leaves the value absent instead of panicking.
fn at(values: &[Option<f64>], index: usize) -> Option<f64> {
    values.get(index).copied().flatten()
}

/// The upstream error for a response that carries no reading at all, naming the place.
fn no_reading(loc: &Location) -> Error {
    Error::Upstream {
        provider: PROVIDER.to_owned(),
        status: None,
        message: format!(
            "the response carries no marine reading for {} ({:.2}, {:.2})",
            loc.name, loc.lat, loc.lon
        ),
    }
}

/// Parses a local wall clock timestamp (`2026-10-05T23:00`) into the response's zone.
fn parse_local(text: &str, tz: Tz, section: &str) -> Result<DateTime<Tz>> {
    let naive =
        NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M").map_err(|error| Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!(
                "the `{section}` timestamp `{text}` is not a local date and time: {error}"
            ),
        })?;
    resolve_local(tz, naive)
}

/// Parses a `YYYY-MM-DD` date.
fn parse_date(text: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").map_err(|error| Error::Upstream {
        provider: PROVIDER.to_owned(),
        status: None,
        message: format!("`{text}` is not a calendar date: {error}"),
    })
}

/// A direction in degrees, wrapped into `0..360`.
///
/// Rounding first is what makes the wrap correct for `359.7`; the conversion cannot fail because
/// the result of `rem_euclid(360)` is non-negative and below 360.
#[allow(clippy::cast_possible_truncation)]
fn degrees(value: f64) -> u16 {
    let wrapped = (value.round() as i64).rem_euclid(360);
    u16::try_from(wrapped).unwrap_or_default()
}

/// The great-circle distance in kilometres between two points (haversine).
#[must_use]
fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6_371.008_8;
    let (phi1, phi2) = (lat1.to_radians(), lat2.to_radians());
    let delta_phi = (lat2 - lat1).to_radians();
    let delta_lambda = (lon2 - lon1).to_radians();
    let a = (delta_phi / 2.0).sin().powi(2)
        + phi1.cos() * phi2.cos() * (delta_lambda / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
}

// ---------------------------------------------------------------------------------------------
// Notes and the `-v` summary
// ---------------------------------------------------------------------------------------------

/// The `-v` note naming a sampled sea cell that sits more than
/// [`FAR_CELL_KM`](crate::model::FAR_CELL_KM) from the requested point; `None` when the cell is
/// close, or when the run did not ask for notes.
///
/// A note, not a warning: a land point legitimately gets the nearest sea, and the panel (step 23's
/// renderer) is what names it for the user; this line is for `-v` diagnosis.
#[must_use]
pub fn far_cell_note(marine: &Marine, verbose: bool, quiet: bool) -> Option<String> {
    if !verbose || quiet || !marine.sampled_cell_is_far() {
        return None;
    }
    Some(format!(
        "open-meteo-marine sampled the sea cell {:.4}, {:.4}, {:.1} km from the requested point",
        marine.sampled_lat, marine.sampled_lon, marine.distance_km
    ))
}

/// Prints the far-cell note when the run is verbose and not quiet.
fn note_far_cell(marine: &Marine, env: &Env<'_>) {
    if let Some(note) = far_cell_note(marine, env.verbose > 0, env.quiet) {
        eprintln!("note: {note}");
    }
}

/// What `-v` prints: the sampled cell, the current instant and the daily span.
fn raw_summary(response: &MarineResponse) -> String {
    let current = response
        .current
        .as_ref()
        .map_or("none", |block| block.time.as_str());
    let days = response.daily.as_ref().map_or(0, |daily| daily.time.len());
    format!(
        "sampled {:.4},{:.4} current {current} days {days}",
        response.latitude, response.longitude
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::{
        BASE, CURRENT_VARIABLES, DAILY_VARIABLES, decode, far_cell_note, haversine_km,
        marine_request,
    };
    use crate::model::{Location, LocationSource, Marine, MarineSource};
    use chrono::DateTime;

    fn location() -> Location {
        Location {
            name: "Sylt".to_owned(),
            admin1: None,
            country: "Germany".to_owned(),
            country_code: Some("DE".to_owned()),
            lat: 54.54,
            lon: 10.23,
            tz: chrono_tz::Tz::Europe__Berlin,
            elevation_m: None,
            population: None,
            source: LocationSource::Geocoder,
            station: None,
            named_by: None,
        }
    }

    fn reading(distance_km: f64) -> Marine {
        Marine {
            time: DateTime::parse_from_rfc3339("2026-10-05T23:00:00+02:00")
                .expect("a valid instant"),
            wave_height_m: Some(0.42),
            wave_direction_deg: Some(253),
            wave_period_s: Some(2.7),
            swell_wave_height_m: Some(0.04),
            sea_surface_temp_c: Some(16.1),
            days: Vec::new(),
            sampled_lat: 54.541_664,
            sampled_lon: 10.208_343_5,
            distance_km,
            source: MarineSource::OpenMeteoMarine,
        }
    }

    #[test]
    fn the_request_is_assembled_in_a_fixed_order() {
        let request = marine_request(&location());
        assert_eq!(request.url(), BASE);
        let pairs: Vec<(&str, &str)> = request
            .query_pairs()
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("latitude", "54.5400"),
                ("longitude", "10.2300"),
                ("current", CURRENT_VARIABLES),
                ("daily", DAILY_VARIABLES),
                ("cell_selection", "sea"),
                ("timezone", "auto"),
            ]
        );
    }

    #[test]
    fn a_far_cell_note_names_the_coordinate_and_distance() {
        let marine = reading(308.44);
        let note = far_cell_note(&marine, true, false).expect("a far cell is named");
        assert!(note.contains("54.5417"), "{note}");
        assert!(note.contains("10.2083"), "{note}");
        assert!(note.contains("308.4 km"), "{note}");
        assert_eq!(far_cell_note(&marine, false, false), None);
        assert_eq!(far_cell_note(&marine, true, true), None);
        assert_eq!(far_cell_note(&reading(1.4), true, false), None);
    }

    #[test]
    fn a_response_without_a_current_block_is_refused() {
        let response = super::MarineResponse {
            latitude: 54.541_664,
            longitude: 10.208_343_5,
            timezone: "Europe/Berlin".to_owned(),
            current: None,
            daily: None,
        };
        let error = decode(&response, &location()).expect_err("no current block");
        assert!(error.to_string().contains("no marine reading"), "{error}");
        assert!(error.to_string().contains("Sylt"), "{error}");
    }

    #[test]
    fn haversine_matches_the_recorded_sylt_offset() {
        let distance = haversine_km(54.54, 10.23, 54.541_664, 10.208_343_5);
        assert!((distance - 1.41).abs() < 0.02, "{distance}");
    }
}
