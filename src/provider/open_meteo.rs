// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Open-Meteo: the keyless default backend.
//!
//! One request per fetch, asking for metric units explicitly, and one job after the response
//! arrives: turn hourly data into the four canonical day parts **in the location's own time zone**,
//! because that is where the day boundaries a user sees actually are.
//!
//! The written rules the aggregation follows (they are choices, not facts upstream hands us):
//!
//! 1. `Morning` is 06:00–11:59, `Noon` 12:00–17:59, `Evening` 18:00–23:59 and `Night` 00:00–05:59
//!    **of the same local date**; the night row is that day's small hours, not the following
//!    night.
//! 2. Temperature, apparent temperature, humidity, wind, wind direction and visibility come from
//!    the single sample closest to the part's midpoint (09:00, 15:00, 21:00, 03:00; a tie takes
//!    the earlier hour).
//! 3. Precipitation is the part **sum**, precipitation probability the part **maximum**.
//! 4. The part's condition is the code with the highest [`Condition::severity_rank`] present,
//!    ties broken by the higher number of hours and then by the earlier hour; a part in which every
//!    code is undescribed falls back to the representative sample's code.
//! 5. A part with no hourly sample at all is an upstream error (the response cannot be rendered
//!    into the canonical shape); a part with fewer than six samples is aggregated normally, which
//!    is what a daylight-saving transition produces.
//!
//! Nothing here is per-model: Open-Meteo's `weather_code` is already a WMO 4677 code, but it still
//! goes through [`Condition::from_u8`] so an undescribed code stays undescribed instead of being
//! clamped into a neighbour.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use chrono_tz::Tz;
use serde::Deserialize;

use super::dayparts::{HourSample, aggregate_day};
use super::{
    Capabilities, Env, FetchRequest, JsonFetch, Provider, ProviderId, attribution, fetch_json,
    local_today, note_short_series, requested_days,
};
use crate::cache::CacheKey;
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::{
    Condition, Current, DayForecast, Location, LocationSource, Report, ReportMode, resolve_local,
};

/// The provider id, as the registry and every error message spell it.
const PROVIDER: &str = "open-meteo";

/// The forecast endpoint.
pub const BASE: &str = "https://api.open-meteo.com/v1/forecast";

/// Current-condition variables, in the fixed order the request uses.
const CURRENT_VARIABLES: &str = "temperature_2m,relative_humidity_2m,apparent_temperature,is_day,\
precipitation,weather_code,cloud_cover,pressure_msl,surface_pressure,wind_speed_10m,\
wind_direction_10m,wind_gusts_10m,visibility,uv_index";

/// Hourly variables, in the fixed order the request uses.
const HOURLY_VARIABLES: &str = "temperature_2m,apparent_temperature,precipitation_probability,\
precipitation,weather_code,wind_speed_10m,wind_direction_10m,relative_humidity_2m,visibility";

/// Daily variables, in the fixed order the request uses.
const DAILY_VARIABLES: &str = "weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset";

/// The Open-Meteo backend. Stateless: one value serves every fetch.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenMeteo;

impl Provider for OpenMeteo {
    fn id(&self) -> ProviderId {
        ProviderId::OpenMeteo
    }

    fn capabilities(&self) -> Capabilities {
        ProviderId::OpenMeteo.metadata().capabilities()
    }

    fn fetch_report(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
        let max_days = self.capabilities().max_days;
        let days = requested_days(req.days, max_days, PROVIDER, env.quiet);
        let local_today = local_today(env, loc.tz);
        let key = CacheKey::weather(PROVIDER, loc.lat, loc.lon, days, local_today);
        let request = forecast_request(loc, days);
        let ttl = Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs));

        let response: ForecastResponse = fetch_json(
            env,
            loc,
            &JsonFetch {
                provider: ProviderId::OpenMeteo,
                request: request.clone(),
                key,
                ttl,
                what: "forecast",
            },
        )?;

        report(&response, loc, request.redacted_url(), days, env)
    }
}

