// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The US National Weather Service backend (`api.weather.gov`): keyless, US and territories.
//!
//! Fetching is a **two-step** walk, like a station lookup: the coordinate goes to
//! `/points/{lat},{lon}` first, which answers the office and grid cell
//! (`properties.{gridId, gridX, gridY}`) plus the location's zone and its nearest place; only then
//! does the grid cell's `forecast/hourly` series and its `forecast` day/night periods get fetched.
//! The point → grid mapping changes far more slowly than a forecast, so it is cached under the
//! `grid/` namespace with a 30-day TTL: a repeated run pays for the two forecast resources instead
//! of the mapping again.
//!
//! What this module knows that the payload does not spell out:
//!
//! 1. **`temperatureUnit` is read per period.** A US point answers Fahrenheit; the value is
//!    converted to °C here, at decode time, so the model and the cache stay metric.
//! 2. **`windSpeed` is a string, and it can be a range.** `"10 mph"` is one value; `"5 to 10 mph"`
//!    is two, and the decoder keeps the **upper bound** (the more significant sustained speed),
//!    emitting one `-v` note that names the range it collapsed. A missing/calm string is `0`.
//! 3. **`windDirection` is a cardinal**, not degrees; it maps back to the sector centre through
//!    [`crate::model::units::compass_degrees`], the shared table's inverse.
//! 4. **The percentage fields are objects.** `probabilityOfPrecipitation` and `relativeHumidity`
//!    are `{unitCode, value}` with a nullable `value`; a `null` stays absent, never `0`.
//! 5. **`shortForecast`/`icon` map to WMO through written-out tables.** The icon code (with the
//!    `land`/`marine` and `day`/`night` path segments stripped) is consulted first and the most
//!    severe code wins; the forecast text is the fallback. An unknown pair is WMO 3 (overcast)
//!    plus one `-v` line.
//! 6. **`properties.timeZone` repairs a provisional zone.** A coordinate or OSM place arrives with
//!    a placeholder `UTC`; the point answer replaces it before the day parts are anchored.
//! 7. **A point outside the coverage answers `404`.** That stays [`Error::Upstream`] (exit 3) and
//!    names the point, so an `auto` chain falls through with a reason the user can read.
//!
//! Attribution: `api.weather.gov` serves US government work in the public domain; the registry row
//! carries the credit line the renderers print.

use std::time::Duration;

use chrono::{DateTime, NaiveDate};
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
use crate::model::units::compass_degrees;
use crate::model::{Condition, Location, Report, ReportMode};

/// The provider id, as the registry and every error message spell it.
const PROVIDER: &str = "nws";

/// The point-lookup endpoint; `/points/{lat},{lon}`.
pub const POINTS_BASE: &str = "https://api.weather.gov/points";

/// The grid-cell endpoint root; the resources hang off `/{office}/{x},{y}`.
pub const GRIDPOINTS_BASE: &str = "https://api.weather.gov/gridpoints";

/// How long a point → grid mapping stays fresh: grids do not move, so a month is generous.
pub const GRID_TTL_SECS: u64 = 30 * 24 * 60 * 60;

/// Miles per hour → km/h.
const MPH_TO_KMH: f32 = 1.609_344;

/// The NWS icon codes and the WMO 4677 code each one maps to.
///
/// These are the condition segments of an icon URL such as
/// `https://api.weather.gov/icons/land/day/rain_showers?size=small` — the `land`/`marine` and
/// `day`/`night` path segments carry no weather meaning and are stripped. An icon can name more
/// than one condition (`.../land/day/ovc/rain`), so the most severe mapped code wins.
pub const ICON_CODES: [(&str, u8); 30] = [
    ("skc", 0),
    ("wind_skc", 0),
    ("hot", 0),
    ("cold", 0),
    ("few", 1),
    ("wind_few", 1),
    ("sct", 2),
    ("wind_sct", 2),
    ("bkn", 3),
    ("wind_bkn", 3),
    ("ovc", 3),
    ("wind_ovc", 3),
    ("fog", 45),
    ("haze", 45),
    ("smoke", 45),
    ("dust", 45),
    ("blizzard", 75),
    ("snow", 71),
    ("rain_showers_hi", 81),
    ("rain_showers", 80),
    ("freezing_rain", 66),
    ("fzra", 66),
    ("sleet", 68),
    ("rain", 61),
    ("tsra", 95),
    ("tsra_sct", 95),
    ("tsra_hi", 95),
    ("tornado", 95),
    ("hurricane", 95),
    ("tropical_storm", 95),
];

