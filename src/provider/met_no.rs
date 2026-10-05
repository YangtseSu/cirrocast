// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! MET Norway's `Locationforecast/2.0` `compact` endpoint: the keyless global backend.
//!
//! One request per fetch, no API key, and one job after the response arrives: turn the
//! `properties.timeseries` steps into the four canonical day parts **in the location's zone**.
//!
//! What this module knows that the payload does not spell out:
//!
//! 1. **Coordinates go out at four decimals** ([`coordinate`]), truncated rather than rounded, so
//!    the requested point never drifts to a neighbouring grid cell; `altitude` is sent only when
//!    the location carries an elevation.
//! 2. **The response has no time zone and no daily block.** The location's zone is what the day
//!    parts are anchored in (raw coordinates therefore aggregate in their provisional `UTC`), and
//!    a day's extremes come from its own samples through [`dayparts::extremes`].
//! 3. **The horizon is stepped.** `next_1_hours` covers the first day and `next_6_hours` the rest,
//!    so a step is read from the finest block the row carries; a trailing row with no `next_*`
//!    block at all is dropped rather than filled with a fabricated zero.
//! 4. **Wind is the one non-canonical unit.** `properties.meta.units.wind_speed` is read and the
//!    value converted from `m/s` to km/h only when it says so; every other unit is already the
//!    canonical one.
//! 5. **`symbol_code` is a family, not a WMO code.** The `_day`/`_night`/`_polartwilight` suffix is
//!    stripped and the base looked up in the written-out table ([`SYMBOLS`]); an unknown base maps
//!    to WMO 3 (overcast) and names itself under `--verbose`.
//! 6. **The terms require the `Expires`/`If-Modified-Since` handshake.** The cache stores both
//!    headers and revalidates after `Expires`; a `304` serves the stored body, so an unchanged
//!    forecast costs headers instead of a body. Coordinates are the only parameters, and the
//!    shared `User-Agent` carries the project's contact information.
//!
//! Attribution: MET Norway's data is CC BY 4.0, which requires naming MET Norway as the source; the
//! registry row carries the line the renderers print.

use std::time::Duration;

use chrono::{DateTime, Timelike};
use chrono_tz::Tz;
use serde::Deserialize;

use super::dayparts::{HourSample, aggregate_day, covered_days, extremes};
use super::{
    Capabilities, Env, FetchRequest, Provider, ProviderId, attribution, local_today,
    note_short_series, requested_days,
};
use crate::cache::{CacheKey, Fetched};
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::{Condition, Current, Location, Report, ReportMode};

/// The provider id, as the registry and every error message spell it.
const PROVIDER: &str = "met-no";

/// The `compact` forecast endpoint; `lat`/`lon` are the only required parameters.
pub const BASE: &str = "https://api.met.no/weatherapi/locationforecast/2.0/compact";

/// Metres per second → km/h.
const MS_TO_KMH: f32 = 3.6;

/// The MET Norway base symbol codes and the WMO 4677 code each one maps to.
///
/// This is the whole official `weathericon/2.0` family — the base codes published with the icon
/// set, every variant stripped. `lightssleetshowersandthunder` and `lightssnowshowersandthunder`
/// keep the double `s` the provider itself calls a typing error; renaming them would break every
/// client, so the typo is reproduced verbatim.
///
/// Sleet has no WMO umbrella of its own, so `lightsleet`/`sleet`/`lightsleetshowers`/`sleetshowers`
/// map to WMO 68 (light rain and snow) and `heavysleet`/`heavysleetshowers` to WMO 69 (heavy rain
/// and snow); every `…andthunder` form maps to the WMO 95 thunderstorm.
pub const SYMBOLS: [(&str, u8); 41] = [
    ("clearsky", 0),
    ("fair", 1),
    ("partlycloudy", 2),
    ("cloudy", 3),
    ("fog", 45),
    ("lightrainshowers", 80),
    ("rainshowers", 81),
    ("heavyrainshowers", 82),
    ("lightrainshowersandthunder", 95),
    ("rainshowersandthunder", 95),
    ("heavyrainshowersandthunder", 95),
    ("lightsleetshowers", 68),
    ("sleetshowers", 68),
    ("heavysleetshowers", 69),
    ("lightssleetshowersandthunder", 95),
    ("sleetshowersandthunder", 95),
    ("heavysleetshowersandthunder", 95),
    ("lightsnowshowers", 85),
    ("snowshowers", 85),
    ("heavysnowshowers", 86),
    ("lightssnowshowersandthunder", 95),
    ("snowshowersandthunder", 95),
    ("heavysnowshowersandthunder", 95),
    ("lightrain", 61),
    ("rain", 63),
    ("heavyrain", 65),
    ("lightrainandthunder", 95),
    ("rainandthunder", 95),
    ("heavyrainandthunder", 95),
    ("lightsleet", 68),
    ("sleet", 68),
    ("heavysleet", 69),
    ("lightsleetandthunder", 95),
    ("sleetandthunder", 95),
    ("heavysleetandthunder", 95),
    ("lightsnow", 71),
    ("snow", 73),
    ("heavysnow", 75),
    ("lightsnowandthunder", 95),
    ("snowandthunder", 95),
    ("heavysnowandthunder", 95),
];

