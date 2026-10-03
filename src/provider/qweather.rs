// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `QWeather`'s v1 weather API: the second key-requiring backend.
//!
//! The v1 endpoints (`/weather/v1/{current,hourly,daily}/{lat}/{lon}`) replace the deprecated city
//! v7 paths (EOL 2027) and are deliberately metric-only. Two calls per fetch — current conditions
//! and the hourly series — each cached under its own `weather/qweather-<part>-…` key.
//!
//! What this module knows that the payload does not spell out:
//!
//! 1. **Every value is a measure object** (`{"value": 10.96, "unit": "°C"}`), so the unit is checked
//!    rather than assumed: a response in another unit is an upstream error, never a silent
//!    conversion.
//! 2. **Humidity and cloud cover are 0–1 fractions**, visibility is **metres**, wind is **m/s**.
//! 3. **The hourly series is anchored to the next UTC midnight**, not to the current hour (probed
//!    at 16:40Z and 23:51Z, both starting at the following `00:00Z`), so the location-local today is
//!    usually incomplete and is skipped — the first fully covered local day is emitted, the same
//!    rule SMHI, `OpenWeatherMap` and WWO use.
//! 4. **The payload carries no time zone**, only UTC instants, so a provisional (UTC) location is
//!    refused rather than aggregated in the wrong zone.
//! 5. **The current block carries no observation time**, so `observed_at` is the fetch instant; the
//!    provider reference records this.
//! 6. **`hours` is capped at 240 and `days` at 10** (probed: 241 and 11 both answer `400`).
//! 7. **Errors are RFC 7807** (`application/problem+json`): a bad key is `401` (mapped to
//!    [`Error::InvalidKey`] by the shared helper), a bad parameter or location is `400`.
//! 8. **The host is per account** and comes from `[providers.qweather].host` (the console shows it
//!    at <https://console.qweather.com/setting>); a missing host is a configuration error naming
//!    that page, never a guessed default.
//!
//! Licence: proprietary (`QWeather` Developers License). The docs require the name `QWeather` plus
//! <https://www.qweather.com> wherever data is shown; the registry row carries the line the
//! renderers print. `GeoAPI` data must not be bulk-cached — this backend does not call `GeoAPI`.

use std::time::Duration;

use chrono::{DateTime, TimeZone as _, Timelike as _, Utc};
use chrono_tz::Tz;
use serde::Deserialize;

use super::dayparts::{HourSample, aggregate_day, covered_days, extremes};
use super::{
    Capabilities, Env, FetchRequest, JsonFetch, Provider, ProviderId, attribution, fetch_json,
    local_today, requested_days,
};
use crate::cache::CacheKey;
use crate::error::{Error, Result};
use crate::geo::provisional_zone;
use crate::http::HttpRequest;
use crate::model::{Condition, Current, Location, Report};

/// The provider id, as the registry and every error message spell it.
const PROVIDER: &str = "qweather";

/// Metres per second → km/h.
const MS_TO_KMH: f32 = 3.6;

/// The documented `hours` ceiling (a larger value answers `400`).
const MAX_HOURS: u32 = 240;

/// The `QWeather` backend. Stateless: one value serves every fetch.
#[derive(Debug, Clone, Copy, Default)]
pub struct QWeather;

impl Provider for QWeather {
    fn id(&self) -> ProviderId {
        ProviderId::QWeather
    }

    fn capabilities(&self) -> Capabilities {
        ProviderId::QWeather.metadata().capabilities()
    }