/// The `shortForecast` phrases and the WMO code each one maps to.
///
/// NWS text is compositional (`"Chance Rain Showers"`, `"Patchy Fog"`, `"Mostly Cloudy"`), so the
/// entries are ordered longest/most specific first and matched as substrings of the lowercased
/// text: `"rain and snow"` must win before `"rain"`, and `"snow showers"` before `"snow"`.
pub const TEXT_CONDITIONS: [(&str, u8); 26] = [
    ("thunder", 95),
    ("tornado", 95),
    ("freezing rain", 66),
    ("rain and snow", 68),
    ("wintry mix", 68),
    ("sleet", 68),
    ("snow showers", 85),
    ("snow", 71),
    ("blizzard", 75),
    ("rain showers", 80),
    ("showers", 80),
    ("drizzle", 51),
    ("rain", 61),
    ("fog", 45),
    ("haze", 45),
    ("smoke", 45),
    ("dust", 45),
    ("overcast", 3),
    ("mostly cloudy", 3),
    ("partly cloudy", 2),
    ("cloudy", 3),
    ("partly sunny", 2),
    ("mostly sunny", 1),
    ("mostly clear", 1),
    ("sunny", 0),
    ("clear", 0),
];

/// The National Weather Service backend. Stateless: one value serves every fetch.
#[derive(Debug, Clone, Copy, Default)]
pub struct Nws;

impl Provider for Nws {
    fn id(&self) -> ProviderId {
        ProviderId::Nws
    }

    fn capabilities(&self) -> Capabilities {
        ProviderId::Nws.metadata().capabilities()
    }

    fn fetch_report(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
        // NWS serves a forecast only; the CLI refuses `--date`/`--history` for a row with
        // `history_days: 0`, and this is the backstop for a hand-built chain.
        if req.window.is_some() {
            return Err(Error::Usage(
                "nws has no archive; drop --date/--history".to_owned(),
            ));
        }

        let max_days = self.capabilities().max_days;
        let days = requested_days(req.days, max_days, PROVIDER, env.quiet);

        // Step 1: the point → grid mapping (cached), which also carries the zone and the place.
        let mapping = grid_mapping(loc, env)?;
        let tz = mapping.timezone()?;
        let mut location = loc.clone();
        if provisional_zone(loc) {
            // A coordinate or OSM place arrived with a placeholder `UTC`; the point answer names
            // the zone its forecast is expressed in, so the day parts are anchored correctly.
            location.tz = tz;
        }
        if env.verbose > 0 {
            eprintln!(
                "{PROVIDER}: grid {} ({}) zone {tz}",
                mapping.grid(),
                mapping.place().unwrap_or_else(|| "no place".to_owned())
            );
        }

        let today = local_today(env, tz);
        let ttl = Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs));
        let hourly_url = format!("{}/forecast/hourly", mapping.gridpoints_url());
        let daily_url = format!("{}/forecast", mapping.gridpoints_url());

        // Step 2: the two forecast resources, each in the `weather/` namespace under the
        // configured TTL. `days == 0` asks for current conditions only, which this payload has no
        // field for (no pressure), so no forecast resource is requested.
        let hourly = if days > 0 {
            Some(fetch_json(
                env,
                &location,
                &JsonFetch {
                    provider: ProviderId::Nws,
                    request: json_request(&hourly_url),
                    key: CacheKey::weather_part(PROVIDER, "hourly", loc.lat, loc.lon, days, today),
                    ttl,
                    what: "hourly forecast",
                },
            )?)
        } else {
            None
        };
        let daily = if days > 0 {
            Some(fetch_json(
                env,
                &location,
                &JsonFetch {
                    provider: ProviderId::Nws,
                    request: json_request(&daily_url),
                    key: CacheKey::weather_part(PROVIDER, "daily", loc.lat, loc.lon, days, today),
                    ttl,
                    what: "daily forecast",
                },
            )?)
        } else {
            None
        };

        report(
            &mapping,
            hourly.as_ref(),
            daily.as_ref(),
            &location,
            hourly_url,
            days,
            tz,
            env,
        )
    }
}

