// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! World Weather Online: one call per fetch against the Local Weather API.
//!
//! What this module knows that the payload does not spell out:
//!
//! 1. **`format=json` is mandatory** — the documented default is XML.
//! 2. **Every scalar arrives as a JSON string** (`"tempC": "15"`, `"precipMM": "0.0"`), and the
//!    descriptive fields are wrapped in single-element arrays (`weatherDesc[0].value`,
//!    `astronomy[0].sunrise`), so the `text_number` / `text_value` helpers below do the unwrapping.
//! 3. **`hourly[].time` is an unpadded local `HHMM` string** (`"0"`, `"300"`, `"2100"`), which is
//!    joined to the day's `date` to make a local timestamp.
//! 4. **`current_condition[0].observation_time` carries no date and is in UTC, not local** (the
//!    docs say local; two recordings at known instants show the UTC wall clock), so it is joined to
//!    the fetch instant's UTC date and then converted to the location's zone.
//! 5. **The payload carries no time zone and no daylight flag**, so a provisional (UTC) location is
//!    refused rather than aggregated in the wrong zone, and `is_day` comes from the local civil day
//!    until step 17 computes real sun times.
//! 6. **The free tier is 100 requests/day and the FAQ gives it 5 forecast days** (the endpoint
//!    itself accepts 14; the pricing page contradicts both), so the registry's `max_days: 5` is the
//!    clamp.
//! 7. **The terms add obligations beyond attribution**: a mandatory end-user disclaimer, a
//!    60-minute cache ceiling for current conditions and 24 hours for forecasts, no resale.
//!
//! Licence: proprietary. Free keys must credit "Weather Data by WorldWeatherOnline.com"; the
//! registry row carries the line the renderers print.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone as _, Timelike as _, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Deserializer};

use super::dayparts::{HourSample, aggregate_day, covers_every_part};
use super::{
    Capabilities, Env, FetchRequest, JsonFetch, Provider, ProviderId, attribution, fetch_json,
    local_today, requested_days,
};
use crate::cache::CacheKey;
use crate::error::{Error, Result};
use crate::geo::provisional_zone;
use crate::http::HttpRequest;
use crate::model::{Condition, Current, Location, Report, ReportMode, resolve_local};

/// The provider id, as the registry and every error message spell it.
const PROVIDER: &str = "worldweatheronline";

/// The Local Weather API endpoint.
pub const WEATHER_URL: &str = "https://api.worldweatheronline.com/premium/v1/weather.ashx";

/// The World Weather Online backend. Stateless: one value serves every fetch.
#[derive(Debug, Clone, Copy, Default)]
pub struct WorldWeatherOnline;

impl Provider for WorldWeatherOnline {
    fn id(&self) -> ProviderId {
        ProviderId::WorldWeatherOnline
    }

    fn capabilities(&self) -> Capabilities {
        ProviderId::WorldWeatherOnline.metadata().capabilities()
    }

    fn fetch_report(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
        let max_days = self.capabilities().max_days;
        let days = requested_days(req.days, max_days, PROVIDER, env.quiet);
        if provisional_zone(loc) {
            return Err(Error::Usage(format!(
                "provider `{PROVIDER}` needs the location's time zone and its response carries only \
                 local times: pass a place name (e.g. `cirrocast -p {PROVIDER} Beijing`) or set \
                 `location.default` instead of raw coordinates"
            )));
        }

        let variable = self
            .capabilities()
            .key_env
            .ok_or_else(|| Error::Config(format!("provider `{PROVIDER}` names no key variable")))?;
        let key = env.keys.get(PROVIDER)?.ok_or_else(|| Error::MissingKey {
            provider: PROVIDER.to_owned(),
            env: variable.to_owned(),
        })?;

        let ttl = Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs));
        let request = HttpRequest::get(WEATHER_URL)
            .query("key", &key)
            .query("q", format!("{:.4},{:.4}", loc.lat, loc.lon))
            .query("format", "json")
            .query("num_of_days", days.max(1).to_string())
            .query("tp", "3")
            .secret(&key);

        let response: Envelope = fetch_json(
            env,
            loc,
            &JsonFetch {
                provider: ProviderId::WorldWeatherOnline,
                request: request.clone(),
                key: CacheKey::weather(PROVIDER, loc.lat, loc.lon, days, local_today(env, loc.tz)),
                ttl,
                what: "local weather",
            },
        )?;

        report(&response.data, loc, request.redacted_url(), days, env)
    }
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// The one-key envelope every response uses.
#[derive(Debug, Clone, Deserialize)]
pub struct Envelope {
    /// The payload.
    pub data: Data,
}