/// The MET Norway backend. Stateless: one value serves every fetch.
#[derive(Debug, Clone, Copy, Default)]
pub struct MetNo;

impl Provider for MetNo {
    fn id(&self) -> ProviderId {
        ProviderId::MetNo
    }

    fn capabilities(&self) -> Capabilities {
        ProviderId::MetNo.metadata().capabilities()
    }

    fn fetch_report(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
        // The CLI refuses `--date`/`--history` for a backend with no archive; this is the backstop
        // for a hand-built chain that reached met-no with a window anyway.
        if req.window.is_some() {
            return Err(Error::Usage(
                "met-no has no archive; drop --date/--history".to_owned(),
            ));
        }

        let max_days = self.capabilities().max_days;
        let days = requested_days(req.days, max_days, PROVIDER, env.quiet);
        let key = CacheKey::weather(PROVIDER, loc.lat, loc.lon, days, local_today(env, loc.tz));
        let request = compact_request(loc);
        let ttl = Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs));
        let place = format!("{} ({:.2}, {:.2})", loc.name, loc.lat, loc.lon);

        // The cache owns the `Expires`/`304` policy; the provider only supplies the conditional
        // request (`If-Modified-Since` from the stored entry) and hands back whatever came, so a
        // `304` refreshes the entry instead of transferring the body again.
        let response: CompactResponse =
            env.cache
                .read_or_fetch_with(&key, ttl, PROVIDER, "forecast", &place, |stale| {
                    let mut conditional = request.clone();
                    if let Some(modified) = stale.and_then(|entry| entry.last_modified.as_deref()) {
                        conditional = conditional.header("If-Modified-Since", modified);
                    }
                    let response = env.http.send(&conditional)?;
                    Ok(Fetched::of(&response))
                })?;

        report(&response, loc, request.redacted_url(), days, env)
    }
}

/// The `compact` request for a location: just the point, at four decimals.
fn compact_request(loc: &Location) -> HttpRequest {
    let mut request = HttpRequest::get(BASE)
        .query("lat", coordinate(loc.lat))
        .query("lon", coordinate(loc.lon));
    if let Some(elevation) = loc.elevation_m {
        request = request.query("altitude", format!("{elevation:.0}"));
    }
    request
}

/// A coordinate as MET Norway accepts it: at most four decimals, truncated toward zero.
///
/// Truncation (rather than rounding) keeps the requested point inside the caller's own cell: the
/// terms fix the resolution, and rounding would move `59.91399` to `59.9140` — a different point
/// than the one asked for.
fn coordinate(value: f64) -> String {
    let scaled = value * 10_000.0;
    let truncated = if scaled < 0.0 {
        scaled.ceil()
    } else {
        scaled.floor()
    };
    format!("{:.4}", truncated / 10_000.0)
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// The `compact` response, in the subset `cirrocast` consumes.
///
/// Unknown fields are ignored on purpose: MET adds variables without notice, and the extra
/// `next_12_hours` block on near-term rows is kept only as a last-resort fallback.
#[derive(Debug, Clone, Deserialize)]
pub struct CompactResponse {
    /// The `GeoJSON` feature's sibling payload: `meta` plus `timeseries`.
    pub properties: Properties,
}

/// The `properties` object.
#[derive(Debug, Clone, Deserialize)]
pub struct Properties {
    /// Update time and the units of every value in `timeseries`.
    pub meta: Meta,
    /// The forecast steps, oldest first.
    #[serde(default)]
    pub timeseries: Vec<Entry>,
}

/// The `meta` object.
#[derive(Debug, Clone, Deserialize)]
pub struct Meta {
    /// When the model run was produced (`2026-10-05T20:29:47Z`).
    pub updated_at: String,
    /// The units every value in `timeseries` is expressed in.
    #[serde(default)]
    pub units: Units,
}

/// The `meta.units` object, in the subset this backend reads.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Units {
    /// The wind speed unit; `m/s` is converted, anything else is treated as canonical km/h.
    #[serde(default)]
    pub wind_speed: Option<String>,
}