/// The request, assembled in a fixed parameter order (the tests assert the URL verbatim).
fn forecast_request(loc: &Location, days: u8) -> HttpRequest {
    let mut request = HttpRequest::get(BASE)
        .query("latitude", format!("{:.4}", loc.lat))
        .query("longitude", format!("{:.4}", loc.lon))
        .query("current", CURRENT_VARIABLES);
    if days > 0 {
        request = request
            .query("hourly", HOURLY_VARIABLES)
            .query("daily", DAILY_VARIABLES)
            .query("forecast_days", days.to_string());
    }
    request
        .query("timezone", "auto")
        .query("temperature_unit", "celsius")
        .query("wind_speed_unit", "kmh")
        .query("precipitation_unit", "mm")
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// The forecast response, in the subset `cirrocast` consumes.
///
/// Unknown fields are ignored on purpose: upstream adds variables and blocks regularly, and a new
/// one never invalidates a cached body.
#[derive(Debug, Clone, Deserialize)]
pub struct ForecastResponse {
    /// Latitude of the grid cell upstream answered for.
    pub latitude: f64,
    /// Longitude of the grid cell upstream answered for.
    pub longitude: f64,
    /// IANA zone name the timestamps are expressed in.
    pub timezone: String,
    /// Current conditions.
    #[serde(default)]
    pub current: Option<CurrentBlock>,
    /// Hourly series.
    #[serde(default)]
    pub hourly: Option<HourlyBlock>,
    /// Daily aggregates.
    #[serde(default)]
    pub daily: Option<DailyBlock>,
}

/// The `current` object.
#[derive(Debug, Clone, Deserialize)]
pub struct CurrentBlock {
    /// Observation time, local wall clock.
    pub time: String,
    /// Air temperature in °C.
    pub temperature_2m: Option<f32>,
    /// Relative humidity in percent.
    pub relative_humidity_2m: Option<f32>,
    /// Apparent temperature in °C.
    pub apparent_temperature: Option<f32>,
    /// `1` in daylight, `0` at night.
    pub is_day: Option<f32>,
    /// Precipitation in the last interval, in mm.
    pub precipitation: Option<f32>,
    /// WMO 4677 weather code.
    pub weather_code: Option<f32>,
    /// Total cloud cover in percent.
    pub cloud_cover: Option<f32>,
    /// Sea level pressure in hPa.
    pub pressure_msl: Option<f32>,
    /// Station pressure in hPa; not rendered, kept because upstream sends it.
    #[serde(default)]
    pub surface_pressure: Option<f32>,
    /// Wind speed in km/h.
    pub wind_speed_10m: Option<f32>,
    /// Direction the wind blows from, in degrees.
    pub wind_direction_10m: Option<f32>,
    /// Gust speed in km/h.
    #[serde(default)]
    pub wind_gusts_10m: Option<f32>,
    /// Horizontal visibility in **metres**.
    #[serde(default)]
    pub visibility: Option<f32>,
    /// UV index; dimensionless, so it has no unit conversion.
    #[serde(default)]
    pub uv_index: Option<f32>,
}

/// The `hourly` object: one array per variable, all as long as `time`.
#[derive(Debug, Clone, Deserialize)]
pub struct HourlyBlock {
    /// Local wall clock timestamps.
    #[serde(default)]
    pub time: Vec<String>,
    /// Air temperature in °C.
    #[serde(default)]
    pub temperature_2m: Vec<Option<f32>>,
    /// Apparent temperature in °C.
    #[serde(default)]
    pub apparent_temperature: Vec<Option<f32>>,
    /// Precipitation probability in percent.
    #[serde(default)]
    pub precipitation_probability: Vec<Option<f32>>,
    /// Precipitation in mm.
    #[serde(default)]
    pub precipitation: Vec<Option<f32>>,
    /// WMO 4677 weather code.
    #[serde(default)]
    pub weather_code: Vec<Option<f32>>,
    /// Wind speed in km/h.
    #[serde(default)]
    pub wind_speed_10m: Vec<Option<f32>>,
    /// Direction the wind blows from, in degrees.
    #[serde(default)]
    pub wind_direction_10m: Vec<Option<f32>>,
    /// Relative humidity in percent.
    #[serde(default)]
    pub relative_humidity_2m: Vec<Option<f32>>,
    /// Horizontal visibility in **metres**.
    #[serde(default)]
    pub visibility: Vec<Option<f32>>,
}

/// The `daily` object.
#[derive(Debug, Clone, Deserialize)]
pub struct DailyBlock {
    /// Local calendar dates.
    #[serde(default)]
    pub time: Vec<String>,
    /// Daily WMO 4677 weather code; not used, the parts carry the condition.
    #[serde(default)]
    pub weather_code: Vec<Option<f32>>,
    /// Daily maximum temperature in °C.
    #[serde(default)]
    pub temperature_2m_max: Vec<Option<f32>>,
    /// Daily minimum temperature in °C.
    #[serde(default)]
    pub temperature_2m_min: Vec<Option<f32>>,
    /// Local sunrise time.
    #[serde(default)]
    pub sunrise: Vec<Option<String>>,
    /// Local sunset time.
    #[serde(default)]
    pub sunset: Vec<Option<String>>,
}

// ---------------------------------------------------------------------------------------------
// Response → canonical model
// ---------------------------------------------------------------------------------------------

/// Turns one response into a [`Report`], correcting the location's zone when it was provisional.
fn report(
    response: &ForecastResponse,
    loc: &Location,
    url: String,
    days: u8,
    env: &Env<'_>,
) -> Result<Report> {
    let tz = response_zone(response)?;
    let mut location = loc.clone();
    if matches!(
        loc.source,
        LocationSource::Coordinates | LocationSource::Osm
    ) {
        // A coordinate or an OSM place has no zone of its own (`UTC` was a placeholder); the
        // provider's answer replaces it, and the header then prints the zone the data is in.
        location.tz = tz;
    }

    let current = response
        .current
        .as_ref()
        .map(|block| current_of(block, tz))
        .transpose()?;

    let mut forecasts = Vec::new();
    if days > 0 {
        let hours = match response.hourly.as_ref() {
            Some(block) => hourly_samples(block, tz)?,
            None => Vec::new(),
        };
        let daily = response.daily.as_ref().ok_or_else(|| Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: "the response has no `daily` block".to_owned(),
        })?;
        for text in daily.time.iter().take(usize::from(days)) {
            let date = parse_date(text)?;
            forecasts.push(daily_forecast(&hours, daily, date, tz)?);
        }
        if forecasts.is_empty() {
            return Err(Error::Upstream {
                provider: PROVIDER.to_owned(),
                status: None,
                message: "the `daily` block has no days".to_owned(),
            });
        }
        note_short_series(forecasts.len(), days, PROVIDER, env);
    }

    Ok(Report {
        location,
        current,
        days: forecasts,
        alerts: Vec::new(),
        air: None,
        astro: None,
        marine: None,
        mode: ReportMode::Forecast,
        attribution: attribution(
            ProviderId::OpenMeteo,
            url,
            env.cache.clock().now().into(),
            (env.verbose > 0).then(|| raw_pairs(response)),
        ),
    })
}