/// The `data` object, in the subset this backend consumes.
#[derive(Debug, Clone, Deserialize)]
pub struct Data {
    /// Current conditions; always exactly one element.
    #[serde(default)]
    pub current_condition: Vec<CurrentBlock>,
    /// The forecast days.
    #[serde(default)]
    pub weather: Vec<DayBlock>,
    /// The error envelope (`{"data":{"error":[…]}}`) upstream answers a bad request with.
    ///
    /// It arrives with HTTP 200, so a successful deserialisation into an otherwise empty `Data` is
    /// not an answer; every field is optional so a drifting error shape cannot mask the failure.
    #[serde(default)]
    pub error: Vec<ErrorBlock>,
}

/// One `data.error[]` entry.
#[derive(Debug, Clone, Deserialize)]
pub struct ErrorBlock {
    /// The human-readable reason, e.g. `API key is invalid.`.
    #[serde(default)]
    pub msg: Option<String>,
}

/// `current_condition[0]`.
#[derive(Debug, Clone, Deserialize)]
pub struct CurrentBlock {
    /// Local observation time, 12-hour clock (`"04:29 PM"`).
    #[serde(default)]
    pub observation_time: String,
    /// Air temperature in °C.
    #[serde(default, rename = "temp_C", deserialize_with = "text_number")]
    pub temp_c: Option<f32>,
    /// Apparent temperature in °C.
    #[serde(default, rename = "FeelsLikeC", deserialize_with = "text_number")]
    pub feels_like_c: Option<f32>,
    /// Relative humidity in percent.
    #[serde(default, deserialize_with = "text_number")]
    pub humidity: Option<f32>,
    /// Sea level pressure in hPa.
    #[serde(default, deserialize_with = "text_number")]
    pub pressure: Option<f32>,
    /// Wind speed in km/h.
    #[serde(default, rename = "windspeedKmph", deserialize_with = "text_number")]
    pub windspeed_kmph: Option<f32>,
    /// Direction the wind blows from, in degrees.
    #[serde(default, rename = "winddirDegree", deserialize_with = "text_number")]
    pub winddir_degree: Option<f32>,
    /// Horizontal visibility in km.
    #[serde(default, deserialize_with = "text_number")]
    pub visibility: Option<f32>,
    /// Precipitation in the last hour, in mm.
    #[serde(default, rename = "precipMM", deserialize_with = "text_number")]
    pub precip_mm: Option<f32>,
    /// Cloud cover in percent.
    #[serde(default, deserialize_with = "text_number")]
    pub cloudcover: Option<f32>,
    /// UV index.
    #[serde(default, rename = "uvIndex", deserialize_with = "text_number")]
    pub uv_index: Option<f32>,
    /// The native condition code.
    #[serde(default, rename = "weatherCode", deserialize_with = "text_number")]
    pub weather_code: Option<f32>,
}

/// One `weather[]` entry.
#[derive(Debug, Clone, Deserialize)]
pub struct DayBlock {
    /// Local date (`YYYY-MM-DD`).
    #[serde(default)]
    pub date: String,
    /// Maximum temperature in °C.
    #[serde(default, rename = "maxtempC", deserialize_with = "text_number")]
    pub maxtemp_c: Option<f32>,
    /// Minimum temperature in °C.
    #[serde(default, rename = "mintempC", deserialize_with = "text_number")]
    pub mintemp_c: Option<f32>,
    /// Sun times; a single-element array.
    #[serde(default)]
    pub astronomy: Vec<AstroBlock>,
    /// The day's 3-hourly slots.
    #[serde(default)]
    pub hourly: Vec<HourBlock>,
}

/// `astronomy[0]`.
#[derive(Debug, Clone, Deserialize)]
pub struct AstroBlock {
    /// Local sunrise, 12-hour clock.
    #[serde(default)]
    pub sunrise: Option<String>,
    /// Local sunset, 12-hour clock.
    #[serde(default)]
    pub sunset: Option<String>,
}

