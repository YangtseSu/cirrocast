// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pirate Weather: one call per fetch, Dark-Sky-shaped payloads.
//!
//! What this module knows that the payload does not spell out:
//!
//! 1. **The key is a path segment** (`/forecast/<key>/<lat>,<lon>`), not a query parameter, so it is
//!    marked for redaction and never appears in a log line, an error message or a cache envelope.
//! 2. **`precipAccumulation` is centimetres under `units=si`** while `precipIntensity` is mm/h of
//!    liquid water. The day parts sum the hourly intensities, so the centimetre field is
//!    deliberately **not** consumed and the scaling trap never applies here.
//! 3. **`extend=hourly` is required for the 7-day horizon**: without it the hourly block covers 48
//!    hours, and the day parts of days 3–7 could not be aggregated from samples.
//! 4. **`timezone` is an IANA name** and `offset` the numeric one, so a provisional location is
//!    repaired from the response instead of refused.
//! 5. **`-999` appears where a value is missing** and fields are sometimes absent entirely; both
//!    become `None`, never a number.
//! 6. **The `icon` strings need a fallback branch** (`none` is documented as "Not Available"), and
//!    the precipitation families are refined by the provider's own mm/h bands (0.4 / 2.5, a
//!    three-way collapse of its four bands).
//! 7. **`humidity`, `cloudCover` and `precipProbability` are 0–1 decimals**, not percentages.
//! 8. **No attribution is documented** in the docs or the terms — the credit line the renderers
//!    print names the service and its terms rather than inventing a mandated string; the terms add a
//!    warranty disclaimer and forbid circumventing the call limit with multiple accounts.

use std::time::Duration;

use chrono::{DateTime, Timelike as _, Utc};
use chrono_tz::Tz;
use serde::Deserialize;

use super::dayparts::{HourSample, aggregate_day, covered_days};
use super::{
    Capabilities, Env, FetchRequest, JsonFetch, Provider, ProviderId, attribution, fetch_json,
    local_today, requested_days,
};
use crate::cache::CacheKey;
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::{Condition, Current, DayForecast, Location, LocationSource, Report};

/// The provider id, as the registry and every error message spell it.
const PROVIDER: &str = "pirateweather";

/// The forecast host; the path continues with `<key>/<lat>,<lon>`.
pub const FORECAST_BASE: &str = "https://api.pirateweather.net/forecast";

/// Metres per second → km/h (`units=si` serves wind in m/s).
const MS_TO_KMH: f32 = 3.6;

/// The value upstream writes where a reading is missing.
const MISSING: f32 = -999.0;

/// The Pirate Weather backend. Stateless: one value serves every fetch.
#[derive(Debug, Clone, Copy, Default)]
pub struct PirateWeather;

impl Provider for PirateWeather {
    fn id(&self) -> ProviderId {
        ProviderId::PirateWeather
    }

    fn capabilities(&self) -> Capabilities {
        ProviderId::PirateWeather.metadata().capabilities()
    }

    fn fetch_report(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
        let max_days = self.capabilities().max_days;
        let days = requested_days(req.days, max_days, PROVIDER, env.quiet);
        let variable = self
            .capabilities()
            .key_env
            .ok_or_else(|| Error::Config(format!("provider `{PROVIDER}` names no key variable")))?;
        let key = env.keys.get(PROVIDER)?.ok_or_else(|| Error::MissingKey {
            provider: PROVIDER.to_owned(),
            env: variable.to_owned(),
        })?;

        let ttl = Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs));
        let request = HttpRequest::get(format!(
            "{FORECAST_BASE}/{key}/{:.4},{:.4}",
            loc.lat, loc.lon
        ))
        .query("units", "si")
        .query("exclude", "minutely,alerts")
        .query("lang", "en")
        .query("extend", "hourly")
        .secret(&key);

        let response: Forecast = fetch_json(
            env,
            loc,
            &JsonFetch {
                provider: ProviderId::PirateWeather,
                request: request.clone(),
                key: CacheKey::weather(PROVIDER, loc.lat, loc.lon, days, local_today(env, loc.tz)),
                ttl,
                what: "forecast",
            },
        )?;

        report(&response, loc, request.redacted_url(), days, env)
    }
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// The forecast response, in the subset this backend consumes.
#[derive(Debug, Clone, Deserialize)]
pub struct Forecast {
    /// IANA zone name.
    #[serde(default)]
    pub timezone: String,
    /// Offset from UTC in hours.
    #[serde(default)]
    pub offset: Option<f32>,
    /// Current conditions.
    #[serde(default)]
    pub currently: Option<Block>,
    /// Hourly series (`extend=hourly` for 168 entries).
    #[serde(default)]
    pub hourly: Option<Series>,
    /// Daily series.
    #[serde(default)]
    pub daily: Option<Series>,
    /// Provenance and units.
    #[serde(default)]
    pub flags: Option<Flags>,
}