/// A NWS JSON request with the media type the API documents.
fn json_request(url: &str) -> HttpRequest {
    HttpRequest::get(url).header("Accept", "application/geo+json")
}

/// Step 1: the point → grid mapping, served from the `grid/` namespace when it is fresh.
fn grid_mapping(loc: &Location, env: &Env<'_>) -> Result<PointsResponse> {
    let point = format!("{:.4},{:.4}", loc.lat, loc.lon);
    fetch_json(
        env,
        loc,
        &JsonFetch {
            provider: ProviderId::Nws,
            request: json_request(&format!("{POINTS_BASE}/{point}")),
            key: CacheKey::grid(PROVIDER, loc.lat, loc.lon),
            ttl: Duration::from_secs(GRID_TTL_SECS),
            what: "point → grid mapping",
        },
    )
    .map_err(|error| name_the_point(error, &point))
}

/// Names the out-of-coverage point in a `404` from the point lookup.
///
/// The point lookup answers a coordinate outside the US and its territories with `404` and a JSON
/// problem body; the taxonomy keeps it [`Error::Upstream`] (exit 3) so a chain falls through, and
/// the message names the point so the user reads *why* rather than a bare status.
fn name_the_point(error: Error, point: &str) -> Error {
    match error {
        Error::Upstream {
            status: Some(404), ..
        } => Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: Some(404),
            message: format!(
                "no NWS forecast grid for point {point}; the point is outside the US and its territories"
            ),
        },
        other => other,
    }
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// The `/points` response, in the subset `cirrocast` consumes.
#[derive(Debug, Clone, Deserialize)]
pub struct PointsResponse {
    /// The point's grid identity, zone and nearest place.
    pub properties: PointsProperties,
}

/// The `/points` `properties` object.
#[derive(Debug, Clone, Deserialize)]
pub struct PointsProperties {
    /// The forecast office identifier, e.g. `TOP`.
    #[serde(rename = "gridId")]
    pub grid_id: String,
    /// The cell's x index within the office.
    #[serde(rename = "gridX")]
    pub grid_x: i64,
    /// The cell's y index within the office.
    #[serde(rename = "gridY")]
    pub grid_y: i64,
    /// The IANA zone the office's forecast is expressed in.
    #[serde(rename = "timeZone")]
    pub time_zone: String,
    /// The nearest populated place, when upstream reports one.
    #[serde(default, rename = "relativeLocation")]
    pub relative_location: Option<RelativeLocation>,
}

/// The `relativeLocation` feature, in the subset this backend reads.
#[derive(Debug, Clone, Deserialize)]
pub struct RelativeLocation {
    /// The feature's `properties`.
    #[serde(default)]
    pub properties: Option<RelativeLocationProperties>,
}

/// The nearest place's `city`/`state`.
#[derive(Debug, Clone, Deserialize)]
pub struct RelativeLocationProperties {
    /// Nearest city.
    #[serde(default)]
    pub city: Option<String>,
    /// Nearest state (or territory) code.
    #[serde(default)]
    pub state: Option<String>,
}

impl PointsResponse {
    /// The grid identity as one `-v` token, e.g. `TOP 32,81`.
    #[must_use]
    pub fn grid(&self) -> String {
        format!(
            "{} {},{}",
            self.properties.grid_id, self.properties.grid_x, self.properties.grid_y
        )
    }

    /// The grid cell's endpoint root, e.g. `https://api.weather.gov/gridpoints/TOP/32,81`.
    #[must_use]
    pub fn gridpoints_url(&self) -> String {
        format!(
            "{GRIDPOINTS_BASE}/{}/{},{}",
            self.properties.grid_id, self.properties.grid_x, self.properties.grid_y
        )
    }

    /// The nearest place as `City, ST`, when upstream reports one.
    #[must_use]
    pub fn place(&self) -> Option<String> {
        let properties = self
            .properties
            .relative_location
            .as_ref()?
            .properties
            .as_ref()?;
        let city = properties.city.as_deref()?.trim();
        match properties.state.as_deref().map(str::trim) {
            Some(state) if !state.is_empty() => Some(format!("{city}, {state}")),
            _ => Some(city.to_owned()),
        }
    }