/// One forecast step.
#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    /// The step's instant, UTC (`2026-10-05T20:00:00Z`).
    pub time: String,
    /// The instant details plus the forward-looking precipitation/symbol blocks.
    pub data: Data,
}

/// A step's `data` object.
#[derive(Debug, Clone, Deserialize)]
pub struct Data {
    /// The conditions at `time`.
    pub instant: Instant,
    /// The following hour; present on the near-term rows.
    #[serde(default)]
    pub next_1_hours: Option<Period>,
    /// The following six hours; present once the horizon widens.
    #[serde(default)]
    pub next_6_hours: Option<Period>,
    /// The following twelve hours; present on some rows, used only as a fallback.
    #[serde(default)]
    pub next_12_hours: Option<Period>,
}

/// The `instant` object.
#[derive(Debug, Clone, Deserialize)]
pub struct Instant {
    /// The instant's measured values.
    pub details: InstantDetails,
}

/// The instant's `details`, every field optional because upstream omits a variable it has no
/// model for.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct InstantDetails {
    /// Mean sea level pressure in hPa.
    #[serde(default)]
    pub air_pressure_at_sea_level: Option<f32>,
    /// Air temperature in °C.
    #[serde(default)]
    pub air_temperature: Option<f32>,
    /// Total cloud cover in percent.
    #[serde(default)]
    pub cloud_area_fraction: Option<f32>,
    /// Relative humidity in percent.
    #[serde(default)]
    pub relative_humidity: Option<f32>,
    /// Direction the wind blows from, in degrees.
    #[serde(default)]
    pub wind_from_direction: Option<f32>,
    /// Wind speed in the unit `meta.units.wind_speed` names.
    #[serde(default)]
    pub wind_speed: Option<f32>,
}

/// A forward-looking period (`next_1_hours`, `next_6_hours`, `next_12_hours`).
#[derive(Debug, Clone, Deserialize)]
pub struct Period {
    /// The symbol summarizing the period.
    #[serde(default)]
    pub summary: Option<Summary>,
    /// The period's accumulated values.
    #[serde(default)]
    pub details: PeriodDetails,
}

/// A period's `summary`.
#[derive(Debug, Clone, Deserialize)]
pub struct Summary {
    /// The MET symbol code, e.g. `clearsky_night`.
    pub symbol_code: String,
}

/// A period's `details`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PeriodDetails {
    /// Precipitation accumulated over the period, in mm.
    #[serde(default)]
    pub precipitation_amount: Option<f32>,
}

// ---------------------------------------------------------------------------------------------
// Response → canonical model
// ---------------------------------------------------------------------------------------------

/// Turns one response into a [`Report`].
fn report(
    response: &CompactResponse,
    loc: &Location,
    url: String,
    days: u8,
    env: &Env<'_>,
) -> Result<Report> {
    let tz = loc.tz;
    let wind = WindUnit::from_label(response.properties.meta.units.wind_speed.as_deref());
    let current = current_of(response, tz, wind, env);

    let mut forecasts = Vec::new();
    if days > 0 {
        let samples = samples(response, tz, wind, env)?;
        if samples.is_empty() {
            return Err(upstream("the response has no usable forecast step"));
        }
        // The compact payload has no daily block, so the extremes come from the day's own samples.
        for date in covered_days(&samples, tz, days) {
            let (temp_min_c, temp_max_c) = extremes(&samples, date, tz, PROVIDER)?;
            forecasts.push(aggregate_day(
                &samples, date, tz, PROVIDER, temp_min_c, temp_max_c, None, None,
            )?);
        }
        if forecasts.is_empty() {
            return Err(upstream(format!(
                "the response covers no complete local day in {tz}"
            )));
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
            ProviderId::MetNo,
            url,
            env.cache.clock().now().into(),
            (env.verbose > 0).then(|| raw_summary(response)),
        ),
    })
}

