// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! SMHI's `SNOW1gv1` point forecast: the keyless Nordic backend.
//!
//! One request per fetch, no parameters: the `parameters=` filter is deliberately not sent because
//! SMHI's separator handling is literal-comma only (`%2C` silently returns just the first
//! parameter), and the unfiltered answer for a point is a few kilobytes.
//!
//! What this module knows that the payload does not spell out:
//!
//! 1. **`9999` is the missing value, in band.** Every parameter can carry it instead of `null`;
//!    it becomes `None` here and never a number.
//! 2. **A step covers an interval, not an instant.** `time` is the interval's end and
//!    `intervalParametersStartTime` its start, so a step's precipitation is the total for that
//!    window — summing the steps of a day part accumulates correctly whatever the step length.
//! 3. **Step length widens with the horizon** (1 h near-term, then 6 h, then 12 h), and the series
//!    starts at the current hour, so the *current* conditions are the most recent step whose
//!    interval has already started.
//! 4. **The series starts at the current hour**, so the location-local today is only complete when
//!    the fetch happens before 06:00 local (its night hours are in the past). A `DayPart` cannot
//!    represent "no data", so the backend emits the first days whose four parts are all covered —
//!    usually starting tomorrow — rather than inventing values for windows SMHI never served. The
//!    current-conditions block still answers "what is it like now".
//! 5. **The payload carries no time zone and no daylight flag.** A location whose zone is still the
//!    provisional `UTC` (raw coordinates, an untagged OSM place) is refused with a usage error
//!    instead of being aggregated in the wrong zone; `is_day` comes from the local civil day
//!    (06:00–18:00), which step 17's astro work can replace with real sun times.
//!
//! Attribution: SMHI's open data is CC BY 4.0 SE, which requires naming SMHI as the source; the
//! registry row carries the line the renderers print.

use std::time::Duration;

use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Tz;
use serde::Deserialize;

use super::dayparts::{HourSample, aggregate_day, covered_days, extremes};
use super::{
    Capabilities, Env, FetchRequest, JsonFetch, Provider, ProviderId, attribution, fetch_json,
    local_today, note_short_series, requested_days,
};
use crate::cache::CacheKey;
use crate::error::{Error, Result};
use crate::geo::provisional_zone;
use crate::http::HttpRequest;
use crate::model::{Condition, Current, Location, Report, ReportMode};

/// The provider id, as the registry and every error message spell it.
const PROVIDER: &str = "smhi";

/// The `SNOW1gv1` root; the path continues with `geotype/point/lon/<lon>/lat/<lat>/data.json`.
///
/// `lon` comes before `lat`, unlike the decommissioned `pmp3g` service this replaced.
pub const BASE: &str = "https://opendata-download-metfcst.smhi.se/api/category/snow1g/version/1";

/// The value SMHI writes for "no data"; it arrives in band, not as `null`.
const MISSING: f32 = 9999.0;

/// Metres per second → km/h.
const MS_TO_KMH: f32 = 3.6;

/// The SMHI backend. Stateless: one value serves every fetch.
#[derive(Debug, Clone, Copy, Default)]
pub struct Smhi;

impl Provider for Smhi {
    fn id(&self) -> ProviderId {
        ProviderId::Smhi
    }

    fn capabilities(&self) -> Capabilities {
        ProviderId::Smhi.metadata().capabilities()
    }

    fn fetch_report(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
        let max_days = self.capabilities().max_days;
        let days = requested_days(req.days, max_days, PROVIDER, env.quiet);
        let key = CacheKey::weather(PROVIDER, loc.lat, loc.lon, days, local_today(env, loc.tz));
        let request = point_request(loc);
        let ttl = Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs));

        let response: PointResponse = fetch_json(
            env,
            loc,
            &JsonFetch {
                provider: ProviderId::Smhi,
                request: request.clone(),
                key,
                ttl,
                what: "point forecast",
            },
        )
        .map_err(|error| out_of_coverage(error, loc))?;

        report(&response, loc, request.redacted_url(), days, env)
    }
}

/// The point-forecast URL for a location, with the grid-snapping left to upstream.
fn point_request(loc: &Location) -> HttpRequest {
    HttpRequest::get(format!(
        "{BASE}/geotype/point/lon/{:.4}/lat/{:.4}/data.json",
        loc.lon, loc.lat
    ))
}