    /// The zone the forecast is expressed in.
    fn timezone(&self) -> Result<Tz> {
        self.properties
            .time_zone
            .parse::<Tz>()
            .map_err(|_| Error::Upstream {
                provider: PROVIDER.to_owned(),
                status: None,
                message: format!("`{}` is not a known time zone", self.properties.time_zone),
            })
    }
}

/// The `forecast/hourly` response, in the subset `cirrocast` consumes.
#[derive(Debug, Clone, Deserialize)]
pub struct HourlyResponse {
    /// The hourly periods.
    pub properties: Periods,
}

/// The `forecast` response, in the subset `cirrocast` consumes.
#[derive(Debug, Clone, Deserialize)]
pub struct DailyResponse {
    /// The day/night periods.
    pub properties: Periods,
}

/// A `properties` object carrying forecast periods.
#[derive(Debug, Clone, Deserialize)]
pub struct Periods {
    /// The periods, oldest first.
    #[serde(default)]
    pub periods: Vec<Period>,
}

/// One forecast period, shared by the hourly and daily resources.
///
/// The daily periods repeat every field but omit `relativeHumidity`, so every field the hourly
/// series alone carries is optional here.
#[derive(Debug, Clone, Deserialize)]
pub struct Period {
    /// The period's start instant, e.g. `2026-10-05T17:00:00-05:00`.
    #[serde(rename = "startTime")]
    pub start_time: String,
    /// The period's end instant.
    #[serde(rename = "endTime")]
    pub end_time: String,
    /// The period's temperature in `temperature_unit`.
    pub temperature: f32,
    /// `F` or `C`; read, never assumed.
    #[serde(rename = "temperatureUnit")]
    pub temperature_unit: String,
    /// The period's precipitation probability, nullable.
    #[serde(default, rename = "probabilityOfPrecipitation")]
    pub probability_of_precipitation: Option<MeasuredValue>,
    /// The period's relative humidity, nullable (and absent on daily periods).
    #[serde(default, rename = "relativeHumidity")]
    pub relative_humidity: Option<MeasuredValue>,
    /// The wind speed as a string, e.g. `10 mph` or `5 to 10 mph`.
    #[serde(default, rename = "windSpeed")]
    pub wind_speed: Option<String>,
    /// The wind direction as a cardinal, e.g. `SSW` (empty for a variable wind).
    #[serde(default, rename = "windDirection")]
    pub wind_direction: Option<String>,
    /// The short human forecast text, the icon table's fallback key.
    #[serde(default, rename = "shortForecast")]
    pub short_forecast: String,
    /// The icon URL whose condition segments map to a WMO code.
    #[serde(default)]
    pub icon: String,
}

/// A `{unitCode, value}` object with a nullable value.
#[derive(Debug, Clone, Deserialize)]
pub struct MeasuredValue {
    /// The reading; `None` is upstream's "no value", which stays absent.
    #[serde(default)]
    pub value: Option<f32>,
}

// ---------------------------------------------------------------------------------------------
// Response → canonical model
// ---------------------------------------------------------------------------------------------