/// One `hourly[]` entry.
#[derive(Debug, Clone, Deserialize)]
pub struct HourBlock {
    /// Unpadded local `HHMM` (`"0"`, `"300"`, `"2100"`).
    #[serde(default)]
    pub time: String,
    /// Air temperature in °C.
    #[serde(default, rename = "tempC", deserialize_with = "text_number")]
    pub temp_c: Option<f32>,
    /// Apparent temperature in °C.
    #[serde(default, rename = "FeelsLikeC", deserialize_with = "text_number")]
    pub feels_like_c: Option<f32>,
    /// Relative humidity in percent.
    #[serde(default, deserialize_with = "text_number")]
    pub humidity: Option<f32>,
    /// Chance of rain in percent.
    #[serde(default, rename = "chanceofrain", deserialize_with = "text_number")]
    pub chance_of_rain: Option<f32>,
    /// Precipitation over the slot, in mm.
    #[serde(default, rename = "precipMM", deserialize_with = "text_number")]
    pub precip_mm: Option<f32>,
    /// Wind speed in km/h.
    #[serde(default, rename = "windspeedKmph", deserialize_with = "text_number")]
    pub windspeed_kmph: Option<f32>,
    /// Direction the wind blows from, in degrees.
    #[serde(default, rename = "winddirDegree", deserialize_with = "text_number")]
    pub winddir_degree: Option<f32>,
    /// Horizontal visibility in km.
    #[serde(default, deserialize_with = "text_number")]
    pub visibility: Option<f32>,
    /// The native condition code.
    #[serde(default, rename = "weatherCode", deserialize_with = "text_number")]
    pub weather_code: Option<f32>,
}

/// Reads a number that upstream sent as a JSON string (`"15"`, `"0.0"`), or as a number.
fn text_number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f32>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Text(String),
        Number(f32),
    }
    Ok(match Option::<Raw>::deserialize(deserializer)? {
        Some(Raw::Number(value)) => Some(value),
        Some(Raw::Text(text)) => text.trim().parse::<f32>().ok(),
        None => None,
    })
}

// ---------------------------------------------------------------------------------------------
// Response → canonical model
// ---------------------------------------------------------------------------------------------

/// Turns one response into a [`Report`].
///
/// `days[0]` is the first location-local date whose four parts all have a sample (today whenever
/// the slots cover it, the next date when they do not).
fn report(data: &Data, loc: &Location, url: String, days: u8, env: &Env<'_>) -> Result<Report> {
    // An error envelope is HTTP 200 with `{"data":{"error":[…]}}`; treating it as a successful
    // empty response would report "no forecast days" for a bad key or an exhausted quota.
    if let Some(error) = data.error.first() {
        return Err(Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!(
                "the response is an error envelope: {}",
                error.msg.as_deref().unwrap_or("no detail")
            ),
        });
    }
    let tz = loc.tz;
    let utc_today = DateTime::<Utc>::from(env.cache.clock().now()).date_naive();
    let current = data
        .current_condition
        .first()
        .and_then(|block| current_of(block, tz, utc_today));

    let mut forecasts = Vec::new();
    if days > 0 {
        for day in &data.weather {
            let day_date =
                NaiveDate::parse_from_str(day.date.trim(), "%Y-%m-%d").map_err(|error| {
                    Error::Upstream {
                        provider: PROVIDER.to_owned(),
                        status: None,
                        message: format!("`{}` is not a calendar date: {error}", day.date),
                    }
                })?;
            let samples: Vec<HourSample> = day
                .hourly
                .iter()
                .filter_map(|hour| sample(hour, day_date, tz))
                .collect();
            // A day whose slots do not cover all four parts cannot be rendered into the canonical
            // shape; it is skipped rather than filled with invented values.
            if !covers_every_part(&samples, day_date, tz) {
                continue;
            }
            let temp_min_c = day.mintemp_c.ok_or_else(|| missing(day_date, "mintempC"))?;
            let temp_max_c = day.maxtemp_c.ok_or_else(|| missing(day_date, "maxtempC"))?;
            forecasts.push(aggregate_day(
                &samples,
                day_date,
                tz,
                PROVIDER,
                temp_min_c,
                temp_max_c,
                astro_time(
                    day.astronomy.first().and_then(|a| a.sunrise.as_deref()),
                    day_date,
                    tz,
                ),
                astro_time(
                    day.astronomy.first().and_then(|a| a.sunset.as_deref()),
                    day_date,
                    tz,
                ),
            )?);
        }
        if forecasts.is_empty() {
            return Err(Error::Upstream {
                provider: PROVIDER.to_owned(),
                status: None,
                message: format!("the response covers no complete local day in {tz}"),
            });
        }
    }

    Ok(Report {
        location: loc.clone(),
        current,
        days: forecasts,
        alerts: Vec::new(),
        air: None,
        astro: None,
        marine: None,
        normals: None,
        mode: ReportMode::Forecast,
        attribution: attribution(
            ProviderId::WorldWeatherOnline,
            url,
            env.cache.clock().now().into(),
            (env.verbose > 0).then(|| {
                format!(
                    "{} day(s) of 3-hourly data, all values strings; local times only",
                    data.weather.len()
                )
            }),
        ),
    })
}