/// A `{ data: [...] }` block.
#[derive(Debug, Clone, Deserialize)]
pub struct Series {
    /// The entries.
    #[serde(default)]
    pub data: Vec<Block>,
}

/// One `currently`, `hourly.data[]` or `daily.data[]` entry.
///
/// The daily entries use a different set of keys than the hourly ones; every field is optional so
/// one type can carry both. Upstream spells the multi-word keys in camelCase.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    /// Unix UTC seconds.
    pub time: i64,
    /// The condition icon.
    #[serde(default)]
    pub icon: Option<String>,
    /// Air temperature in °C.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Apparent temperature in °C.
    #[serde(default)]
    pub apparent_temperature: Option<f32>,
    /// Precipitation intensity in mm/h (liquid water equivalent).
    #[serde(default)]
    pub precip_intensity: Option<f32>,
    /// Precipitation probability, 0–1.
    #[serde(default)]
    pub precip_probability: Option<f32>,
    /// Relative humidity, 0–1.
    #[serde(default)]
    pub humidity: Option<f32>,
    /// Cloud cover, 0–1.
    #[serde(default)]
    pub cloud_cover: Option<f32>,
    /// Sea level pressure in hPa.
    #[serde(default)]
    pub pressure: Option<f32>,
    /// Wind speed in m/s.
    #[serde(default)]
    pub wind_speed: Option<f32>,
    /// Gust speed in m/s.
    #[serde(default)]
    pub wind_gust: Option<f32>,
    /// Direction the wind blows from, in degrees.
    #[serde(default)]
    pub wind_bearing: Option<f32>,
    /// Horizontal visibility in km (capped at 16).
    #[serde(default)]
    pub visibility: Option<f32>,
    /// UV index.
    #[serde(default)]
    pub uv_index: Option<f32>,
    /// Daily maximum temperature in °C.
    #[serde(default)]
    pub temperature_high: Option<f32>,
    /// Daily minimum temperature in °C.
    #[serde(default)]
    pub temperature_low: Option<f32>,
    /// Sunrise, unix UTC seconds.
    #[serde(default)]
    pub sunrise_time: Option<i64>,
    /// Sunset, unix UTC seconds.
    #[serde(default)]
    pub sunset_time: Option<i64>,
}

/// The `flags` object.
#[derive(Debug, Clone, Deserialize)]
pub struct Flags {
    /// The unit system upstream actually used.
    #[serde(default)]
    pub units: Option<String>,
    /// The candidate sources behind the answer.
    #[serde(default)]
    pub sources: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Response → canonical model
// ---------------------------------------------------------------------------------------------

/// Turns one response into a [`Report`], repairing a provisional zone from `timezone`.
fn report(
    response: &Forecast,
    loc: &Location,
    url: String,
    days: u8,
    env: &Env<'_>,
) -> Result<Report> {
    let tz = response
        .timezone
        .parse::<Tz>()
        .map_err(|_| Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!("`{}` is not a known time zone", response.timezone),
        })?;

    let mut location = loc.clone();
    if matches!(
        loc.source,
        LocationSource::Coordinates | LocationSource::Osm
    ) {
        location.tz = tz;
    }

    let current = response
        .currently
        .as_ref()
        .and_then(|block| current_of(block, tz));

    let forecasts = if days > 0 {
        days_of(response, tz, days)?
    } else {
        Vec::new()
    };

    Ok(Report {
        location,
        current,
        days: forecasts,
        alerts: Vec::new(),
        attribution: attribution(
            ProviderId::PirateWeather,
            url,
            env.cache.clock().now().into(),
            (env.verbose > 0).then(|| {
                format!(
                    "units {} offset {:+}h sources {}",
                    response
                        .flags
                        .as_ref()
                        .and_then(|flags| flags.units.as_deref())
                        .unwrap_or("unknown"),
                    response.offset.unwrap_or(0.0),
                    response
                        .flags
                        .as_ref()
                        .map_or(0, |flags| flags.sources.len()),
                )
            }),
        ),
    })
}

