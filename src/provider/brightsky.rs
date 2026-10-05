// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Bright Sky's `/weather` endpoint: DWD open data resold by a keyless, self-hostable service.
//!
//! One request per fetch, no API key, and one job after the response arrives: turn the hourly
//! `weather[]` rows into the four canonical day parts **in the location's zone**.
//!
//! What this module knows that the payload does not spell out (all measured on 2026-10-06, the day
//! the provider landed):
//!
//! 1. **`last_date` is an inclusive *timestamp*, not a date.** `date=2026-10-06&last_date=…`
//!    returned 1 row for `last_date=2026-10-06`, 25 for `2026-10-07` and 49 for `2026-10-08`: the
//!    answer always ends at `last_date` 00:00Z. Asking for `today + days - 1` therefore serves
//!    `days - 1` whole days, so the request uses `last_date = today + days` — the instant at the
//!    start of the day *after* the last requested one — which yields exactly `days` whole days (the
//!    trailing boundary row is the incomplete next day and is dropped by `covered_days`).
//! 2. **The horizon is 10 whole days.** A request far past the horizon clamps to the forecast
//!    feed's `sources[].last_record` (`2026-10-16T04:00Z` at Berlin's `BERLIN-ALEX.`, Munich's
//!    `MUENCHEN STADT` and Bergen's `BERGEN` alike on 2026-10-06). Ten whole days fit under that
//!    boundary (`today + 10` = `2026-10-16T00:00Z`), so the registry row declares `max_days: 10`.
//! 3. **`units` stays at the API's default, `dwd`.** That default is the canonical set the model
//!    wants (`temperature` °C, `wind_speed` km/h, `precipitation` mm, `visibility` m,
//!    `relative_humidity` %); asking for `units=si` would return Kelvins, m/s and Pascals, which
//!    the provider would have to convert back. The field names stay the same in either mode.
//! 4. **`null` is "no reading", never a zero.** `precipitation` is required by the canonical day
//!    part (a hole must not become 0 mm), so a row without it is **dropped**; `relative_humidity`,
//!    `visibility`, `cloud_cover`, gust and wind direction are optional in the model and keep their
//!    own nullness. The Berlin recording happens to report `relative_humidity: null` on every row,
//!    which is exactly this case.
//! 5. **`condition` and `icon` are two small closed sets.** They are written out ([`CONDITIONS`],
//!    [`ICONS`]) and exhaustiveness-tested; a value outside either falls back to WMO 3 (overcast)
//!    and names itself under `--verbose`. `condition` is the precipitation/obscuration state, so it
//!    wins whenever it is not `dry`; for `dry` the sky state comes from `icon`.
//! 6. **The `sources[]` block is station metadata**, not data: it is carried into
//!    `Attribution.raw` so `-v` names the DWD station (`BERLIN-ALEX.`, id, WMO/DWD ids, distance).
//!
//! Attribution: Bright Sky serves DWD open data, which is CC BY 4.0; the registry row carries the
//! line the renderers print, and the DWD's own terms of use apply to the data.

use std::time::Duration;

use chrono::{DateTime, Days, NaiveDate, Timelike, Utc};
use chrono_tz::Tz;
use serde::Deserialize;

use super::dayparts::{HourSample, aggregate_day, covered_days, extremes};
use super::{
    Capabilities, Env, FetchRequest, JsonFetch, Provider, ProviderId, attribution, fetch_json,
    local_today, note_short_series, requested_days,
};
use crate::cache::CacheKey;
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::{Condition, Current, Location, Report, ReportMode};

/// The provider id, as the registry and every error message spell it.
const PROVIDER: &str = "brightsky";

/// The `/weather` endpoint; `lat`/`lon`/`date` are required and `last_date` bounds the span.
pub const BASE: &str = "https://api.brightsky.dev/weather";