/// The zone the response is expressed in.
fn response_zone(response: &ForecastResponse) -> Result<Tz> {
    response
        .timezone
        .parse::<Tz>()
        .map_err(|_| Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!("`{}` is not a known time zone", response.timezone),
        })
}

/// The current conditions, requiring every field the canonical model has no `Option` for.
fn current_of(block: &CurrentBlock, tz: Tz) -> Result<Current> {
    let observed_at = parse_local(&block.time, tz, "current")?.fixed_offset();
    Ok(Current {
        observed_at,
        temp_c: require(block.temperature_2m, "current.temperature_2m")?,
        feels_like_c: Some(require(
            block.apparent_temperature,
            "current.apparent_temperature",
        )?),
        humidity_pct: Some(percent(require(
            block.relative_humidity_2m,
            "current.relative_humidity_2m",
        )?)),
        precip_mm: require(block.precipitation, "current.precipitation")?,
        weather: condition_of(require(block.weather_code, "current.weather_code")?),
        cloud_cover_pct: Some(percent(require(block.cloud_cover, "current.cloud_cover")?)),
        pressure_hpa: require(
            block.pressure_msl.or(block.surface_pressure),
            "current.pressure_msl",
        )?,
        wind_kmh: require(block.wind_speed_10m, "current.wind_speed_10m")?,
        wind_dir_deg: Some(degrees(require(
            block.wind_direction_10m,
            "current.wind_direction_10m",
        )?)),
        wind_gust_kmh: block.wind_gusts_10m,
        // Upstream reports visibility in metres; the model stores kilometres.
        visibility_km: block.visibility.map(|metres| metres / 1000.0),
        uv_index: block.uv_index,
        is_day: require(block.is_day, "current.is_day")? >= 0.5,
    })
}