    fn fetch_report(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
        let max_days = self.capabilities().max_days;
        let days = requested_days(req.days, max_days, PROVIDER, env.quiet);
        if provisional_zone(loc) {
            return Err(Error::Usage(format!(
                "provider `{PROVIDER}` needs the location's time zone and its response carries only \
                 UTC instants: pass a place name (e.g. `cirrocast -p {PROVIDER} Beijing`) or set \
                 `location.default` instead of raw coordinates"
            )));
        }

        let host = host(env)?;
        let variable = self
            .capabilities()
            .key_env
            .ok_or_else(|| Error::Config(format!("provider `{PROVIDER}` names no key variable")))?;
        let key = env.keys.get(PROVIDER)?.ok_or_else(|| Error::MissingKey {
            provider: PROVIDER.to_owned(),
            env: variable.to_owned(),
        })?;

        let ttl = Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs));
        let today = local_today(env, loc.tz);

        let current_request = request(&host, "current", loc, &key);
        let current: CurrentResponse = fetch_json(
            env,
            loc,
            &JsonFetch {
                provider: ProviderId::QWeather,
                request: current_request.clone(),
                key: CacheKey::weather_part(PROVIDER, "current", loc.lat, loc.lon, days, today),
                ttl,
                what: "current",
            },
        )?;

        let mut url = current_request.redacted_url();
        let mut forecasts = Vec::new();
        if days > 0 {
            // The series starts at the next UTC midnight, so covering `days` location-local days
            // needs two extra UTC days of slack at either end of the offset range.
            let hours = (u32::from(days) + 2) * 24;
            let hourly_request = request(&host, "hourly", loc, &key)
                .query("hours", hours.min(MAX_HOURS).to_string());
            let hourly: HourlyResponse = fetch_json(
                env,
                loc,
                &JsonFetch {
                    provider: ProviderId::QWeather,
                    request: hourly_request.clone(),
                    key: CacheKey::weather_part(PROVIDER, "hourly", loc.lat, loc.lon, days, today),
                    ttl,
                    what: "hourly",
                },
            )?;
            url = hourly_request.redacted_url();
            forecasts = days_of(&hourly, loc.tz, days)?;
        }

        let now: DateTime<Utc> = env.cache.clock().now().into();
        Ok(Report {
            location: loc.clone(),
            current: Some(current_of(&current, now, loc.tz)?),
            days: forecasts,
            alerts: Vec::new(),
            air: None,
            attribution: attribution(
                ProviderId::QWeather,
                url,
                now,
                (env.verbose > 0).then(|| {
                    format!(
                        "host {host}; v1 metric-only measures; metadata tag {}",
                        current.metadata.tag
                    )
                }),
            ),
        })
    }
}

/// The account's API host, from `[providers.qweather].host`.
fn host(env: &Env<'_>) -> Result<String> {
    let host = env
        .config
        .providers
        .qweather
        .host
        .trim()
        .trim_end_matches('/');
    if host.is_empty() {
        return Err(Error::Config(format!(
            "provider `{PROVIDER}` needs its account API host: set providers.qweather.host \
             (see https://console.qweather.com/setting, or `cirrocast provider info {PROVIDER}`)"
        )));
    }
    validate_host(host)?;
    Ok(host.to_owned())
}