/// The daily entries as canonical days, using the hourly series for the four parts.
fn days_of(response: &Forecast, tz: Tz, days: u8) -> Result<Vec<DayForecast>> {
    let samples: Vec<HourSample> = response
        .hourly
        .as_ref()
        .map(|series| {
            series
                .data
                .iter()
                .filter_map(|block| sample(block, tz))
                .collect()
        })
        .unwrap_or_default();
    if samples.is_empty() {
        return Err(Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!("the response has no usable hourly entry in {tz}"),
        });
    }

    let daily = response.daily.as_ref().ok_or_else(|| Error::Upstream {
        provider: PROVIDER.to_owned(),
        status: None,
        message: "the response has no `daily` block".to_owned(),
    })?;

    let mut forecasts = Vec::new();
    let covered = covered_days(&samples, tz, days);
    for block in &daily.data {
        let date = local_time(block.time, tz).date_naive();
        // A day the hourly block does not fully cover (the last day of the series) cannot be
        // rendered into the canonical shape and is skipped.
        if !covered.contains(&date) {
            continue;
        }
        let (Some(temp_min_c), Some(temp_max_c)) =
            (value(block.temperature_low), value(block.temperature_high))
        else {
            continue;
        };
        forecasts.push(aggregate_day(
            &samples,
            date,
            tz,
            PROVIDER,
            temp_min_c,
            temp_max_c,
            block
                .sunrise_time
                .map(|time| local_time(time, tz).fixed_offset()),
            block
                .sunset_time
                .map(|time| local_time(time, tz).fixed_offset()),
        )?);
    }
    if forecasts.is_empty() {
        return Err(Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!("the response covers no complete local day in {tz}"),
        });
    }
    Ok(forecasts)
}

/// The current conditions.
fn current_of(block: &Block, tz: Tz) -> Option<Current> {
    let observed_at = local_time(block.time, tz);
    Some(Current {
        observed_at: observed_at.fixed_offset(),
        temp_c: value(block.temperature)?,
        feels_like_c: value(block.apparent_temperature),
        humidity_pct: value(block.humidity).map(fraction)?,
        precip_mm: value(block.precip_intensity).unwrap_or(0.0),
        weather: condition_of(
            block.icon.as_deref().unwrap_or_default(),
            block.precip_intensity,
        ),
        cloud_cover_pct: value(block.cloud_cover).map(fraction)?,
        pressure_hpa: value(block.pressure)?,
        wind_kmh: value(block.wind_speed)? * MS_TO_KMH,
        wind_dir_deg: value(block.wind_bearing).map(degrees)?,
        wind_gust_kmh: value(block.wind_gust).map(|gust| gust * MS_TO_KMH),
        visibility_km: value(block.visibility),
        uv_index: value(block.uv_index),
        is_day: daylight(block.icon.as_deref().unwrap_or_default(), observed_at),
    })
}

/// One hourly entry as a sample (its intensity is mm/h over a one-hour step, so mm per step).
fn sample(block: &Block, tz: Tz) -> Option<HourSample> {
    Some(HourSample {
        at: local_time(block.time, tz),
        temp_c: value(block.temperature)?,
        feels_like_c: value(block.apparent_temperature),
        precip_mm: value(block.precip_intensity).unwrap_or(0.0),
        precip_prob_pct: value(block.precip_probability).map(fraction),
        weather: condition_of(
            block.icon.as_deref().unwrap_or_default(),
            block.precip_intensity,
        ),
        wind_kmh: value(block.wind_speed)? * MS_TO_KMH,
        wind_dir_deg: value(block.wind_bearing).map(degrees),
        humidity_pct: value(block.humidity).map(fraction),
        visibility_km: value(block.visibility),
    })
}

/// Whether it is daylight: an explicit day/night icon wins, otherwise the local civil day.
///
/// Only the `clear-*` and `partly-cloudy-*` icons carry a suffix, so the civil-day rule covers the
/// rest (the same stand-in SMHI and WWO use until step 17 computes real sun times).
fn daylight(icon: &str, at: DateTime<Tz>) -> bool {
    if icon.ends_with("-night") {
        return false;
    }
    if icon.ends_with("-day") {
        return true;
    }
    matches!(at.hour(), 6..=17)
}

/// A unix UTC instant in the location's zone.
fn local_time(unix_seconds: i64, tz: Tz) -> DateTime<Tz> {
    let utc = DateTime::<Utc>::from_timestamp(unix_seconds, 0)
        .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default());
    utc.with_timezone(&tz)
}

/// A reading, with the `-999` sentinel turned into `None`.
fn value(raw: Option<f32>) -> Option<f32> {
    raw.filter(|value| (*value - MISSING).abs() > f32::EPSILON)
}