/// Decodes the hourly block into usable samples.
fn hourly_samples(block: &HourlyBlock, tz: Tz) -> Result<Vec<HourSample>> {
    let arrays: [(&str, usize); 8] = [
        ("temperature_2m", block.temperature_2m.len()),
        ("apparent_temperature", block.apparent_temperature.len()),
        ("precipitation", block.precipitation.len()),
        ("weather_code", block.weather_code.len()),
        ("wind_speed_10m", block.wind_speed_10m.len()),
        ("wind_direction_10m", block.wind_direction_10m.len()),
        ("relative_humidity_2m", block.relative_humidity_2m.len()),
        ("visibility", block.visibility.len()),
    ];
    for (name, length) in arrays {
        if length != block.time.len() {
            return Err(Error::Upstream {
                provider: PROVIDER.to_owned(),
                status: None,
                message: format!(
                    "the hourly `{name}` array holds {length} values for {} timestamps",
                    block.time.len()
                ),
            });
        }
    }

    let mut samples = Vec::with_capacity(block.time.len());
    for index in 0..block.time.len() {
        let instant = parse_local(at(&block.time, index, "time")?, tz, "hourly")?;
        let (Some(temp_c), Some(feels_like_c), Some(precip_mm), Some(wind_kmh), Some(code)) = (
            *at(&block.temperature_2m, index, "temperature_2m")?,
            *at(&block.apparent_temperature, index, "apparent_temperature")?,
            *at(&block.precipitation, index, "precipitation")?,
            *at(&block.wind_speed_10m, index, "wind_speed_10m")?,
            *at(&block.weather_code, index, "weather_code")?,
        ) else {
            // One of the values the canonical model cannot express as "missing"; the hour is
            // dropped, and a part left without any sample is reported as an upstream error.
            continue;
        };
        samples.push(HourSample {
            at: instant,
            temp_c,
            feels_like_c: Some(feels_like_c),
            precip_mm,
            // Upstream omits this array (or truncates it) when no model covers an hour; a missing
            // entry is `None`, never an error and never a zero.
            precip_prob_pct: block
                .precipitation_probability
                .get(index)
                .copied()
                .flatten()
                .map(percent),
            weather: condition_of(code),
            wind_kmh,
            wind_dir_deg: (*at(&block.wind_direction_10m, index, "wind_direction_10m")?)
                .map(degrees),
            humidity_pct: (*at(&block.relative_humidity_2m, index, "relative_humidity_2m")?)
                .map(percent),
            // Upstream reports visibility in metres; the model stores kilometres.
            visibility_km: (*at(&block.visibility, index, "visibility")?)
                .map(|metres| metres / 1000.0),
        });
    }
    Ok(samples)
}

/// Aggregates one daily entry of the response: the daily extremes and sun times come from the
/// `daily` block, the four parts from [`dayparts::aggregate_day`].
fn daily_forecast(
    hours: &[HourSample],
    daily: &DailyBlock,
    date: NaiveDate,
    tz: Tz,
) -> Result<DayForecast> {
    let index = daily
        .time
        .iter()
        .position(|text| text == &date.format("%Y-%m-%d").to_string())
        .ok_or_else(|| Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!("the `daily` block has no entry for {date}"),
        })?;

    let temp_min_c = require(
        *at(&daily.temperature_2m_min, index, "temperature_2m_min")?,
        "daily.temperature_2m_min",
    )?;
    let temp_max_c = require(
        *at(&daily.temperature_2m_max, index, "temperature_2m_max")?,
        "daily.temperature_2m_max",
    )?;

    let sunrise = event(at(&daily.sunrise, index, "sunrise")?.as_deref(), tz)?;
    let sunset = event(at(&daily.sunset, index, "sunset")?.as_deref(), tz)?;
    // Inside the polar circles upstream answers `00:00` for both instants rather than `null`; a
    // zero-length day is not a sunrise, so both become absent.
    let (sunrise, sunset) = match (sunrise, sunset) {
        (Some(rise), Some(set)) if rise == set => (None, None),
        other => other,
    };

    aggregate_day(
        hours, date, tz, PROVIDER, temp_min_c, temp_max_c, sunrise, sunset,
    )
}

/// One sunrise/sunset value: absent (`null`), or a local time in `tz`.
fn event(text: Option<&str>, tz: Tz) -> Result<Option<DateTime<chrono::FixedOffset>>> {
    match text.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => Ok(Some(parse_local(text, tz, "daily")?.fixed_offset())),
        None => Ok(None),
    }
}