/// Whether `host` is the HTTPS account authority the `QWeather` console shows.
///
/// The key travels in the `X-QW-Api-Key` header, so a plain `http://` authority would leak it in
/// cleartext and any other authority would send it to a third party; the legacy shared domains also
/// answer `403 Invalid Host`. The authority (everything up to the first `/`) must end with
/// `.re.qweatherapi.com` and carry a non-empty account label before it.
fn validate_host(host: &str) -> Result<()> {
    let rejection = || {
        Error::Config(format!(
            "providers.qweather.host: `{host}` must be the HTTPS account host from \
             https://console.qweather.com/setting, e.g. `https://<account-id>.re.qweatherapi.com`"
        ))
    };
    let Some(rest) = host.strip_prefix("https://") else {
        return Err(rejection());
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let Some(account) = authority.strip_suffix(".re.qweatherapi.com") else {
        return Err(rejection());
    };
    if account.is_empty() {
        return Err(rejection());
    }
    Ok(())
}

/// One endpoint's URL for a location, with the key in the documented header.
fn request(host: &str, endpoint: &str, loc: &Location, key: &str) -> HttpRequest {
    HttpRequest::get(format!(
        "{host}/weather/v1/{endpoint}/{:.4}/{:.4}",
        loc.lat, loc.lon
    ))
    .query("lang", "en")
    .header("X-QW-Api-Key", key)
    .secret(key)
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// A measure object: the value and the unit upstream sent it in.
#[derive(Debug, Clone, Deserialize)]
pub struct Measure {
    /// The number.
    pub value: f32,
    /// Its unit, e.g. `°C`, `m/s`, `mm`, `hPa`, `m`.
    #[serde(default)]
    pub unit: String,
}

/// The `metadata` object every v1 response carries.
#[derive(Debug, Clone, Deserialize)]
pub struct Metadata {
    /// The response's data tag.
    #[serde(default)]
    pub tag: String,
    /// The attribution pages upstream asks to be shown.
    #[serde(default)]
    pub attributions: Vec<String>,
}

/// `/weather/v1/current`, in the subset this backend consumes.
#[derive(Debug, Clone, Deserialize)]
pub struct CurrentResponse {
    /// Provenance.
    pub metadata: Metadata,
    /// The condition.
    pub condition: ConditionBlock,
    /// Air temperature.
    pub temperature: Measure,
    /// Apparent temperature.
    #[serde(default, rename = "feelsLike")]
    pub feels_like: Option<Measure>,
    /// Relative humidity, 0–1.
    #[serde(default)]
    pub humidity: Option<f32>,
    /// Wind.
    pub wind: Wind,
    /// Gust speed.
    #[serde(default, rename = "windGust")]
    pub wind_gust: Option<Measure>,
    /// Precipitation.
    #[serde(default)]
    pub precipitation: Option<Precipitation>,
    /// Sea level pressure.
    #[serde(default)]
    pub pressure: Option<Measure>,
    /// Horizontal visibility in metres.
    #[serde(default)]
    pub visibility: Option<Measure>,
    /// Total cloud cover, 0–1.
    #[serde(default, rename = "cloudCover")]
    pub cloud_cover: Option<f32>,
    /// UV index.
    #[serde(default, rename = "uvIndex")]
    pub uv_index: Option<f32>,
}

/// `/weather/v1/hourly`, in the subset this backend consumes.
#[derive(Debug, Clone, Deserialize)]
pub struct HourlyResponse {
    /// Provenance.
    pub metadata: Metadata,
    /// The hourly entries.
    #[serde(default)]
    pub hours: Vec<HourBlock>,
}

/// One `hours[]` entry.
#[derive(Debug, Clone, Deserialize)]
pub struct HourBlock {
    /// The hour, UTC (`2026-10-01T00:00Z`).
    #[serde(rename = "forecastTime")]
    pub forecast_time: String,
    /// The condition.
    pub condition: ConditionBlock,
    /// Air temperature.
    pub temperature: Measure,
    /// Apparent temperature.
    #[serde(default, rename = "feelsLike")]
    pub feels_like: Option<Measure>,
    /// Relative humidity, 0–1.
    #[serde(default)]
    pub humidity: Option<f32>,
    /// Wind.
    pub wind: Wind,
    /// Gust speed.
    #[serde(default, rename = "windGust")]
    pub wind_gust: Option<Measure>,
    /// Precipitation.
    #[serde(default)]
    pub precipitation: Option<Precipitation>,
    /// Sea level pressure.
    #[serde(default)]
    pub pressure: Option<Measure>,
    /// Horizontal visibility in metres.
    #[serde(default)]
    pub visibility: Option<Measure>,
    /// Total cloud cover, 0–1.
    #[serde(default, rename = "cloudCover")]
    pub cloud_cover: Option<f32>,
    /// UV index.
    #[serde(default, rename = "uvIndex")]
    pub uv_index: Option<f32>,
}

/// A `condition` object.
#[derive(Debug, Clone, Deserialize)]
pub struct ConditionBlock {
    /// The native condition code, as a string (`"100"`).
    pub code: String,
}

/// A `wind` object.
#[derive(Debug, Clone, Deserialize)]
pub struct Wind {
    /// Direction.
    #[serde(default)]
    pub direction: Option<Direction>,
    /// Speed.
    pub speed: Measure,
}

/// A `wind.direction` object.
#[derive(Debug, Clone, Deserialize)]
pub struct Direction {
    /// Degrees clockwise from north.
    #[serde(default)]
    pub degree: Option<f32>,
}

/// A `precipitation` object.
#[derive(Debug, Clone, Deserialize)]
pub struct Precipitation {
    /// The amount in mm.
    #[serde(default)]
    pub amount: Option<Measure>,
    /// The probability in percent.
    #[serde(default)]
    pub probability: Option<f32>,
}

// ---------------------------------------------------------------------------------------------
// Response → canonical model
// ---------------------------------------------------------------------------------------------

/// The current conditions; `observed_at` is the fetch instant (v1 reports none).
fn current_of(response: &CurrentResponse, now: DateTime<Utc>, tz: Tz) -> Result<Current> {
    let observed_at = now.with_timezone(&tz);
    let humidity = response
        .humidity
        .ok_or_else(|| missing("current.humidity"))?;
    let cloud_cover = response
        .cloud_cover
        .ok_or_else(|| missing("current.cloudCover"))?;
    let pressure = response
        .pressure
        .as_ref()
        .ok_or_else(|| missing("current.pressure"))?;

    Ok(Current {
        observed_at: observed_at.fixed_offset(),
        temp_c: metric(&response.temperature, "°C", "current.temperature")?,
        feels_like_c: response
            .feels_like
            .as_ref()
            .map(|measure| metric(measure, "°C", "current.feelsLike"))
            .transpose()?,
        humidity_pct: fraction(humidity),
        precip_mm: response
            .precipitation
            .as_ref()
            .and_then(|precipitation| precipitation.amount.as_ref())
            .map(|amount| metric(amount, "mm", "current.precipitation.amount"))
            .transpose()?
            .unwrap_or(0.0),
        weather: condition_of(&response.condition.code),
        cloud_cover_pct: fraction(cloud_cover),
        pressure_hpa: metric(pressure, "hPa", "current.pressure")?,
        wind_kmh: metric(&response.wind.speed, "m/s", "current.wind.speed")? * MS_TO_KMH,
        wind_dir_deg: response
            .wind
            .direction
            .as_ref()
            .and_then(|direction| direction.degree)
            .map(degrees)
            .ok_or_else(|| missing("current.wind.direction.degree"))?,
        wind_gust_kmh: response
            .wind_gust
            .as_ref()
            .map(|measure| metric(measure, "m/s", "current.windGust"))
            .transpose()?
            .map(|speed| speed * MS_TO_KMH),
        visibility_km: response
            .visibility
            .as_ref()
            .map(|measure| metric(measure, "m", "current.visibility"))
            .transpose()?
            .map(|metres| metres / 1000.0),
        uv_index: response.uv_index,
        is_day: daylight(&response.condition.code, observed_at),
    })
}

/// The hourly entries as canonical days.
fn days_of(response: &HourlyResponse, tz: Tz, days: u8) -> Result<Vec<crate::model::DayForecast>> {
    let samples: Vec<HourSample> = response
        .hours
        .iter()
        .filter_map(|hour| sample(hour, tz).transpose())
        .collect::<Result<Vec<_>>>()?;
    if samples.is_empty() {
        return Err(Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!("the response has no usable hourly entry in {tz}"),
        });
    }

    let mut forecasts = Vec::new();
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
    Ok(forecasts)
}

/// One hourly entry as a sample; `None` when a value the canonical model requires is absent.
fn sample(hour: &HourBlock, tz: Tz) -> Result<Option<HourSample>> {
    let at = instant(&hour.forecast_time)?.with_timezone(&tz);
    let Some(direction) = hour.wind.direction.as_ref().and_then(|d| d.degree) else {
        return Ok(None);
    };

    Ok(Some(HourSample {
        at,
        temp_c: metric(&hour.temperature, "°C", "hours[].temperature")?,
        feels_like_c: hour
            .feels_like
            .as_ref()
            .map(|measure| metric(measure, "°C", "hours[].feelsLike"))
            .transpose()?,
        precip_mm: hour
            .precipitation
            .as_ref()
            .and_then(|precipitation| precipitation.amount.as_ref())
            .map(|amount| metric(amount, "mm", "hours[].precipitation.amount"))
            .transpose()?
            .unwrap_or(0.0),
        precip_prob_pct: hour
            .precipitation
            .as_ref()
            .and_then(|precipitation| precipitation.probability)
            .map(percent),
        weather: condition_of(&hour.condition.code),
        wind_kmh: metric(&hour.wind.speed, "m/s", "hours[].wind.speed")? * MS_TO_KMH,
        wind_dir_deg: Some(degrees(direction)),
        humidity_pct: hour.humidity.map(fraction),
        visibility_km: hour
            .visibility
            .as_ref()
            .map(|measure| metric(measure, "m", "hours[].visibility"))
            .transpose()?
            .map(|metres| metres / 1000.0),
    }))
}

/// A measure in the unit the canonical model stores, or an upstream error when it arrived in
/// another one: v1 is metric-only, and a silent conversion would corrupt every value.
fn metric(measure: &Measure, unit: &str, what: &str) -> Result<f32> {
    if measure.unit != unit {
        return Err(Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!(
                "`{what}` arrived in `{}`; `{unit}` was expected",
                measure.unit
            ),
        });
    }
    Ok(measure.value)
}