/// A 0–1 fraction as a whole percent.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn fraction(value: f32) -> u8 {
    (value * 100.0).round().clamp(0.0, 100.0) as u8
}

/// A direction in degrees, normalised into `0..360`.
#[allow(clippy::cast_possible_truncation)]
fn degrees(value: f32) -> u16 {
    let wrapped = (value.round() as i64).rem_euclid(360);
    u16::try_from(wrapped).unwrap_or_default()
}

/// Pirate Weather's `icon` (refined by intensity) as a WMO 4677 code.
///
/// The intensity bands are the provider's own (0.4 and 2.5 mm/h of liquid water, a three-way
/// collapse of its four documented bands); an icon outside the default set — including the
/// documented `none` — stays unknown.
fn condition_of(icon: &str, intensity: Option<f32>) -> Condition {
    let bands = |light: u8, moderate: u8, heavy: u8| match value(intensity).unwrap_or(0.0) {
        mm_h if mm_h < 0.4 => light,
        mm_h if mm_h <= 2.5 => moderate,
        _ => heavy,
    };
    Condition::from_u8(match icon {
        "clear-day" | "clear-night" => 0,
        "partly-cloudy-day" | "partly-cloudy-night" => 2,
        "cloudy" | "wind" => 3,
        "fog" => 45,
        "rain" => bands(61, 63, 65),
        "snow" => bands(71, 73, 75),
        "sleet" => bands(66, 67, 67),
        "thunderstorm" => 95,
        "hail" => 96,
        _ => 255,
    })
}

#[cfg(test)]
mod tests {
    use super::{Block, condition_of, current_of, sample};
    use crate::model::Condition;

    #[test]
    fn the_missing_sentinel_never_becomes_a_zero_percent() {
        let hourly: Block = serde_json::from_str(
            r#"{"time":0,"temperature":1.0,"humidity":-999,"cloudCover":-999,
                "precipProbability":-999,"windSpeed":1.0,"windBearing":0,"icon":"clear-day"}"#,
        )
        .expect("an hourly block");
        let hourly_sample =
            sample(&hourly, chrono_tz::Tz::UTC).expect("temperature and wind are present");
        assert_eq!(hourly_sample.humidity_pct, None);
        assert_eq!(hourly_sample.precip_prob_pct, None);

        // The current block's humidity and cloud cover are required readings: a sentinel value
        // yields no current block at all rather than an invented 0 %.
        let current: Block = serde_json::from_str(
            r#"{"time":0,"temperature":1.0,"pressure":1010.0,"windSpeed":1.0,
                "windBearing":0,"humidity":-999,"cloudCover":0.5,"icon":"clear-day"}"#,
        )
        .expect("a current block");
        assert_eq!(current_of(&current, chrono_tz::Tz::UTC), None);
    }

    #[test]
    fn the_default_icon_set_maps_to_described_conditions() {
        for icon in [
            "clear-day",
            "clear-night",
            "partly-cloudy-day",
            "partly-cloudy-night",
            "cloudy",
            "wind",
            "fog",
            "rain",
            "snow",
            "sleet",
            "thunderstorm",
        ] {
            assert!(
                condition_of(icon, Some(0.0)).is_known(),
                "{icon} maps to an undescribed condition"
            );
        }
        // `hail` is reserved upstream; it is mapped as well.
        assert!(condition_of("hail", Some(0.0)).is_known());
    }

    #[test]
    fn the_intensity_bands_refine_the_precipitation_family() {
        assert_eq!(condition_of("rain", Some(0.1)), Condition::from_u8(61));
        assert_eq!(condition_of("rain", Some(0.4)), Condition::from_u8(63));
        assert_eq!(condition_of("rain", Some(2.5)), Condition::from_u8(63));
        assert_eq!(condition_of("rain", Some(10.0)), Condition::from_u8(65));
        assert_eq!(condition_of("snow", Some(0.2)), Condition::from_u8(71));
        assert_eq!(condition_of("snow", Some(5.0)), Condition::from_u8(75));
        assert_eq!(condition_of("sleet", Some(1.0)), Condition::from_u8(67));
    }

    #[test]
    fn a_sentinel_intensity_is_treated_as_no_precipitation() {
        assert_eq!(condition_of("rain", Some(-999.0)), Condition::from_u8(61));
        assert_eq!(condition_of("rain", None), Condition::from_u8(61));
    }

    #[test]
    fn an_unlisted_icon_stays_unknown() {
        assert!(!condition_of("none", Some(0.0)).is_known());
        assert!(!condition_of("smoke", Some(0.0)).is_known());
        assert!(!condition_of("", Some(0.0)).is_known());
    }
}