/// Names an out-of-coverage point, which SMHI answers with a bare `404` (the docs claim `400`).
///
/// Anything else keeps its own taxonomy: a chain still falls through, but the `-v` line and the
/// fallback warning say *why* SMHI could not answer.
fn out_of_coverage(error: Error, loc: &Location) -> Error {
    match error {
        Error::Upstream {
            status: Some(400 | 404),
            ..
        } => Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: Some(404),
            message: format!(
                "out of coverage: {:.2},{:.2} is outside the SMHI valid area",
                loc.lat, loc.lon
            ),
        },
        other => other,
    }
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// The `SNOW1gv1` point response, in the subset `circoast` consumes.
///
/// Unknown fields are ignored on purpose: SMHI adds parameters to the payload without notice.
#[derive(Debug, Clone, Deserialize)]
pub struct PointResponse {
    /// When the run was produced.
    #[serde(rename = "createdTime")]
    pub created_time: String,
    /// The model's valid-from time.
    #[serde(rename = "referenceTime")]
    pub reference_time: String,
    /// The grid point the answer is for (`[lon, lat]`).
    pub geometry: Geometry,
    /// The forecast steps.
    #[serde(rename = "timeSeries", default)]
    pub time_series: Vec<Step>,
}

/// The grid point upstream answered for.
#[derive(Debug, Clone, Deserialize)]
pub struct Geometry {
    /// `[longitude, latitude]`.
    pub coordinates: Vec<f64>,
}

/// One forecast step: an interval and its parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    /// The interval's end, UTC (`2026-09-30T16:00:00Z`).
    pub time: String,
    /// The interval's start; absent on very old payloads, in which case the step is treated as one
    /// hour long.
    #[serde(rename = "intervalParametersStartTime", default)]
    pub interval_start: Option<String>,
    /// The parameters, keyed by name.
    #[serde(default)]
    pub data: Data,
}

/// The parameters this backend reads; every one of them can be missing or `9999`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Data {
    /// Air temperature at 2 m, °C.
    pub air_temperature: Option<f32>,
    /// Wind speed at 10 m, m/s.
    pub wind_speed: Option<f32>,
    /// Direction the wind blows from at 10 m, degrees.
    pub wind_from_direction: Option<f32>,
    /// Gust speed at 10 m, m/s.
    pub wind_speed_of_gust: Option<f32>,
    /// Relative humidity at 2 m, percent.
    pub relative_humidity: Option<f32>,
    /// Mean sea level pressure, hPa.
    pub air_pressure_at_mean_sea_level: Option<f32>,
    /// Horizontal visibility, km.
    pub visibility_in_air: Option<f32>,
    /// The weather symbol, 1–27 (an integer that arrives as a float).
    pub symbol_code: Option<f32>,
    /// Precipitation accumulated over the step, kg/m² (numerically mm).
    pub precipitation_amount_mean_deterministic: Option<f32>,
    /// Probability of precipitation, percent.
    pub probability_of_precipitation: Option<f32>,
    /// Total cloud cover, oktas (0–8).
    pub cloud_area_fraction: Option<f32>,
}

// ---------------------------------------------------------------------------------------------
// Response → canonical model
// ---------------------------------------------------------------------------------------------