/// The `condition` values Bright Sky documents, and the WMO 4677 code each one maps to.
///
/// The set is the API's own enum (`dry`/`fog`/`rain`/`sleet`/`snow`/`hail`/`thunderstorm`).
/// `dry` means "no precipitation" — the sky state is then read from [`ICONS`], which is why its
/// own code here is only the fallback for a row that carries no icon.
pub const CONDITIONS: [(&str, u8); 7] = [
    ("dry", 0),
    ("fog", 45),
    ("rain", 63),
    ("sleet", 68),
    ("snow", 73),
    ("hail", 96),
    ("thunderstorm", 95),
];

/// The `icon` values Bright Sky documents, and the WMO 4677 code each one maps to.
///
/// The set is the API's own enum, DWD's icon family. `wind` is not a sky state and WMO has no
/// wind-only code, so it maps to overcast (3).
pub const ICONS: [(&str, u8); 12] = [
    ("clear-day", 0),
    ("clear-night", 0),
    ("partly-cloudy-day", 2),
    ("partly-cloudy-night", 2),
    ("cloudy", 3),
    ("fog", 45),
    ("wind", 3),
    ("rain", 63),
    ("sleet", 68),
    ("snow", 73),
    ("hail", 96),
    ("thunderstorm", 95),
];

/// The Bright Sky backend. Stateless: one value serves every fetch.
#[derive(Debug, Clone, Copy, Default)]
pub struct BrightSky;

impl Provider for BrightSky {
    fn id(&self) -> ProviderId {
        ProviderId::BrightSky
    }

    fn capabilities(&self) -> Capabilities {
        ProviderId::BrightSky.metadata().capabilities()
    }

    fn fetch_report(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
        // The CLI refuses `--date`/`--history` for a backend with no archive; this is the backstop
        // for a hand-built chain that reached bright-sky with a window anyway.
        if req.window.is_some() {
            return Err(Error::Usage(
                "brightsky has no archive; drop --date/--history".to_owned(),
            ));
        }

        let max_days = self.capabilities().max_days;
        let days = requested_days(req.days, max_days, PROVIDER, env.quiet);
        let today = local_today(env, loc.tz);
        let key = CacheKey::weather(PROVIDER, loc.lat, loc.lon, days, today);
        let request = weather_request(loc, today, days)?;
        let ttl = Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs));

        let response: WeatherResponse = fetch_json(
            env,
            loc,
            &JsonFetch {
                provider: ProviderId::BrightSky,
                request: request.clone(),
                key,
                ttl,
                what: "hourly forecast",
            },
        )?;

        report(&response, loc, request.redacted_url(), days, env)
    }
}