/// Parses a local wall clock timestamp (`2026-09-30T12:15`) into the location's zone.
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

/// A required upstream value: `null` is an upstream error naming the field.
fn require<T>(value: Option<T>, field: &str) -> Result<T> {
    value.ok_or_else(|| Error::Upstream {
        provider: PROVIDER.to_owned(),
        status: None,
        message: format!("the response carries no `{field}`"),
    })
}

/// One value of a parallel array, with an error instead of an index panic.
fn at<'a, T>(values: &'a [T], index: usize, field: &str) -> Result<&'a T> {
    values.get(index).ok_or_else(|| Error::Upstream {
        provider: PROVIDER.to_owned(),
        status: None,
        message: format!("the `{field}` array is shorter than `time`"),
    })
}

/// A WMO code as the canonical condition; an out-of-range value stays undescribed.
///
/// A negative or oversized upstream code is not a number this model can carry, so it maps to the
/// unknown sentinel rather than being clamped into the `u8` range (`-1` must not become `0`,
/// "Clear sky").
fn condition_of(code: f32) -> Condition {
    if (0.0..=255.0).contains(&code) {
        Condition::from_u8(clamped_u8(code, 255))
    } else {
        Condition::from_u8(255)
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

/// The `(local time, weather code)` pairs the `-v` attribution keeps.
fn raw_pairs(response: &ForecastResponse) -> String {
    let pairs: Vec<(String, u8)> = response
        .hourly
        .as_ref()
        .map(|hourly| {
            hourly
                .time
                .iter()
                .cloned()
                .zip(
                    hourly
                        .weather_code
                        .iter()
                        .map(|code| code.map_or(255, |code| condition_of(code).code())),
                )
                .collect()
        })
        .unwrap_or_default();
    serde_json::to_string(&pairs).unwrap_or_else(|_| "[]".to_owned())
}

#[cfg(test)]
mod tests {
    // The aggregation compares temperatures and precipitation totals that came from literals, so
    // exact equality is the point; approximate comparison would hide a wrong sample being picked.
    #![allow(clippy::float_cmp)]

    use chrono::NaiveDateTime;

    use super::{
        BASE, CURRENT_VARIABLES, DAILY_VARIABLES, DailyBlock, DayForecast, HOURLY_VARIABLES,
        HourlyBlock, forecast_request, hourly_samples,
    };
    use crate::model::{Condition, DayPartKind, Location, LocationSource, resolve_local};
    use crate::provider::dayparts::HourSample;
    use crate::provider::requested_days;

    fn berlin() -> chrono_tz::Tz {
        chrono_tz::Tz::Europe__Berlin
    }

    fn naive(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M").expect("a local timestamp")
    }

    fn sample(text: &str, temp_c: f32, code: u8) -> HourSample {
        HourSample {
            at: resolve_local(berlin(), naive(text)).expect("a local time"),
            temp_c,
            feels_like_c: Some(temp_c - 1.0),
            precip_mm: 0.0,
            precip_prob_pct: None,
            weather: Condition::from_u8(code),
            wind_kmh: 10.0,
            wind_dir_deg: Some(270),
            humidity_pct: Some(50),
            visibility_km: Some(10.0),
        }
    }

    /// One full local day, `temp_c` equal to the local hour so a test can see which sample was
    /// picked.
    fn day_hours(date: &str) -> Vec<HourSample> {
        (0..24_u8)
            .map(|hour| sample(&format!("{date}T{hour:02}:00"), f32::from(hour), 1))
            .collect()
    }

    fn daily(date: &str) -> DailyBlock {
        DailyBlock {
            time: vec![date.to_owned()],
            weather_code: vec![Some(1.0)],
            temperature_2m_max: vec![Some(30.0)],
            temperature_2m_min: vec![Some(20.0)],
            sunrise: vec![Some(format!("{date}T06:00"))],
            sunset: vec![Some(format!("{date}T18:00"))],
        }
    }

    fn aggregate(
        date: &str,
        hours: &[HourSample],
        daily: &DailyBlock,
    ) -> super::Result<DayForecast> {
        super::daily_forecast(
            hours,
            daily,
            chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").expect("a date"),
            berlin(),
        )
    }

    fn location() -> Location {
        Location {
            name: "Berlin".to_owned(),
            admin1: None,
            country: "Germany".to_owned(),
            country_code: Some("DE".to_owned()),
            lat: 52.52,
            lon: 13.405,
            tz: berlin(),
            elevation_m: None,
            population: None,
            source: LocationSource::Geocoder,
            station: None,
        }
    }

    fn hourly_block() -> HourlyBlock {
        HourlyBlock {
            time: vec!["2026-05-04T00:00".to_owned()],
            temperature_2m: vec![Some(1.0)],
            apparent_temperature: vec![Some(0.0)],
            precipitation_probability: vec![Some(10.0)],
            precipitation: vec![Some(0.0)],
            weather_code: vec![Some(1.0)],
            wind_speed_10m: vec![Some(5.0)],
            wind_direction_10m: vec![Some(180.0)],
            relative_humidity_2m: vec![Some(50.0)],
            visibility: vec![Some(10_000.0)],
        }
    }

    #[test]
    fn the_request_is_assembled_in_a_fixed_order_with_metric_units() {
        let request = forecast_request(&location(), 3);
        assert_eq!(request.url(), BASE);
        let names: Vec<&str> = request
            .query_pairs()
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "latitude",
                "longitude",
                "current",
                "hourly",
                "daily",
                "forecast_days",
                "timezone",
                "temperature_unit",
                "wind_speed_unit",
                "precipitation_unit",
            ]
        );
        let values: Vec<&str> = request
            .query_pairs()
            .iter()
            .map(|(_, value)| value.as_str())
            .collect();
        assert_eq!(values[0], "52.5200");
        assert_eq!(values[1], "13.4050");
        assert_eq!(values[2], CURRENT_VARIABLES);
        assert_eq!(values[3], HOURLY_VARIABLES);
        assert_eq!(values[4], DAILY_VARIABLES);
        assert_eq!(values[5], "3");
        assert_eq!(&values[6..], ["auto", "celsius", "kmh", "mm"]);
    }

    #[test]
    fn a_current_only_request_asks_for_neither_hourly_nor_daily() {
        let request = forecast_request(&location(), 0);
        let names: Vec<&str> = request
            .query_pairs()
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "latitude",
                "longitude",
                "current",
                "timezone",
                "temperature_unit",
                "wind_speed_unit",
                "precipitation_unit",
            ]
        );
    }

    #[test]
    fn the_day_cap_is_the_registry_horizon() {
        assert_eq!(
            requested_days(0, 16, "open-meteo", true),
            0,
            "zero means current only"
        );
        assert_eq!(requested_days(3, 16, "open-meteo", true), 3);
        assert_eq!(requested_days(20, 16, "open-meteo", true), 16);
    }

    #[test]
    fn every_part_samples_the_hour_closest_to_its_midpoint() {
        let hours = day_hours("2026-05-04");
        let forecast =
            aggregate("2026-05-04", &hours, &daily("2026-05-04")).expect("the day aggregates");
        for (kind, expected) in [
            (DayPartKind::Morning, 9.0),
            (DayPartKind::Noon, 15.0),
            (DayPartKind::Evening, 21.0),
            (DayPartKind::Night, 3.0),
        ] {
            let part = &forecast.parts[kind.index()];
            assert_eq!(part.kind, kind, "the parts must follow DayPartKind::ALL");
            assert_eq!(part.temp_c, expected, "{kind:?}");
        }
        assert_eq!(forecast.temp_min_c, 20.0);
        assert_eq!(forecast.temp_max_c, 30.0);
    }

    #[test]
    fn a_night_sample_is_the_small_hours_of_its_own_date() {
        let hours = day_hours("2026-05-04");
        let forecast = aggregate("2026-05-04", &hours, &daily("2026-05-04")).expect("aggregates");
        assert_eq!(forecast.parts[DayPartKind::Night.index()].temp_c, 3.0);
        assert_eq!(
            forecast.parts[DayPartKind::Night.index()].kind,
            DayPartKind::Night
        );
    }

    #[test]
    fn precipitation_is_summed_and_the_probability_is_the_maximum() {
        let mut hours = day_hours("2026-05-04");
        for hour in hours.iter_mut().take(12).skip(6) {
            hour.precip_mm = 0.5;
            hour.precip_prob_pct = Some(40);
        }
        hours[9].precip_prob_pct = Some(80);
        hours[3].precip_mm = 0.1;

        let forecast = aggregate("2026-05-04", &hours, &daily("2026-05-04")).expect("aggregates");
        let morning = &forecast.parts[DayPartKind::Morning.index()];
        assert_eq!(morning.precip_mm, 3.0);
        assert_eq!(morning.precip_prob_pct, Some(80));
        let night = &forecast.parts[DayPartKind::Night.index()];
        assert_eq!(night.precip_mm, 0.1);
        assert_eq!(
            night.precip_prob_pct, None,
            "no sample in the part has a probability"
        );
    }

    #[test]
    fn the_most_severe_code_wins_over_the_most_frequent_one() {
        let mut hours = day_hours("2026-05-04");
        for hour in hours.iter_mut().take(18).skip(12) {
            hour.weather = Condition::from_u8(51);
        }
        hours[17].weather = Condition::from_u8(61);
        let forecast = aggregate("2026-05-04", &hours, &daily("2026-05-04")).expect("aggregates");
        assert_eq!(
            forecast.parts[DayPartKind::Noon.index()].weather,
            Condition::from_u8(61)
        );
    }

    #[test]
    fn equal_severity_is_broken_by_the_higher_frequency() {
        let mut hours = day_hours("2026-05-04");
        // Clear sky (rank 1) shows up first, mainly clear (rank 1) more often.
        for hour in hours.iter_mut().take(12).skip(6) {
            hour.weather = Condition::from_u8(0);
        }
        for hour in hours.iter_mut().take(12).skip(8) {
            hour.weather = Condition::from_u8(1);
        }
        let forecast = aggregate("2026-05-04", &hours, &daily("2026-05-04")).expect("aggregates");
        assert_eq!(
            forecast.parts[DayPartKind::Morning.index()].weather,
            Condition::from_u8(1)
        );
    }

    #[test]
    fn a_part_of_undescribed_codes_falls_back_to_the_representative_sample() {
        let mut hours = day_hours("2026-05-04");
        for (index, hour) in hours.iter_mut().enumerate() {
            hour.weather = Condition::from_u8(200 + u8::try_from(index).expect("0..24 fits a u8"));
        }
        hours[9].weather = Condition::from_u8(250);
        let forecast = aggregate("2026-05-04", &hours, &daily("2026-05-04")).expect("aggregates");
        let morning = &forecast.parts[DayPartKind::Morning.index()];
        assert_eq!(morning.weather, Condition::from_u8(250));
        assert!(!morning.weather.is_known());
    }

    #[test]
    fn a_part_without_a_sample_is_an_upstream_error() {
        let hours: Vec<HourSample> = (0..6_u8)
            .map(|hour| sample(&format!("2026-05-04T{hour:02}:00"), 1.0, 1))
            .collect();
        let error = aggregate("2026-05-04", &hours, &daily("2026-05-04"))
            .expect_err("the morning has no data");
        assert!(
            error
                .to_string()
                .contains("no hourly data for 2026-05-04 morning")
        );
        assert_eq!(error.exit_code(), 3);
    }

    #[test]
    fn a_short_local_day_still_groups_every_part() {
        // 2026-03-29 is the spring-forward day in Europe/Berlin: the local 02:00 never happened.
        let hours: Vec<HourSample> = (0..24_u8)
            .filter(|hour| *hour != 2)
            .map(|hour| sample(&format!("2026-03-29T{hour:02}:00"), f32::from(hour), 1))
            .collect();
        let forecast = aggregate("2026-03-29", &hours, &daily("2026-03-29")).expect("aggregates");
        assert_eq!(
            forecast.parts[DayPartKind::Night.index()].temp_c,
            3.0,
            "the night still samples 03:00"
        );
        assert_eq!(forecast.parts[DayPartKind::Morning.index()].temp_c, 9.0);
    }

    #[test]
    fn a_daily_entry_missing_from_the_response_is_an_upstream_error() {
        let hours = day_hours("2026-05-04");
        let error = aggregate("2026-05-05", &hours, &daily("2026-05-04"))
            .expect_err("the daily block has no 2026-05-05");
        assert!(error.to_string().contains("no entry for 2026-05-05"));
    }

    #[test]
    fn the_polar_placeholder_is_not_a_sunrise() {
        let hours = day_hours("2026-01-12");
        let mut daily = daily("2026-01-12");
        daily.sunrise = vec![Some("2026-01-12T00:00".to_owned())];
        daily.sunset = vec![Some("2026-01-12T00:00".to_owned())];
        let forecast = aggregate("2026-01-12", &hours, &daily).expect("aggregates");
        assert_eq!(forecast.sunrise, None);
        assert_eq!(forecast.sunset, None);
    }

    #[test]
    fn sunrise_carries_the_location_offset() {
        let hours = day_hours("2026-05-04");
        let forecast = aggregate("2026-05-04", &hours, &daily("2026-05-04")).expect("aggregates");
        let sunrise = forecast.sunrise.expect("a sunrise");
        assert_eq!(sunrise.to_string(), "2026-05-04 06:00:00 +02:00");
        assert_eq!(
            forecast.sunset.expect("a sunset").to_string(),
            "2026-05-04 18:00:00 +02:00"
        );
    }

    #[test]
    fn a_null_required_value_drops_the_hour() {
        let mut block = hourly_block();
        block.temperature_2m = vec![None];
        assert_eq!(
            hourly_samples(&block, berlin()).expect("decodes"),
            Vec::<HourSample>::new()
        );
    }

    #[test]
    fn hourly_arrays_of_different_lengths_are_upstream_errors() {
        let mut block = hourly_block();
        block.visibility = Vec::new();
        let error = hourly_samples(&block, berlin()).expect_err("the arrays disagree");
        assert!(error.to_string().contains("visibility"));
        assert_eq!(error.exit_code(), 3);
    }

    #[test]
    fn a_short_probability_array_leaves_those_hours_without_a_probability() {
        let mut block = hourly_block();
        // Two timestamps, but only the first has a probability; upstream omits the value for a
        // model or an hour it cannot cover.
        block.time = vec!["2026-05-04T00:00".to_owned(), "2026-05-04T01:00".to_owned()];
        block.temperature_2m = vec![Some(1.0), Some(2.0)];
        block.apparent_temperature = vec![Some(0.0), Some(1.0)];
        block.precipitation = vec![Some(0.0), Some(0.0)];
        block.weather_code = vec![Some(1.0), Some(1.0)];
        block.wind_speed_10m = vec![Some(5.0), Some(5.0)];
        block.wind_direction_10m = vec![Some(180.0), Some(180.0)];
        block.relative_humidity_2m = vec![Some(50.0), Some(50.0)];
        block.visibility = vec![Some(10_000.0), Some(10_000.0)];

        let samples =
            hourly_samples(&block, berlin()).expect("a short probability array is not an error");
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].precip_prob_pct, Some(10));
        assert_eq!(samples[1].precip_prob_pct, None);
    }

    #[test]
    fn visibility_is_converted_from_metres_and_direction_is_wrapped() {
        let mut block = hourly_block();
        block.wind_direction_10m = vec![Some(365.0)];
        block.relative_humidity_2m = vec![Some(101.0)];
        block.precipitation_probability = vec![Some(-3.0)];
        let samples = hourly_samples(&block, berlin()).expect("decodes");
        assert_eq!(samples[0].visibility_km, Some(10.0));
        assert_eq!(samples[0].wind_dir_deg, Some(5));
        assert_eq!(samples[0].humidity_pct, Some(100));
        assert_eq!(samples[0].precip_prob_pct, Some(0));
    }

    #[test]
    fn an_out_of_range_weather_code_stays_undescribed() {
        // A negative code must not be clamped to 0 ("Clear sky"), nor a huge one to 255-as-a-code.
        for code in [-1.0, -0.4, 256.0, 1e9, f32::NAN] {
            let condition = super::condition_of(code);
            assert!(!condition.is_known(), "{code} must stay undescribed");
        }
        assert_ne!(super::condition_of(-1.0), Condition::from_u8(0));
    }

    #[test]
    fn a_payload_without_the_unused_metadata_fields_still_decodes() {
        // `elevation`, `utc_offset_seconds` and `timezone_abbreviation` are not read by any code,
        // so a response that omits them must still parse.
        let response: super::ForecastResponse = serde_json::from_str(
            r#"{"latitude":52.52,"longitude":13.405,"timezone":"Europe/Berlin"}"#,
        )
        .expect("a minimal payload decodes");
        assert_eq!(response.timezone, "Europe/Berlin");
    }
}