/// Turns one response into a [`Report`].
///
/// `days[0]` is the first location-local date whose four parts all have a sample (today whenever
/// the steps cover it, the next date when they do not); SMHI's 12-hour tail steps leave the last
/// requested day incomplete, so a `-v` note reports the shortfall.
fn report(
    response: &PointResponse,
    loc: &Location,
    url: String,
    days: u8,
    env: &Env<'_>,
) -> Result<Report> {
    if provisional_zone(loc) {
        return Err(Error::Usage(format!(
            "provider `{PROVIDER}` needs the location's time zone and its response carries none: \
             pass a place name (e.g. `cirrocast -p {PROVIDER} Stockholm`) or set `location.default` \
             instead of raw coordinates"
        )));
    }

    let tz = loc.tz;
    let samples = samples(response, tz)?;
    if samples.is_empty() {
        return Err(Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: "the response has no usable forecast step".to_owned(),
        });
    }

    let current = current_of(response, tz, env);

    let mut forecasts = Vec::new();
    if days > 0 {
        for date in covered_days(&samples, tz, days) {
            let (temp_min_c, temp_max_c) = extremes(&samples, date, tz, PROVIDER)?;
            forecasts.push(aggregate_day(
                &samples, date, tz, PROVIDER, temp_min_c, temp_max_c, None, None,
            )?);
        }
        if forecasts.is_empty() {
            return Err(Error::Upstream {
                provider: PROVIDER.to_owned(),
                status: None,
                message: format!("the response covers no complete local day in {tz}"),
            });
        }
        note_short_series(forecasts.len(), days, PROVIDER, env);
    }

    Ok(Report {
        location: loc.clone(),
        current,
        days: forecasts,
        alerts: Vec::new(),
        air: None,
        astro: None,
        marine: None,
        mode: ReportMode::Forecast,
        attribution: attribution(
            ProviderId::Smhi,
            url,
            env.cache.clock().now().into(),
            (env.verbose > 0).then(|| raw_summary(response)),
        ),
    })
}

/// The usable steps, in canonical units and the location's zone.
///
/// A step is dropped when a value the canonical model has no `Option` for is missing (temperature,
/// wind speed and direction, precipitation, symbol): a hole must not become a zero.
fn samples(response: &PointResponse, tz: Tz) -> Result<Vec<HourSample>> {
    let mut samples = Vec::new();
    for step in &response.time_series {
        let Some(sample) = sample(step, tz)? else {
            continue;
        };
        samples.push(sample);
    }
    samples.sort_by_key(|sample| sample.at);
    Ok(samples)
}

/// One step as a sample, or `None` when it lacks a value the canonical model requires.
fn sample(step: &Step, tz: Tz) -> Result<Option<HourSample>> {
    let at = instant(&step.time)?;
    let (Some(temp_c), Some(wind_mps), Some(direction), Some(precip_mm), Some(symbol)) = (
        value(step.data.air_temperature),
        value(step.data.wind_speed),
        value(step.data.wind_from_direction),
        value(step.data.precipitation_amount_mean_deterministic),
        value(step.data.symbol_code),
    ) else {
        return Ok(None);
    };

    Ok(Some(HourSample {
        at: at.with_timezone(&tz),
        temp_c,
        // SMHI publishes no apparent temperature; `None` keeps the renderers honest.
        feels_like_c: None,
        precip_mm,
        precip_prob_pct: value(step.data.probability_of_precipitation).map(rounded_percent),
        weather: condition_of(symbol),
        wind_kmh: wind_mps * MS_TO_KMH,
        wind_dir_deg: Some(degrees(direction)),
        humidity_pct: value(step.data.relative_humidity).map(rounded_percent),
        visibility_km: value(step.data.visibility_in_air),
    }))
}

/// The current conditions: the most recent step whose interval has already started.
///
/// `None` when that step lacks a value the canonical `Current` requires — the renderers handle a
/// missing current block, and a zero would be a lie.
fn current_of(response: &PointResponse, tz: Tz, env: &Env<'_>) -> Option<Current> {
    let now: DateTime<Utc> = env.cache.clock().now().into();
    let step = response
        .time_series
        .iter()
        .rfind(|step| interval_start(step).is_some_and(|start| start <= now))
        .or_else(|| response.time_series.first())?;

    let observed_at = instant(&step.time).ok()?.with_timezone(&tz);
    let temp_c = value(step.data.air_temperature)?;
    // Humidity and cloud cover are optional in the canonical `Current`: SMHI's in-band `9999`
    // marks a missing reading, which must null only that field, not the whole block.
    let humidity = value(step.data.relative_humidity);
    let cloud = value(step.data.cloud_area_fraction);
    let pressure = value(step.data.air_pressure_at_mean_sea_level)?;
    let wind_mps = value(step.data.wind_speed)?;
    let direction = value(step.data.wind_from_direction)?;
    let symbol = value(step.data.symbol_code)?;

    Some(Current {
        observed_at: observed_at.fixed_offset(),
        temp_c,
        feels_like_c: None,
        humidity_pct: humidity.map(rounded_percent),
        precip_mm: value(step.data.precipitation_amount_mean_deterministic).unwrap_or(0.0),
        weather: condition_of(symbol),
        cloud_cover_pct: cloud.map(|cloud| rounded_percent(cloud * 12.5)),
        pressure_hpa: pressure,
        wind_kmh: wind_mps * MS_TO_KMH,
        wind_dir_deg: Some(degrees(direction)),
        wind_gust_kmh: value(step.data.wind_speed_of_gust).map(|mps| mps * MS_TO_KMH),
        visibility_km: value(step.data.visibility_in_air),
        uv_index: None,
        // The payload has no daylight flag; the local civil day is the honest stand-in until step
        // 17 computes real sun times.
        is_day: matches!(observed_at.hour(), 6..=17),
    })
}