/// The `/weather` request for a location: the point at four decimals, the local today and the end
/// of the span, all in UTC.
///
/// `last_date` is the start of the day *after* the last requested one because the API treats it as
/// an inclusive instant (see the module note); `span` never drops below one so a current-only run
/// (`days == 0`) still gets a day of rows to answer from.
fn weather_request(loc: &Location, today: NaiveDate, days: u8) -> Result<HttpRequest> {
    let span = u64::from(days.max(1));
    let last = today
        .checked_add_days(Days::new(span))
        .ok_or_else(|| upstream("the requested date range overflows the calendar"))?;
    Ok(HttpRequest::get(BASE)
        .query("lat", format!("{:.4}", loc.lat))
        .query("lon", format!("{:.4}", loc.lon))
        .query("date", today.format("%Y-%m-%d").to_string())
        .query("last_date", last.format("%Y-%m-%d").to_string())
        .query("tz", "UTC"))
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// The `/weather` response, in the subset `cirrocast` consumes.
///
/// Unknown fields are ignored on purpose: Bright Sky adds parameters to the payload without notice.
#[derive(Debug, Clone, Deserialize)]
pub struct WeatherResponse {
    /// The hourly rows, oldest first.
    #[serde(default)]
    pub weather: Vec<Record>,
    /// The DWD stations the rows were drawn from.
    #[serde(default)]
    pub sources: Vec<Source>,
}

/// One hourly row.
///
/// Every meteorological value is optional: upstream reports `null` for a reading it has no model
/// for, and that must never become a zero.
#[derive(Debug, Clone, Deserialize)]
pub struct Record {
    /// The record's instant, UTC (`2026-10-06T00:00:00+00:00`).
    pub timestamp: String,
    /// Air temperature at 2 m, °C (the API's default `dwd` units).
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Wind speed at 10 m, km/h.
    #[serde(default)]
    pub wind_speed: Option<f32>,
    /// Direction the wind blows from, degrees.
    #[serde(default)]
    pub wind_direction: Option<f32>,
    /// Gust speed at 10 m, km/h.
    #[serde(default)]
    pub wind_gust_speed: Option<f32>,
    /// Relative humidity, percent.
    #[serde(default)]
    pub relative_humidity: Option<f32>,
    /// Precipitation in the preceding interval, mm.
    #[serde(default)]
    pub precipitation: Option<f32>,
    /// The precipitation/obscuration state; see [`CONDITIONS`].
    #[serde(default)]
    pub condition: Option<String>,
    /// The DWD icon alias; see [`ICONS`].
    #[serde(default)]
    pub icon: Option<String>,
    /// Horizontal visibility, m.
    #[serde(default)]
    pub visibility: Option<f32>,
    /// Total cloud cover, percent.
    #[serde(default)]
    pub cloud_cover: Option<f32>,
    /// Atmospheric pressure at mean sea level, hPa.
    #[serde(default)]
    pub pressure_msl: Option<f32>,
}

/// One station in the `sources[]` block, in the subset `cirrocast` reads for `-v`.
#[derive(Debug, Clone, Deserialize)]
pub struct Source {
    /// Bright Sky's internal source id.
    pub id: i64,
    /// DWD weather station id, typically five digits.
    #[serde(default)]
    pub dwd_station_id: Option<String>,
    /// WMO station id.
    #[serde(default)]
    pub wmo_station_id: Option<String>,
    /// Human readable station name.
    #[serde(default)]
    pub station_name: Option<String>,
    /// How the source produced its rows (`forecast`, `current`, `historical`, `synop`).
    #[serde(default)]
    pub observation_type: Option<String>,
    /// Distance of the station from the requested point, in metres.
    #[serde(default)]
    pub distance: Option<f64>,
    /// Latest instant the source can answer for, i.e. the horizon marker.
    #[serde(default)]
    pub last_record: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Response → canonical model
// ---------------------------------------------------------------------------------------------

/// Turns one response into a [`Report`].
fn report(
    response: &WeatherResponse,
    loc: &Location,
    url: String,
    days: u8,
    env: &Env<'_>,
) -> Result<Report> {
    let tz = loc.tz;
    let samples = samples(response, tz, env.verbose > 0)?;
    if samples.is_empty() {
        return Err(upstream("the response has no usable forecast row"));
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
            ProviderId::BrightSky,
            url,
            env.cache.clock().now().into(),
            (env.verbose > 0).then(|| raw_summary(response)),
        ),
    })
}

/// The usable rows, in canonical units and the location's zone.
///
/// A row is dropped when a value the canonical model has no `Option` for is missing (temperature,
/// wind speed, precipitation, a resolvable condition): a hole must not become a zero.
pub fn samples(response: &WeatherResponse, tz: Tz, verbose: bool) -> Result<Vec<HourSample>> {
    let mut samples = Vec::new();
    for record in &response.weather {
        if let Some(sample) = sample(record, tz, verbose)? {
            samples.push(sample);
        }
    }
    samples.sort_by_key(|sample| sample.at);
    Ok(samples)
}