/// Parses an ISO 8601 UTC instant.
///
/// v1 writes the minutes form (`2026-10-01T00:00Z`), which is valid ISO 8601 but not strict
/// RFC 3339, so the minutes form is tried first and the RFC 3339 parser is the fallback.
fn instant(text: &str) -> Result<DateTime<Utc>> {
    let text = text.trim();
    for format in ["%Y-%m-%dT%H:%M:%SZ", "%Y-%m-%dT%H:%MZ"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(text, format) {
            return Ok(Utc.from_utc_datetime(&naive));
        }
    }
    DateTime::parse_from_rfc3339(text)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!("`{text}` is not an ISO 8601 timestamp: {error}"),
        })
}

/// The upstream error for a field the canonical model requires.
fn missing(field: &str) -> Error {
    Error::Upstream {
        provider: PROVIDER.to_owned(),
        status: None,
        message: format!("the response has no `{field}`"),
    }
}

/// A 0–1 fraction as a whole percent.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn fraction(value: f32) -> u8 {
    (value * 100.0).round().clamp(0.0, 100.0) as u8
}

/// A percentage upstream already sent as a whole percent (0–100): clamped, never scaled.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn percent(value: f32) -> u8 {
    value.round().clamp(0.0, 100.0) as u8
}

/// A direction in degrees, normalised into `0..360`.
#[allow(clippy::cast_possible_truncation)]
fn degrees(value: f32) -> u16 {
    let wrapped = (value.round() as i64).rem_euclid(360);
    u16::try_from(wrapped).unwrap_or_default()
}