/// The usable steps, in canonical units and the location's zone.
fn samples(
    response: &CompactResponse,
    tz: Tz,
    wind: WindUnit,
    env: &Env<'_>,
) -> Result<Vec<HourSample>> {
    let mut samples = Vec::new();
    for entry in &response.properties.timeseries {
        if let Some(sample) = sample(entry, tz, wind, env)? {
            samples.push(sample);
        }
    }
    samples.sort_by_key(|sample| sample.at);
    Ok(samples)
}

/// One step as a sample, or `None` when it carries no usable period, symbol or temperature.
fn sample(entry: &Entry, tz: Tz, wind: WindUnit, env: &Env<'_>) -> Result<Option<HourSample>> {
    let Some(period) = period(entry) else {
        return Ok(None);
    };
    let Some(symbol) = symbol_code(period) else {
        return Ok(None);
    };
    let details = &entry.data.instant.details;
    let (Some(temp_c), Some(speed)) = (details.air_temperature, details.wind_speed) else {
        return Ok(None);
    };
    let at = instant(&entry.time, tz)?;
    Ok(Some(HourSample {
        at,
        temp_c,
        // The compact payload carries no apparent temperature.
        feels_like_c: None,
        precip_mm: period.details.precipitation_amount.unwrap_or(0.0),
        // ... nor a precipitation probability.
        precip_prob_pct: None,
        weather: condition_of(symbol, env.verbose > 0),
        wind_kmh: wind.kmh(speed),
        wind_dir_deg: details.wind_from_direction.map(degrees),
        humidity_pct: details.relative_humidity.map(percent),
        // ... nor a visibility.
        visibility_km: None,
    }))
}

/// The current conditions, from the first step.
///
/// `None` when that step lacks a value the canonical `Current` requires; the renderers handle a
/// missing current block, and a zero would be a lie.
fn current_of(
    response: &CompactResponse,
    tz: Tz,
    wind: WindUnit,
    env: &Env<'_>,
) -> Option<Current> {
    let entry = response.properties.timeseries.first()?;
    let period = period(entry)?;
    let symbol = symbol_code(period)?;
    let details = &entry.data.instant.details;
    let at = instant(&entry.time, tz).ok()?;
    Some(Current {
        observed_at: at.fixed_offset(),
        temp_c: details.air_temperature?,
        feels_like_c: None,
        humidity_pct: details.relative_humidity.map(percent),
        precip_mm: period.details.precipitation_amount.unwrap_or(0.0),
        weather: condition_of(symbol, env.verbose > 0),
        cloud_cover_pct: details.cloud_area_fraction.map(percent),
        pressure_hpa: details.air_pressure_at_sea_level?,
        wind_kmh: wind.kmh(details.wind_speed?),
        wind_dir_deg: details.wind_from_direction.map(degrees),
        wind_gust_kmh: None,
        visibility_km: None,
        uv_index: None,
        // No daylight flag: the local civil day (06:00–18:00) is the closest this payload offers.
        is_day: (6..18).contains(&at.hour()),
    })
}

/// The finest forward-looking period a step carries.
///
/// Near-term rows offer 1-, 6- and 12-hour blocks; the horizon keeps only the 6-hour one, and a
/// few trailing rows carry none at all (which drops the step).
fn period(entry: &Entry) -> Option<&Period> {
    entry
        .data
        .next_1_hours
        .as_ref()
        .or(entry.data.next_6_hours.as_ref())
        .or(entry.data.next_12_hours.as_ref())
}

/// The symbol code of a period, when it carries a summary.
fn symbol_code(period: &Period) -> Option<&str> {
    period
        .summary
        .as_ref()
        .map(|summary| summary.symbol_code.as_str())
}