/// One row as a sample, or `None` when it lacks a value the canonical model requires.
fn sample(record: &Record, tz: Tz, verbose: bool) -> Result<Option<HourSample>> {
    let at = instant(&record.timestamp, tz)?;
    let (Some(temp_c), Some(wind_kmh), Some(precip_mm)) =
        (record.temperature, record.wind_speed, record.precipitation)
    else {
        return Ok(None);
    };
    let Some(weather) = condition_of(record.condition.as_deref(), record.icon.as_deref(), verbose)
    else {
        return Ok(None);
    };

    Ok(Some(HourSample {
        at,
        temp_c,
        // The payload carries no apparent temperature.
        feels_like_c: None,
        precip_mm,
        // ... nor a precipitation probability.
        precip_prob_pct: None,
        weather,
        wind_kmh,
        wind_dir_deg: record.wind_direction.map(degrees),
        humidity_pct: record.relative_humidity.map(percent),
        visibility_km: record.visibility.map(metres_to_km),
    }))
}

/// The current conditions: the most recent row whose instant has already passed, else the first.
///
/// `None` when that row lacks a value the canonical `Current` requires (temperature, wind,
/// precipitation, pressure, a resolvable condition) — the renderers handle a missing current block,
/// and a zero would be a lie.
fn current_of(response: &WeatherResponse, tz: Tz, env: &Env<'_>) -> Option<Current> {
    let now: DateTime<Utc> = env.cache.clock().now().into();
    let record = response
        .weather
        .iter()
        .filter_map(|record| instant(&record.timestamp, tz).ok().map(|at| (at, record)))
        .filter(|(at, _)| at.with_timezone(&Utc) <= now)
        .max_by_key(|(at, _)| *at)
        .or_else(|| {
            response
                .weather
                .first()
                .and_then(|record| instant(&record.timestamp, tz).ok().map(|at| (at, record)))
        })?;
    let (at, record) = record;

    Some(Current {
        observed_at: at.fixed_offset(),
        temp_c: record.temperature?,
        feels_like_c: None,
        humidity_pct: record.relative_humidity.map(percent),
        precip_mm: record.precipitation?,
        weather: condition_of(
            record.condition.as_deref(),
            record.icon.as_deref(),
            env.verbose > 0,
        )?,
        cloud_cover_pct: record.cloud_cover.map(percent),
        pressure_hpa: record.pressure_msl?,
        wind_kmh: record.wind_speed?,
        wind_dir_deg: record.wind_direction.map(degrees),
        wind_gust_kmh: record.wind_gust_speed,
        visibility_km: record.visibility.map(metres_to_km),
        uv_index: None,
        // No daylight flag in the payload: the local civil day (06:00–18:00) stands in.
        is_day: (6..18).contains(&at.hour()),
    })
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
// Condition tables
// ---------------------------------------------------------------------------------------------

/// The canonical condition for a row, or `None` when it names neither a condition nor an icon.
///
/// `condition` is the precipitation/obscuration state and wins whenever it is not `dry`; for `dry`
/// (and for a row with no `condition` at all) the sky state comes from `icon`. An unknown value
/// falls back to WMO 3 (overcast) and names itself under `--verbose`.
pub fn condition_of(
    condition: Option<&str>,
    icon: Option<&str>,
    verbose: bool,
) -> Option<Condition> {
    match condition {
        Some(name) if !name.eq_ignore_ascii_case("dry") => {
            Some(lookup(&CONDITIONS, name, "condition", verbose))
        }
        Some(_) | None => match icon {
            Some(icon) => Some(lookup(&ICONS, icon, "icon", verbose)),
            None => condition.map(|_| Condition::from_u8(0)),
        },
    }
}

/// The WMO code for `name` in `table`, or WMO 3 with a `--verbose` note when it is not listed.
fn lookup(table: &[(&str, u8)], name: &str, field: &str, verbose: bool) -> Condition {
    table
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map_or_else(
            || {
                if verbose {
                    eprintln!(
                        "note: {PROVIDER} {field} `{name}` is not in the table; reporting WMO 3 (overcast)"
                    );
                }
                Condition::from_u8(3)
            },
            |&(_, code)| Condition::from_u8(code),
        )
}

// ---------------------------------------------------------------------------------------------
// Unit helpers
// ---------------------------------------------------------------------------------------------

/// Metres → kilometres, the model's visibility unit.
fn metres_to_km(metres: f32) -> f32 {
    metres / 1000.0
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

/// What `-v` prints: the station block and the row span.
pub fn raw_summary(response: &WeatherResponse) -> String {
    let sources: Vec<String> = response
        .sources
        .iter()
        .map(|source| {
            let distance = source
                .distance
                .map_or_else(|| "?".to_owned(), |metres| format!("{metres:.0} m"));
            format!(
                "#{} {} (DWD {}, WMO {}, {}, {distance})",
                source.id,
                source.station_name.as_deref().unwrap_or("?"),
                source.dwd_station_id.as_deref().unwrap_or("?"),
                source.wmo_station_id.as_deref().unwrap_or("?"),
                source.observation_type.as_deref().unwrap_or("?"),
            )
        })
        .collect();
    let sources = if sources.is_empty() {
        "none".to_owned()
    } else {
        sources.join("; ")
    };
    format!(
        "sources: {sources}; rows {} ({}..{})",
        response.weather.len(),
        response
            .weather
            .first()
            .map_or("-", |record| record.timestamp.as_str()),
        response
            .weather
            .last()
            .map_or("-", |record| record.timestamp.as_str()),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::{CONDITIONS, ICONS, condition_of, degrees, metres_to_km};
    use crate::model::Condition;

    #[test]
    fn every_condition_and_icon_maps_to_the_documented_wmo_code() {
        // The independent expectation: the API's own two enums, written out again so the tables'
        // values are checked rather than echoed.
        let conditions: [(&str, u8); 7] = [
            ("dry", 0),
            ("fog", 45),
            ("rain", 63),
            ("sleet", 68),
            ("snow", 73),
            ("hail", 96),
            ("thunderstorm", 95),
        ];
        let icons: [(&str, u8); 12] = [
            ("clear-day", 0),
            ("clear-night", 0),
            ("partly-cloudy-day", 2),
            ("partly-cloudy-night", 2),
            ("cloudy", 3),
            ("fog", 45),
            ("wind", 3),
            ("rain", 63),
            ("sleet", 68),
            ("snow", 73),
            ("hail", 96),
            ("thunderstorm", 95),
        ];
        assert_eq!(CONDITIONS.len(), conditions.len(), "the condition enum");
        assert_eq!(ICONS.len(), icons.len(), "the icon enum");
        for (name, code) in conditions {
            assert_eq!(
                condition_of(Some(name), None, false).map(Condition::code),
                Some(code),
                "condition `{name}` must map to WMO {code}"
            );
        }
        for (name, code) in icons {
            assert_eq!(
                condition_of(None, Some(name), false).map(Condition::code),
                Some(code),
                "icon `{name}` must map to WMO {code}"
            );
        }
    }

    #[test]
    fn a_non_dry_condition_wins_over_the_icon_and_dry_defers_to_it() {
        assert_eq!(
            condition_of(Some("rain"), Some("partly-cloudy-day"), false),
            Some(Condition::from_u8(63))
        );
        assert_eq!(
            condition_of(Some("dry"), Some("cloudy"), false),
            Some(Condition::from_u8(3)),
            "`dry` takes its sky state from the icon"
        );
        assert_eq!(
            condition_of(Some("dry"), None, false),
            Some(Condition::from_u8(0))
        );
        assert_eq!(condition_of(None, None, false), None, "no state at all");
    }

    #[test]
    fn unknown_values_fall_back_to_overcast() {
        assert_eq!(
            condition_of(Some("hailstorm"), None, false),
            Some(Condition::from_u8(3))
        );
        assert_eq!(
            condition_of(Some("dry"), Some("sunny"), false),
            Some(Condition::from_u8(3))
        );
    }

    #[test]
    fn units_convert_to_the_canonical_scale() {
        assert_eq!(metres_to_km(33_300.0), 33.3);
        assert_eq!(degrees(238.0), 238);
        assert_eq!(degrees(359.7), 0, "rounding happens before the wrap");
    }
}