/// A parameter value, with SMHI's in-band missing marker turned into `None`.
fn value(raw: Option<f32>) -> Option<f32> {
    raw.filter(|value| (*value - MISSING).abs() > f32::EPSILON)
}

/// A 0–100 parameter as a whole percent.
///
/// The cast is safe by construction — the clamp runs first, so the value is an integral `f32`
/// inside the target range — which is exactly what the truncation/sign-loss lints cannot see.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn rounded_percent(value: f32) -> u8 {
    value.round().clamp(0.0, 100.0) as u8
}

/// A direction in degrees, normalised into `0..360`.
///
/// Rounding first is what makes the wrap correct for `359.7`; the conversion cannot fail because
/// the result of `rem_euclid(360)` is non-negative and below 360.
#[allow(clippy::cast_possible_truncation)]
fn degrees(value: f32) -> u16 {
    let wrapped = (value.round() as i64).rem_euclid(360);
    u16::try_from(wrapped).unwrap_or_default()
}

/// A step's interval start: the payload's own field, else one hour before the interval's end.
fn interval_start(step: &Step) -> Option<DateTime<Utc>> {
    match step.interval_start.as_deref() {
        Some(text) => instant(text).ok(),
        None => instant(&step.time)
            .ok()
            .map(|end| end - chrono::Duration::hours(1)),
    }
}

/// Parses an ISO 8601 UTC timestamp (`2026-09-30T16:00:00Z`).
fn instant(text: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text.trim())
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!("`{text}` is not an ISO 8601 timestamp: {error}"),
        })
}

/// SMHI's `symbol_code` (1–27) as a WMO 4677 code.
///
/// Sleet has no WMO umbrella, so the `66`/`67` family carries it (documented in the provider
/// reference); an unlisted code stays unknown instead of being clamped into a neighbour.
fn condition_of(symbol: f32) -> Condition {
    // The cast cannot fail: SMHI's codes are small integers, and a hostile one only lands on an
    // undescribed value that `from_u8` keeps as `Unknown`.
    #[allow(clippy::cast_possible_truncation)]
    let code = symbol.round() as i64;
    Condition::from_u8(match code {
        1 => 0,                  // clear sky
        2 => 1,                  // nearly clear
        3 | 4 => 2,              // variable cloudiness / halfclear
        5 | 6 => 3,              // cloudy (5 is absent from SMHI's own symbol page) / overcast
        7 => 45,                 // fog
        8 => 80,                 // light rain showers
        9 => 81,                 // moderate rain showers
        10 => 82,                // heavy rain showers
        11 | 21 => 95,           // thunderstorm / rain with thunder
        12 | 22 => 66,           // light sleet showers / light sleet
        13 | 14 | 23 | 24 => 67, // moderate / heavy sleet, showers or steady
        15 | 16 => 85,           // light / moderate snow showers
        17 => 86,                // heavy snow showers
        18 => 61,                // light rain
        19 => 63,                // moderate rain
        20 => 65,                // heavy rain
        25 => 71,                // light snowfall
        26 => 73,                // moderate snowfall
        27 => 75,                // heavy snowfall
        _ => 255,                // undescribed: `Condition::from_u8` keeps 255 as unknown
    })
}

/// What `-v` prints: the run's timestamps, the grid point and the step count.
fn raw_summary(response: &PointResponse) -> String {
    let point = match response.geometry.coordinates.as_slice() {
        [lon, lat] => format!("{lat:.4},{lon:.4}"),
        _ => "unknown".to_owned(),
    };
    format!(
        "created {} reference {} point {} steps {}",
        response.created_time,
        response.reference_time,
        point,
        response.time_series.len()
    )
}