/// Whether it is daylight: the night icon family decides, otherwise the local civil day.
///
/// Only the clear/cloudy codes have a night variant (`150`–`153`), so the civil-day rule covers the
/// precipitation and obscuration codes (the same stand-in SMHI, WWO and Pirate Weather use).
fn daylight(code: &str, at: DateTime<Tz>) -> bool {
    match code.trim().parse::<u16>() {
        Ok(150..=153) => false,
        Ok(100..=104) => true,
        _ => matches!(at.hour(), 6..=17),
    }
}

/// `QWeather`'s condition code as a WMO 4677 code.
///
/// The families follow the provider's published code list (100–999, plus the 150–153 night family
/// its own examples emit); `900`/`901` (hot/cold) and `999` (unknown) have no WMO equivalent and
/// stay undescribed, and an unlisted code is unknown rather than clamped into a neighbour.
#[allow(clippy::match_same_arms)]
fn condition_of(code: &str) -> Condition {
    let code = code.trim().parse::<u16>().unwrap_or(0);
    Condition::from_u8(match code {
        100 | 150 => 0,              // sunny / clear night
        102 | 152 => 1,              // few clouds
        101 | 103 | 151 | 153 => 2,  // cloudy, partly cloudy
        104 => 3,                    // overcast
        300 => 80,                   // shower rain
        301 => 82,                   // heavy shower rain
        302 => 95,                   // thundershower
        303 | 304 => 96,             // heavy thunderstorm, thunderstorm with hail
        305 => 61,                   // light rain
        306 | 314 | 315 => 63,       // moderate rain
        307..=312 | 316..=318 => 65, // heavy rain and the rainstorm ladder
        313 => 66,                   // freezing rain
        399 => 63,                   // rain
        400 | 408 => 71,             // light snow
        401 | 409 | 499 => 73,       // moderate snow
        402 | 403 | 410 => 75,       // heavy snow, snowstorm
        404..=406 => 66,             // sleet, rain and snow
        407 => 85,                   // snow flurry
        500..=514 => 45,             // mist, fog, haze, sand, dust and their stronger forms
        515 => 56,                   // freezing drizzle
        _ => 255,                    // hot, cold, unknown and anything unlisted
    })
}