/// The current conditions; the observation time is joined to `utc_today` and converted to `tz`.
fn current_of(block: &CurrentBlock, tz: Tz, utc_today: NaiveDate) -> Option<Current> {
    let time = NaiveTime::parse_from_str(block.observation_time.trim(), "%I:%M %p").ok()?;
    let observed_at = Utc
        .from_utc_datetime(&utc_today.and_time(time))
        .with_timezone(&tz);
    Some(Current {
        observed_at: observed_at.fixed_offset(),
        temp_c: block.temp_c?,
        feels_like_c: block.feels_like_c,
        humidity_pct: block.humidity.map(percent),
        precip_mm: block.precip_mm.unwrap_or(0.0),
        weather: condition_of(block.weather_code?),
        cloud_cover_pct: block.cloudcover.map(percent),
        pressure_hpa: block.pressure?,
        wind_kmh: block.windspeed_kmph?,
        wind_dir_deg: Some(degrees(block.winddir_degree?)),
        wind_gust_kmh: None,
        visibility_km: block.visibility,
        uv_index: block.uv_index,
        // The payload has no daylight flag; the local civil day is the honest stand-in.
        is_day: matches!(observed_at.hour(), 6..=17),
    })
}

/// One 3-hourly slot as a sample, or `None` when a required value is missing or unparsable.
fn sample(hour: &HourBlock, date: NaiveDate, tz: Tz) -> Option<HourSample> {
    let at = local_hour(&hour.time, date, tz)?;
    Some(HourSample {
        at,
        temp_c: hour.temp_c?,
        feels_like_c: hour.feels_like_c,
        precip_mm: hour.precip_mm.unwrap_or(0.0),
        precip_prob_pct: hour.chance_of_rain.map(percent),
        weather: condition_of(hour.weather_code?),
        wind_kmh: hour.windspeed_kmph?,
        wind_dir_deg: hour.winddir_degree.map(degrees),
        humidity_pct: hour.humidity.map(percent),
        visibility_km: hour.visibility,
    })
}

/// `"300"` + `2026-10-01` → 03:00 local on that date.
fn local_hour(text: &str, date: NaiveDate, tz: Tz) -> Option<DateTime<Tz>> {
    let digits = text.trim();
    let padded = format!("{digits:0>4}");
    let time = NaiveTime::parse_from_str(&padded, "%H%M").ok()?;
    resolve_local(tz, date.and_time(time)).ok()
}

/// A `"06:10 AM"` sun time joined to `date`.
fn astro_time(
    text: Option<&str>,
    date: NaiveDate,
    tz: Tz,
) -> Option<DateTime<chrono::FixedOffset>> {
    let time = NaiveTime::parse_from_str(text?.trim(), "%I:%M %p").ok()?;
    resolve_local(tz, date.and_time(time))
        .ok()
        .map(|local| local.fixed_offset())
}

/// The upstream error for a missing daily aggregate.
fn missing(date: NaiveDate, field: &str) -> Error {
    Error::Upstream {
        provider: PROVIDER.to_owned(),
        status: None,
        message: format!("the response has no `{field}` for {date}"),
    }
}

/// A 0–100 value as a whole percent.
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