/// Turns the mapping and the two resources into a [`Report`].
#[allow(clippy::too_many_arguments)] // the two resources, the mapping and the run context
fn report(
    mapping: &PointsResponse,
    hourly: Option<&HourlyResponse>,
    daily: Option<&DailyResponse>,
    loc: &Location,
    url: String,
    days: u8,
    tz: Tz,
    env: &Env<'_>,
) -> Result<Report> {
    let mut forecasts = Vec::new();
    if days > 0 {
        let hourly = hourly.ok_or_else(|| upstream("the hourly forecast is missing"))?;
        let daily = daily.ok_or_else(|| upstream("the daily forecast is missing"))?;
        let hours = samples(hourly, tz, env.verbose > 0 && !env.quiet)?;
        if hours.is_empty() {
            return Err(upstream("the hourly forecast has no usable period"));
        }
        for date in covered_days(&hours, tz, days) {
            // NWS publishes a daily high/low pair per day; a date the daily block does not reach
            // falls back to the hourly samples' own extremes rather than dropping the day.
            let (temp_min_c, temp_max_c) = match daily_extremes(daily, date, tz)? {
                Some(extremes) => extremes,
                None => extremes(&hours, date, tz, PROVIDER)?,
            };
            forecasts.push(aggregate_day(
                &hours, date, tz, PROVIDER, temp_min_c, temp_max_c, None, None,
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
        // The forecast payload carries no pressure, so the canonical `Current` block cannot be
        // filled honestly; the row declares `current: false` and the renderers omit the block.
        current: None,
        days: forecasts,
        alerts: Vec::new(),
        air: None,
        astro: None,
        marine: None,
        normals: None,
        mode: ReportMode::Forecast,
        attribution: attribution(
            ProviderId::Nws,
            url,
            env.cache.clock().now().into(),
            (env.verbose > 0).then(|| raw_summary(mapping, hourly)),
        ),
    })
}

/// The usable hourly periods, in canonical units and the location's zone.
///
/// `verbose` prints the decode notes (a collapsed wind range, an unknown forecast text) when the
/// run asked for them.
pub fn samples(response: &HourlyResponse, tz: Tz, verbose: bool) -> Result<Vec<HourSample>> {
    let mut samples = Vec::with_capacity(response.properties.periods.len());
    for period in &response.properties.periods {
        let at = instant(&period.start_time, tz)?;
        let temp_c = temperature_c(period.temperature, &period.temperature_unit);
        let (wind_kmh, note) = wind_kmh(period.wind_speed.as_deref().unwrap_or(""));
        if let Some(note) = note
            && verbose
        {
            eprintln!("note: {PROVIDER} {note}");
        }
        samples.push(HourSample {
            at,
            temp_c,
            // The forecast payload carries no apparent temperature.
            feels_like_c: None,
            // ... nor a quantitative precipitation forecast; the probability is all it offers, and
            // a zero amount would look like a measurement, so it stays 0 with the probability
            // beside it.
            precip_mm: 0.0,
            precip_prob_pct: period
                .probability_of_precipitation
                .as_ref()
                .and_then(|value| value.value)
                .map(percent),
            weather: condition_of(&period.short_forecast, &period.icon, verbose),
            wind_kmh,
            wind_dir_deg: period.wind_direction.as_deref().and_then(compass_degrees),
            humidity_pct: period
                .relative_humidity
                .as_ref()
                .and_then(|value| value.value)
                .map(percent),
            // ... nor a visibility.
            visibility_km: None,
        });
    }
    samples.sort_by_key(|sample| sample.at);
    Ok(samples)
}

/// The daily high/low of one local date, from the day/night periods that **end** on it.
///
/// The end instant is what anchors a period to the canonical day: NWS's `Tonight` period runs from
/// the evening of one day into the small hours of the next, and the canonical day's `Night` part is
/// those small hours, so the low belongs to the date the period ends on. `None` when the daily
/// block carries no period ending on `date`.
fn daily_extremes(response: &DailyResponse, date: NaiveDate, tz: Tz) -> Result<Option<(f32, f32)>> {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    let mut found = false;
    for period in &response.properties.periods {
        if instant(&period.end_time, tz)?.date_naive() != date {
            continue;
        }
        let temp_c = temperature_c(period.temperature, &period.temperature_unit);
        min = min.min(temp_c);
        max = max.max(temp_c);
        found = true;
    }
    Ok(found.then_some((min, max)))
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

/// An upstream failure with no HTTP status of its own.
fn upstream(message: impl Into<String>) -> Error {
    Error::Upstream {
        provider: PROVIDER.to_owned(),
        status: None,
        message: message.into(),
    }
}

// ---------------------------------------------------------------------------------------------
// Decoding rules
// ---------------------------------------------------------------------------------------------

/// The km/h a NWS `windSpeed` string stands for, with a `-v` note when a range was collapsed.
///
/// NWS reports a range as one string (`"5 to 10 mph"`); the **upper bound** is kept (the more
/// significant sustained speed) and the note names the range. A string with no number (a calm or
/// missing hour) is `0`.
#[must_use]
pub fn wind_kmh(text: &str) -> (f32, Option<String>) {
    let numbers: Vec<f32> = text
        .split_ascii_whitespace()
        .filter_map(|token| token.parse::<f32>().ok())
        .collect();
    let Some(upper) = numbers.iter().copied().reduce(f32::max) else {
        return (0.0, None);
    };
    let note = (numbers.len() > 1)
        .then(|| format!("collapsed wind range `{text}` to its upper bound ({upper} mph)"));
    (upper * MPH_TO_KMH, note)
}

/// A NWS temperature in °C.
///
/// The unit is read per period: `F` is converted here, at decode time, so the model stays metric;
/// any other spelling (`C`, or a payload that omits the unit) is already canonical.
#[must_use]
pub fn temperature_c(value: f32, unit: &str) -> f32 {
    if unit.trim().eq_ignore_ascii_case("F") {
        (value - 32.0) * 5.0 / 9.0
    } else {
        value
    }
}

/// The canonical condition for a period: icon first, then the forecast text, else WMO 3 plus a
/// `-v` line naming the unknown text.
#[must_use]
pub fn condition_of(short_forecast: &str, icon: &str, verbose: bool) -> Condition {
    if let Some(condition) = icon_condition(icon) {
        return condition;
    }
    if let Some(condition) = text_condition(short_forecast) {
        return condition;
    }
    if verbose {
        eprintln!(
            "note: {PROVIDER} shortForecast `{short_forecast}` is not in the table; reporting WMO 3 (overcast)"
        );
    }
    Condition::from_u8(3)
}

/// The most severe WMO condition an icon URL names, or `None` when none of its codes is mapped.
#[must_use]
pub fn icon_condition(icon: &str) -> Option<Condition> {
    let path = icon.split('?').next().unwrap_or(icon);
    let mut segments = path.split('/');
    // The condition codes follow the `land`/`marine` and `day`/`night` markers.
    segments.find(|segment| matches!(*segment, "day" | "night"))?;
    segments
        .filter_map(|code| {
            ICON_CODES
                .iter()
                .find(|(name, _)| *name == code)
                .map(|&(_, wmo)| Condition::from_u8(wmo))
        })
        .reduce(|best, candidate| {
            if candidate.severity_rank() > best.severity_rank() {
                candidate
            } else {
                best
            }
        })
}

/// The WMO condition a `shortForecast` text names, or `None` when no keyword matches.
#[must_use]
pub fn text_condition(short_forecast: &str) -> Option<Condition> {
    let text = short_forecast.trim().to_ascii_lowercase();
    TEXT_CONDITIONS
        .iter()
        .find(|(needle, _)| text.contains(needle))
        .map(|&(_, wmo)| Condition::from_u8(wmo))
}

/// A percentage from upstream, clamped into the model's `u8`.
fn percent(value: f32) -> u8 {
    clamped_u8(value, 100)
}

/// Rounds an upstream value and clamps it into `0..=max`.
///
/// The casts are safe by construction: the clamp runs first, so the value is an integral `f32`
/// inside the target range.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn clamped_u8(value: f32, max: u8) -> u8 {
    value.round().clamp(0.0, f32::from(max)) as u8
}

/// What `-v` prints: the grid and place, the zone, and the hourly period count.
fn raw_summary(mapping: &PointsResponse, hourly: Option<&HourlyResponse>) -> String {
    format!(
        "grid {}; zone {}; {} hourly periods",
        mapping.grid(),
        mapping.properties.time_zone,
        hourly.map_or(0, |response| response.properties.periods.len())
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::{temperature_c, wind_kmh};

    #[test]
    fn a_wind_range_keeps_its_upper_bound_and_names_itself() {
        let (single, note) = wind_kmh("10 mph");
        assert!((single - 16.093_44).abs() < 0.001);
        assert!(note.is_none());

        let (range, note) = wind_kmh("5 to 10 mph");
        assert!((range - 16.093_44).abs() < 0.001);
        let note = note.expect("a range is reported");
        assert!(note.contains("5 to 10 mph"), "{note}");
    }

    #[test]
    fn a_missing_wind_is_calm() {
        assert_eq!(wind_kmh(""), (0.0, None));
        assert_eq!(wind_kmh("0 mph"), (0.0, None));
    }

    #[test]
    fn fahrenheit_converts_and_celsius_stays() {
        assert!((temperature_c(76.0, "F") - 24.444_445).abs() < 0.001);
        assert_eq!(temperature_c(20.0, "C"), 20.0);
    }
}