#[cfg(test)]
mod tests {
    use super::{condition_of, validate_host};
    use crate::model::Condition;

    /// Every code the provider publishes, minus the three with no weather meaning.
    const PUBLISHED: [u16; 51] = [
        100, 101, 102, 103, 104, 300, 301, 302, 303, 304, 305, 306, 307, 308, 309, 310, 311, 312,
        313, 314, 315, 316, 317, 318, 399, 400, 401, 402, 403, 404, 405, 406, 407, 408, 409, 410,
        499, 500, 501, 502, 503, 504, 507, 508, 509, 510, 511, 512, 513, 514, 515,
    ];

    #[test]
    fn every_published_code_maps_to_a_described_condition() {
        for code in PUBLISHED {
            let condition = condition_of(&code.to_string());
            assert!(
                condition.is_known(),
                "{code} maps to an undescribed condition"
            );
        }
        // The night family the published table omits but the API emits.
        for code in 150..=153 {
            assert!(condition_of(&code.to_string()).is_known(), "{code}");
        }
    }

    #[test]
    fn the_meaningless_codes_stay_unknown() {
        for code in ["900", "901", "999", "0", ""] {
            assert!(
                !condition_of(code).is_known(),
                "{code} should be undescribed"
            );
        }
    }

    #[test]
    fn the_host_must_be_the_https_account_authority() {
        assert!(validate_host("https://abc123.re.qweatherapi.com").is_ok());
        assert!(validate_host("https://my-account.re.qweatherapi.com").is_ok());
        for rejected in [
            "http://abc123.re.qweatherapi.com",
            "https://abc123.qweatherapi.com",
            "https://.re.qweatherapi.com",
            "https://attacker.example",
            "https://api.qweather.com",
            "ftp://abc123.re.qweatherapi.com",
        ] {
            assert!(
                validate_host(rejected).is_err(),
                "{rejected} must be rejected"
            );
        }
    }

    #[test]
    fn the_families_follow_the_published_groups() {
        assert_eq!(condition_of("100").description_en(), "Clear sky");
        assert_eq!(condition_of("104").description_en(), "Overcast");
        assert_eq!(condition_of("302").description_en(), "Thunderstorm");
        assert_eq!(condition_of("307").description_en(), "Heavy rain");
        assert_eq!(condition_of("400").description_en(), "Slight snow fall");
        assert_eq!(condition_of("501").description_en(), "Fog");
        // 515 is freezing drizzle, not another fog variant.
        assert_eq!(condition_of("515"), Condition::from_u8(56));
        assert_eq!(
            condition_of("515").description_en(),
            "Light freezing drizzle"
        );
        assert_eq!(condition_of("150").description_en(), "Clear sky");
        assert_eq!(condition_of("999"), Condition::from_u8(255));
    }
}