/// World Weather Online's `weatherCode` as a WMO 4677 code.
///
/// The families follow the provider's published code feed (113–395), one arm per published code;
/// each obscuration maps onto the model's own code for the phenomenon it names (mist, smoke, haze,
/// dust, sand), and only the phenomena this crate's table does not describe (the storm forms,
/// smog) choose the nearest described family. An unlisted code stays unknown instead of being
/// clamped into a neighbour.
#[allow(clippy::match_same_arms)]
fn condition_of(code: f32) -> Condition {
    #[allow(clippy::cast_possible_truncation)]
    let code = code.round() as i64;
    Condition::from_u8(match code {
        113 => 0, // clear / sunny
        116 => 1, // partly cloudy
        119 => 2, // cloudy
        122 => 3, // overcast
        // The feed's obscuration family, each onto the model's own code for the phenomenon it
        // names (review 05 §3.4). The storm forms take the raised-by-wind family because the
        // standard's duststorm codes are not in this crate's table, and smog has no WMO 4677 code
        // and takes the suspended-particle haze family rather than fog.
        125 => 5,  // haze
        128 => 6,  // dust haze
        131 => 7,  // blowing dust
        134 => 7,  // dust storm
        137 => 7,  // sandstorm
        140 => 7,  // severe sandstorm
        143 => 10, // mist
        146 => 4,  // smoke
        149 => 5,  // smoky haze
        152 => 5,  // smog
        155 => 5,  // severe smog
        158 => 6,  // Saharan dust
        161 => 6,  // dust
        176 => 51, // patchy rain nearby
        179 => 71, // patchy snow nearby
        182 => 66, // patchy sleet nearby
        185 => 56, // patchy freezing drizzle nearby
        200 => 95, // thundery outbreaks in nearby
        227 => 85, // blowing snow
        230 => 75, // blizzard
        248 => 45, // fog
        260 => 48, // freezing fog
        263 => 51, // patchy light drizzle
        266 => 53, // light drizzle
        281 => 56, // freezing drizzle
        284 => 57, // heavy freezing drizzle
        293 => 61, // patchy light rain
        296 => 61, // light rain
        299 => 63, // moderate rain at times
        302 => 63, // moderate rain
        305 => 65, // heavy rain at times
        308 => 65, // heavy rain
        311 => 66, // light freezing rain
        314 => 67, // moderate or heavy freezing rain
        317 => 66, // light sleet
        320 => 67, // moderate or heavy sleet
        323 => 71, // patchy light snow
        326 => 71, // light snow
        329 => 73, // patchy moderate snow
        332 => 73, // moderate snow
        335 => 75, // patchy heavy snow
        338 => 75, // heavy snow
        350 => 77, // ice pellets
        353 => 80, // light rain shower
        356 => 81, // moderate or heavy rain shower
        359 => 82, // torrential rain shower
        362 => 66, // light sleet showers
        365 => 67, // moderate or heavy sleet showers
        368 => 85, // light snow showers
        371 => 86, // moderate or heavy snow showers
        374 => 77, // light showers of ice pellets
        377 => 77, // moderate or heavy showers of ice pellets
        386 => 95, // patchy light rain with thunder
        389 => 96, // moderate or heavy rain with thunder
        392 => 95, // patchy light snow with thunder
        395 => 96, // moderate or heavy snow with thunder
        _ => 255,  // undescribed: `Condition::from_u8` keeps 255 as unknown
    })
}

#[cfg(test)]
mod tests {
    use super::condition_of;
    use crate::model::Condition;

    /// Every code the provider's published feed lists and the WMO 4677 code the mapping owes it.
    ///
    /// The expectation is written out from the vendor's own feed
    /// (`worldweatheronline.com/feed/wwoConditionCodes.xml`) rather than echoed from
    /// [`condition_of`], so a future edit that drops a code or collapses two distinct phenomena —
    /// the review-05 §3.4 defects, where mist and smoky haze became fog and the eleven obscuration
    /// codes had no arm at all — fails here.
    const PUBLISHED: [(i64, u8); 60] = [
        (113, 0),  // clear / sunny
        (116, 1),  // partly cloudy
        (119, 2),  // cloudy
        (122, 3),  // overcast
        (125, 5),  // haze
        (128, 6),  // dust haze
        (131, 7),  // blowing dust
        (134, 7),  // dust storm
        (137, 7),  // sandstorm
        (140, 7),  // severe sandstorm
        (143, 10), // mist
        (146, 4),  // smoke
        (149, 5),  // smoky haze
        (152, 5),  // smog
        (155, 5),  // severe smog
        (158, 6),  // Saharan dust
        (161, 6),  // dust
        (176, 51), // patchy rain nearby
        (179, 71), // patchy snow nearby
        (182, 66), // patchy sleet nearby
        (185, 56), // patchy freezing drizzle nearby
        (200, 95), // thundery outbreaks in nearby
        (227, 85), // blowing snow
        (230, 75), // blizzard
        (248, 45), // fog
        (260, 48), // freezing fog
        (263, 51), // patchy light drizzle
        (266, 53), // light drizzle
        (281, 56), // freezing drizzle
        (284, 57), // heavy freezing drizzle
        (293, 61), // patchy light rain
        (296, 61), // light rain
        (299, 63), // moderate rain at times
        (302, 63), // moderate rain
        (305, 65), // heavy rain at times
        (308, 65), // heavy rain
        (311, 66), // light freezing rain
        (314, 67), // moderate or heavy freezing rain
        (317, 66), // light sleet
        (320, 67), // moderate or heavy sleet
        (323, 71), // patchy light snow
        (326, 71), // light snow
        (329, 73), // patchy moderate snow
        (332, 73), // moderate snow
        (335, 75), // patchy heavy snow
        (338, 75), // heavy snow
        (350, 77), // ice pellets
        (353, 80), // light rain shower
        (356, 81), // moderate or heavy rain shower
        (359, 82), // torrential rain shower
        (362, 66), // light sleet showers
        (365, 67), // moderate or heavy sleet showers
        (368, 85), // light snow showers
        (371, 86), // moderate or heavy snow showers
        (374, 77), // light showers of ice pellets
        (377, 77), // moderate or heavy showers of ice pellets
        (386, 95), // patchy light rain with thunder
        (389, 96), // moderate or heavy rain with thunder
        (392, 95), // patchy light snow with thunder
        (395, 96), // moderate or heavy snow with thunder
    ];