/// Parses an RFC 3339 instant and converts it to the location's zone.
fn instant(text: &str, tz: Tz) -> Result<DateTime<Tz>> {
    DateTime::parse_from_rfc3339(text)
        .map(|at| at.with_timezone(&tz))
        .map_err(|error| Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!("the timestamp `{text}` is not an RFC 3339 instant: {error}"),
        })
}

/// An upstream failure with no HTTP status of its own (a decoded payload that cannot be rendered).
fn upstream(message: impl Into<String>) -> Error {
    Error::Upstream {
        provider: PROVIDER.to_owned(),
        status: None,
        message: message.into(),
    }
}

// ---------------------------------------------------------------------------------------------
// Symbol table
// ---------------------------------------------------------------------------------------------

/// The WMO code for a MET symbol, or `None` when the base code is not in the table.
fn wmo_of(symbol: &str) -> Option<Condition> {
    let base = base_symbol(symbol);
    SYMBOLS
        .iter()
        .find(|(name, _)| *name == base)
        .map(|&(_, code)| Condition::from_u8(code))
}

/// Strips the `_day`/`_night`/`_polartwilight` variant suffix, when present.
fn base_symbol(symbol: &str) -> &str {
    ["_polartwilight", "_night", "_day"]
        .into_iter()
        .find_map(|suffix| symbol.strip_suffix(suffix))
        .unwrap_or(symbol)
}

/// The canonical condition for a symbol; an unknown base becomes WMO 3 and names itself under
/// `--verbose`.
fn condition_of(symbol: &str, verbose: bool) -> Condition {
    wmo_of(symbol).unwrap_or_else(|| {
        if verbose {
            eprintln!(
                "note: {PROVIDER} symbol `{symbol}` is not in the table; reporting WMO 3 (overcast)"
            );
        }
        Condition::from_u8(3)
    })
}

/// The wind speed unit a response declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindUnit {
    /// Already canonical.
    Kmh,
    /// Metres per second, converted at decode time.
    Mps,
}

impl WindUnit {
    /// The unit `label` names; anything but `m/s` is treated as canonical km/h.
    fn from_label(label: Option<&str>) -> Self {
        match label.map(str::trim) {
            Some(label) if label.eq_ignore_ascii_case("m/s") => Self::Mps,
            _ => Self::Kmh,
        }
    }

    /// `value` in km/h.
    fn kmh(self, value: f32) -> f32 {
        match self {
            Self::Mps => value * MS_TO_KMH,
            Self::Kmh => value,
        }
    }
}

/// A percentage from upstream, clamped into the model's `u8`.
fn percent(value: f32) -> u8 {
    clamped_u8(value, 100)
}

/// Rounds an upstream value and clamps it into `0..=max`.
///
/// The casts are safe by construction — the clamp runs first, so the value is an integral `f32`
/// inside the target range — which is exactly what the truncation/sign-loss lints cannot see.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn clamped_u8(value: f32, max: u8) -> u8 {
    value.round().clamp(0.0, f32::from(max)) as u8
}

/// A wind direction in degrees, wrapped into `0..360`.
///
/// Rounding first is what makes the wrap correct for `359.7`; the conversion cannot fail because
/// the result of `rem_euclid(360)` is non-negative and below 360.
#[allow(clippy::cast_possible_truncation)]
fn degrees(value: f32) -> u16 {
    let wrapped = (value.round() as i64).rem_euclid(360);
    u16::try_from(wrapped).unwrap_or_default()
}