    #[test]
    fn the_recorded_fixture_deserialises_into_three_days_of_eight_slots() {
        let text = std::fs::read_to_string("tests/fixtures/wwo/weather_ashx.json")
            .expect("the fixture is readable");
        let envelope: super::Envelope =
            serde_json::from_str(&text).expect("the recorded shape parses");
        assert_eq!(envelope.data.current_condition.len(), 1);
        assert_eq!(envelope.data.weather.len(), 3);
        for day in &envelope.data.weather {
            assert_eq!(day.hourly.len(), 8, "{} slots", day.date);
            assert!(day.mintemp_c.is_some() && day.maxtemp_c.is_some());
            assert_eq!(day.astronomy.len(), 1);
        }
    }

    #[test]
    #[allow(clippy::cast_precision_loss)] // the published codes are small integers
    fn every_published_code_maps_to_its_documented_wmo_code() {
        for (code, wmo) in PUBLISHED {
            let condition = condition_of(code as f32);
            assert_eq!(condition.code(), wmo, "{code} must map to WMO {wmo}");
        }
    }

    #[test]
    fn an_unlisted_code_stays_unknown() {
        assert!(!condition_of(100.0).is_known());
        assert!(!condition_of(400.0).is_known());
    }

    #[test]
    fn freezing_fog_is_not_plain_fog() {
        assert_eq!(condition_of(248.0), Condition::from_u8(45));
        assert_eq!(condition_of(260.0), Condition::from_u8(48));
        assert_ne!(condition_of(260.0), condition_of(248.0));
    }

    #[test]
    fn a_missing_humidity_or_cloud_cover_nulls_only_that_field() {
        let block: super::CurrentBlock = serde_json::from_str(
            r#"{"observation_time":"12:00 PM","temp_C":"10","FeelsLikeC":"9",
                "pressure":"1010","windspeedKmph":"5","winddirDegree":"180",
                "weatherCode":"113"}"#,
        )
        .expect("a current block");
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 5).expect("a date");
        let current =
            super::current_of(&block, chrono_tz::Tz::UTC, today).expect("the block survives");
        assert_eq!(current.humidity_pct, None);
        assert_eq!(current.cloud_cover_pct, None);
        assert_eq!(current.wind_dir_deg, Some(180));
    }

    #[test]
    fn the_families_follow_the_published_groups() {
        assert_eq!(condition_of(113.0).description_en(), "Clear sky");
        assert_eq!(condition_of(122.0).description_en(), "Overcast");
        assert_eq!(condition_of(143.0).description_en(), "Mist");
        assert_eq!(condition_of(149.0).description_en(), "Haze");
        assert_eq!(condition_of(248.0).description_en(), "Fog");
        assert_eq!(condition_of(296.0).description_en(), "Slight rain");
        assert_eq!(condition_of(338.0).description_en(), "Heavy snow fall");
        assert_eq!(
            condition_of(389.0).description_en(),
            "Thunderstorm with slight hail"
        );
    }
}