/// What `-v` prints: the run's update time and unit plus the `(instant, WMO code)` pairs.
fn raw_summary(response: &CompactResponse) -> String {
    let pairs: Vec<(String, u8)> = response
        .properties
        .timeseries
        .iter()
        .map(|entry| {
            let code = period(entry)
                .and_then(symbol_code)
                .and_then(wmo_of)
                .map_or(255, Condition::code);
            (entry.time.clone(), code)
        })
        .collect();
    let pairs = serde_json::to_string(&pairs).unwrap_or_else(|_| "[]".to_owned());
    format!(
        "updated {}; wind {}; {pairs}",
        response.properties.meta.updated_at,
        response
            .properties
            .meta
            .units
            .wind_speed
            .as_deref()
            .unwrap_or("?"),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::{SYMBOLS, base_symbol, condition_of, coordinate, wmo_of};
    use crate::model::Condition;

    #[test]
    fn every_base_symbol_maps_to_the_documented_wmo_code() {
        // The independent expectation: the official `weathericon/2.0` base list, written out
        // again so the table's own values are checked rather than echoed.
        let expected: [(&str, u8); 41] = [
            ("clearsky", 0),
            ("fair", 1),
            ("partlycloudy", 2),
            ("cloudy", 3),
            ("fog", 45),
            ("lightrainshowers", 80),
            ("rainshowers", 81),
            ("heavyrainshowers", 82),
            ("lightrainshowersandthunder", 95),
            ("rainshowersandthunder", 95),
            ("heavyrainshowersandthunder", 95),
            ("lightsleetshowers", 68),
            ("sleetshowers", 68),
            ("heavysleetshowers", 69),
            ("lightssleetshowersandthunder", 95),
            ("sleetshowersandthunder", 95),
            ("heavysleetshowersandthunder", 95),
            ("lightsnowshowers", 85),
            ("snowshowers", 85),
            ("heavysnowshowers", 86),
            ("lightssnowshowersandthunder", 95),
            ("snowshowersandthunder", 95),
            ("heavysnowshowersandthunder", 95),
            ("lightrain", 61),
            ("rain", 63),
            ("heavyrain", 65),
            ("lightrainandthunder", 95),
            ("rainandthunder", 95),
            ("heavyrainandthunder", 95),
            ("lightsleet", 68),
            ("sleet", 68),
            ("heavysleet", 69),
            ("lightsleetandthunder", 95),
            ("sleetandthunder", 95),
            ("heavysleetandthunder", 95),
            ("lightsnow", 71),
            ("snow", 73),
            ("heavysnow", 75),
            ("lightsnowandthunder", 95),
            ("snowandthunder", 95),
            ("heavysnowandthunder", 95),
        ];
        assert_eq!(SYMBOLS.len(), expected.len(), "the official base list");
        for (name, code) in expected {
            assert_eq!(
                wmo_of(name).map(Condition::code),
                Some(code),
                "`{name}` must map to WMO {code}"
            );
        }
        // The two double-`s` spellings are the provider's own documented typo.
        assert_eq!(
            wmo_of("lightssleetshowersandthunder").map(Condition::code),
            Some(95)
        );
        assert_eq!(
            wmo_of("lightssnowshowersandthunder").map(Condition::code),
            Some(95)
        );
    }

    #[test]
    fn variant_suffixes_are_stripped_before_lookup() {
        assert_eq!(base_symbol("clearsky_night"), "clearsky");
        assert_eq!(base_symbol("clearsky_day"), "clearsky");
        assert_eq!(base_symbol("partlycloudy_polartwilight"), "partlycloudy");
        assert_eq!(
            base_symbol("lightssleetshowersandthunder_polartwilight"),
            "lightssleetshowersandthunder"
        );
        assert_eq!(
            base_symbol("cloudy"),
            "cloudy",
            "a suffixless code is untouched"
        );
        assert_eq!(
            wmo_of("clearsky_day").map(Condition::code),
            Some(0),
            "the suffix does not change the code"
        );
    }

    #[test]
    fn an_unknown_symbol_falls_back_to_overcast() {
        assert_eq!(wmo_of("not-a-symbol"), None);
        assert_eq!(
            wmo_of("lightrain_weekend"),
            None,
            "an unknown suffix is not stripped"
        );
        assert_eq!(condition_of("not-a-symbol", false), Condition::from_u8(3));
        // The verbose branch emits the note and still answers overcast.
        assert_eq!(condition_of("not-a-symbol", true), Condition::from_u8(3));
    }

    #[test]
    fn coordinates_are_truncated_to_four_decimals() {
        assert_eq!(coordinate(59.9139), "59.9139");
        assert_eq!(coordinate(10.7522), "10.7522");
        assert_eq!(coordinate(59.91399), "59.9139", "truncated, not rounded");
        assert_eq!(
            coordinate(-9.13939),
            "-9.1393",
            "negative values truncate toward zero"
        );
        assert_eq!(coordinate(52.52), "52.5200");
    }
}
