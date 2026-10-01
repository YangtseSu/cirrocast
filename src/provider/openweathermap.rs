// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `OpenWeatherMap`: the first key-requiring backend.
//!
//! Two calls per fetch — `/data/2.5/weather` for the current conditions and `/data/2.5/forecast`
//! for the 3-hourly series — each cached under its own `weather/openweathermap-<part>-…` key, so a
//! run that only needs the current block does not pay for the forecast.
//!
//! What this module knows that the payload does not spell out:
//!
//! 1. **Metric means m/s for wind.** `units=metric` gives °C and m/s; the canonical model is km/h.
//! 2. **`rain`/`snow` blocks are absent, not zero**, when nothing falls, and the accumulation key
//!    differs between the endpoints (`rain.1h` for the current conditions, `rain.3h` per forecast
//!    slot) — the same field name means different windows.
//! 3. **`main.temp_min`/`temp_max` are not daily extremes** (the docs say they usually equal
//!    `temp`); the day extremes are computed from the slots.
//! 4. **The forecast starts at the next 3-hour boundary**, so the location-local today is usually
//!    incomplete and is skipped (a `DayPart` cannot represent "no data") — the same rule SMHI uses.
//! 5. **`weather[0]` is the primary condition** and `weather[0].main` is not translated; the code is
//!    what maps, and `weather[0].icon`'s trailing letter is the day/night flag.
//! 6. **`deg` is omitted for a calm wind.** The canonical `Current` has no "no direction" state, so
//!    a missing direction becomes 0 with the speed also at 0.
//! 7. **The payload carries no time zone name**, only a UTC offset, so a provisional (UTC) location
//!    is refused rather than aggregated in the wrong zone.
//!
//! Licence: the data is `ODbL` 1.0 and the terms require visible attribution ("Weather data provided
//! by `OpenWeather`, a link to <https://openweathermap.org/> and the logo); the registry row carries
//! the line the renderers print.

use std::time::Duration;

use chrono::{DateTime, Utc};
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
const PROVIDER: &str = "openweathermap";

/// The current-conditions endpoint.
pub const CURRENT_URL: &str = "https://api.openweathermap.org/data/2.5/weather";

/// The 5-day / 3-hour forecast endpoint.
pub const FORECAST_URL: &str = "https://api.openweathermap.org/data/2.5/forecast";

/// Metres per second → km/h (`units=metric` serves wind in m/s).
const MS_TO_KMH: f32 = 3.6;

/// The `OpenWeatherMap` backend. Stateless: one value serves every fetch.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenWeatherMap;

impl Provider for OpenWeatherMap {
    fn id(&self) -> ProviderId {
        ProviderId::OpenWeatherMap
    }

    fn capabilities(&self) -> Capabilities {
        ProviderId::OpenWeatherMap.metadata().capabilities()
    }

    fn fetch(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
        let max_days = self.capabilities().max_days;
        let days = requested_days(req.days, max_days, PROVIDER, env.quiet);
        if provisional_zone(loc) {
            return Err(Error::Usage(format!(
                "provider `{PROVIDER}` needs the location's time zone and its response carries only \
                 a UTC offset: pass a place name (e.g. `cirrocast -p {PROVIDER} Beijing`) or set \
                 `location.default` instead of raw coordinates"
            )));
        }

        let secret = self
            .capabilities()
            .key_env
            .map(str::to_owned)
            .ok_or_else(|| Error::Config(format!("provider `{PROVIDER}` names no key variable")))?;
        let key = env.keys.get(PROVIDER)?.ok_or_else(|| Error::MissingKey {
            provider: PROVIDER.to_owned(),
            env: secret,
        })?;

        let ttl = Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs));
        let today = local_today(env, loc.tz);

        let current_request = request(CURRENT_URL, loc, &key);
        let current: CurrentResponse = fetch_json(
            env,
            loc,
            &JsonFetch {
                provider: ProviderId::OpenWeatherMap,
                request: current_request.clone(),
                key: CacheKey::weather_part(PROVIDER, "current", loc.lat, loc.lon, days, today),
                ttl,
                what: "current",
            },
        )?;
        let current = current_of(&current, loc.tz, env);

        let mut url = current_request.redacted_url();
        let mut forecasts = Vec::new();
        if days > 0 {
            let forecast_request = request(FORECAST_URL, loc, &key);
            let forecast: ForecastResponse = fetch_json(
                env,
                loc,
                &JsonFetch {
                    provider: ProviderId::OpenWeatherMap,
                    request: forecast_request.clone(),
                    key: CacheKey::weather_part(
                        PROVIDER, "forecast", loc.lat, loc.lon, days, today,
                    ),
                    ttl,
                    what: "forecast",
                },
            )?;
            url = forecast_request.redacted_url();
            forecasts = days_of(&forecast, loc.tz, days)?;
        }

        Ok(Report {
            location: loc.clone(),
            current,
            days: forecasts,
            attribution: attribution(
                ProviderId::OpenWeatherMap,
                url,
                env.cache.clock().now().into(),
                (env.verbose > 0).then(|| {
                    format!(
                        "current {CURRENT_URL} forecast {FORECAST_URL} (units=metric, 3-hour slots)"
                    )
                }),
            ),
        })
    }
}

/// One endpoint's URL for a location, with the key in the query and marked for redaction.
fn request(url: &str, loc: &Location, key: &str) -> HttpRequest {
    HttpRequest::get(url)
        .query("lat", format!("{:.4}", loc.lat))
        .query("lon", format!("{:.4}", loc.lon))
        .query("units", "metric")
        .query("appid", key)
        .secret(key)
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// `/data/2.5/weather`, in the subset this backend consumes.
#[derive(Debug, Clone, Deserialize)]
pub struct CurrentResponse {
    /// Observation time, unix UTC seconds.
    pub dt: i64,
    /// The location's offset from UTC in seconds.
    pub timezone: i32,
    /// The place name upstream matched.
    #[serde(default)]
    pub name: String,
    /// Conditions, most significant first.
    #[serde(default)]
    pub weather: Vec<Weather>,
    /// Temperature, humidity and pressure.
    pub main: Main,
    /// Wind speed, direction and gust.
    pub wind: Wind,
    /// Cloud cover.
    pub clouds: Clouds,
    /// Horizontal visibility in **metres**, capped at 10 km.
    #[serde(default)]
    pub visibility: Option<f32>,
    /// Rain over the last hour, when it rained.
    #[serde(default)]
    pub rain: Option<Precip>,
    /// Snow over the last hour, when it snowed.
    #[serde(default)]
    pub snow: Option<Precip>,
}

/// `/data/2.5/forecast`, in the subset this backend consumes.
#[derive(Debug, Clone, Deserialize)]
pub struct ForecastResponse {
    /// The city block; its `timezone` is an offset in seconds.
    pub city: City,
    /// The 3-hourly slots, 40 of them for a full 5 days.
    #[serde(default)]
    pub list: Vec<Slot>,
}

/// The `city` object of the forecast response.
#[derive(Debug, Clone, Deserialize)]
pub struct City {
    /// The location's offset from UTC in seconds.
    pub timezone: i32,
    /// The place name upstream matched.
    #[serde(default)]
    pub name: String,
}

/// One 3-hourly forecast slot.
#[derive(Debug, Clone, Deserialize)]
pub struct Slot {
    /// Slot time, unix UTC seconds.
    pub dt: i64,
    /// Temperature, humidity and pressure.
    pub main: Main,
    /// Conditions, most significant first.
    #[serde(default)]
    pub weather: Vec<Weather>,
    /// Wind speed, direction and gust.
    pub wind: Wind,
    /// Cloud cover.
    pub clouds: Clouds,
    /// Horizontal visibility in metres.
    #[serde(default)]
    pub visibility: Option<f32>,
    /// Rain over the slot, when it rained.
    #[serde(default)]
    pub rain: Option<Precip>,
    /// Snow over the slot, when it snowed.
    #[serde(default)]
    pub snow: Option<Precip>,
    /// Probability of precipitation, 0–1.
    #[serde(default)]
    pub pop: Option<f32>,
    /// Day/night flag (`d`/`n`).
    #[serde(default)]
    pub sys: Option<SlotSys>,
}

/// The `sys` object of a forecast slot.
#[derive(Debug, Clone, Deserialize)]
pub struct SlotSys {
    /// Part of the day: `d` or `n`.
    #[serde(default)]
    pub pod: String,
}

/// The primary condition of an endpoint's `weather` array.
#[derive(Debug, Clone, Deserialize)]
pub struct Weather {
    /// `OpenWeatherMap`'s condition id.
    pub id: u16,
    /// Icon id; its trailing letter is the day/night variant.
    #[serde(default)]
    pub icon: String,
}

/// Temperature, humidity and pressure, as both endpoints spell them.
#[derive(Debug, Clone, Deserialize)]
pub struct Main {
    /// Air temperature in °C under `units=metric`.
    pub temp: f32,
    /// Apparent temperature in °C.
    #[serde(default)]
    pub feels_like: Option<f32>,
    /// Relative humidity in percent.
    pub humidity: f32,
    /// Sea level pressure in hPa.
    pub pressure: f32,
}

/// Wind, as both endpoints spell it (m/s under `units=metric`).
#[derive(Debug, Clone, Deserialize)]
pub struct Wind {
    /// Wind speed in m/s.
    pub speed: f32,
    /// Direction the wind blows from, in degrees; absent when the wind is calm.
    #[serde(default)]
    pub deg: Option<f32>,
    /// Gust speed in m/s.
    #[serde(default)]
    pub gust: Option<f32>,
}

/// Cloud cover.
#[derive(Debug, Clone, Deserialize)]
pub struct Clouds {
    /// Cloudiness in percent.
    pub all: f32,
}

/// A precipitation block; the accumulation key differs per endpoint (`1h` vs `3h`).
#[derive(Debug, Clone, Deserialize)]
pub struct Precip {
    /// The last hour, in mm (current conditions).
    #[serde(rename = "1h", default)]
    pub one_h: Option<f32>,
    /// The last three hours, in mm (forecast slots).
    #[serde(rename = "3h", default)]
    pub three_h: Option<f32>,
}

// ---------------------------------------------------------------------------------------------
// Response → canonical model
// ---------------------------------------------------------------------------------------------

/// The current conditions; `None` when a value the canonical model requires is missing.
fn current_of(response: &CurrentResponse, tz: Tz, env: &Env<'_>) -> Option<Current> {
    let observed_at = local_time(response.dt, tz).fixed_offset();
    let weather = response.weather.first()?;

    Some(Current {
        observed_at,
        temp_c: response.main.temp,
        feels_like_c: response.main.feels_like,
        humidity_pct: percent(response.main.humidity),
        precip_mm: accumulation(response.rain.as_ref(), response.snow.as_ref(), Window::Hour),
        weather: condition_of(weather.id),
        cloud_cover_pct: percent(response.clouds.all),
        pressure_hpa: response.main.pressure,
        wind_kmh: response.wind.speed * MS_TO_KMH,
        // A calm wind has no direction and OWM omits `deg`; the model has no "no direction" state.
        wind_dir_deg: degrees(response.wind.deg.unwrap_or(0.0)),
        wind_gust_kmh: response.wind.gust.map(|gust| gust * MS_TO_KMH),
        visibility_km: response.visibility.map(|metres| metres / 1000.0),
        // The 2.5 endpoints carry no UV index.
        uv_index: None,
        is_day: !weather.icon.ends_with('n'),
    })
    .inspect(|current| {
        if env.verbose > 0 {
            eprintln!(
                "provider: {PROVIDER} current at {} (offset {:+}s)",
                current.observed_at.to_rfc3339(),
                response.timezone
            );
        }
    })
}

/// The forecast slots as canonical days.
fn days_of(
    response: &ForecastResponse,
    tz: Tz,
    days: u8,
) -> Result<Vec<crate::model::DayForecast>> {
    let samples = samples(response, tz);
    if samples.is_empty() {
        return Err(Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: "the response has no usable forecast slot".to_owned(),
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

/// The usable slots, in canonical units and the location's zone.
fn samples(response: &ForecastResponse, tz: Tz) -> Vec<HourSample> {
    response
        .list
        .iter()
        .filter_map(|slot| {
            let weather = slot.weather.first()?;
            Some(HourSample {
                at: local_time(slot.dt, tz),
                temp_c: slot.main.temp,
                feels_like_c: slot.main.feels_like,
                precip_mm: accumulation(slot.rain.as_ref(), slot.snow.as_ref(), Window::ThreeHours),
                precip_prob_pct: slot.pop.map(|pop| percent(pop * 100.0)),
                weather: condition_of(weather.id),
                wind_kmh: slot.wind.speed * MS_TO_KMH,
                wind_dir_deg: slot.wind.deg.map(degrees),
                humidity_pct: Some(percent(slot.main.humidity)),
                visibility_km: slot.visibility.map(|metres| metres / 1000.0),
            })
        })
        .collect()
}

/// Which accumulation window a precipitation block carries.
#[derive(Debug, Clone, Copy)]
enum Window {
    /// `rain.1h` / `snow.1h` — the current-conditions endpoint.
    Hour,
    /// `rain.3h` / `snow.3h` — one forecast slot.
    ThreeHours,
}

/// The precipitation total of one block, in mm; absent blocks are zero.
fn accumulation(rain: Option<&Precip>, snow: Option<&Precip>, window: Window) -> f32 {
    let value = |block: Option<&Precip>| match (block, window) {
        (Some(block), Window::Hour) => block.one_h,
        (Some(block), Window::ThreeHours) => block.three_h,
        (None, _) => None,
    };
    value(rain).unwrap_or(0.0) + value(snow).unwrap_or(0.0)
}

/// A unix UTC instant in the location's zone.
fn local_time(unix_seconds: i64, tz: Tz) -> DateTime<Tz> {
    let utc = DateTime::<Utc>::from_timestamp(unix_seconds, 0)
        .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default());
    utc.with_timezone(&tz)
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

/// `OpenWeatherMap`'s condition id as a WMO 4677 code.
///
/// The mapping is lossy on purpose (the plan's note: `2xx` thunder, `7xx` atmosphere → fog), and a
/// code outside the published families stays unknown rather than being clamped into a neighbour.
fn condition_of(id: u16) -> Condition {
    Condition::from_u8(match id {
        200..=232 => 95,       // thunderstorm family, with or without rain/drizzle
        300 => 51,             // light intensity drizzle
        301 | 310..=321 => 53, // drizzle, drizzle rain, shower drizzle
        302 => 55,             // heavy intensity drizzle
        500 => 61,             // light rain
        501 => 63,             // moderate rain
        502..=504 => 65,       // heavy, very heavy and extreme rain
        511 | 611..=616 => 66, // freezing rain, sleet and rain-and-snow mixes
        520 => 80,             // light intensity shower rain
        521 => 81,             // shower rain
        522 | 531 => 82,       // heavy and ragged shower rain
        600 => 71,             // light snow
        601 => 73,             // snow
        602 => 75,             // heavy snow
        620..=622 => 85,       // light, normal and heavy shower snow
        701..=781 => 45,       // mist, smoke, haze, fog, sand, dust, ash, squalls, tornado
        800 => 0,              // clear sky
        801 => 1,              // few clouds (11–25 %)
        802 => 2,              // scattered clouds (25–50 %)
        803 | 804 => 3,        // broken and overcast clouds
        _ => 255,              // undescribed: `Condition::from_u8` keeps 255 as unknown
    })
}

#[cfg(test)]
mod tests {
    use super::condition_of;

    /// Every condition id the provider documents, so a missing family arm cannot hide.
    const PUBLISHED: [u16; 49] = [
        200, 201, 202, 210, 211, 212, 221, 230, 231, 232, 300, 301, 302, 310, 311, 312, 313, 314,
        321, 500, 501, 502, 503, 504, 511, 520, 521, 522, 531, 600, 601, 602, 611, 612, 613, 615,
        616, 620, 621, 622, 701, 711, 721, 731, 741, 751, 761, 762, 771,
    ];

    #[test]
    fn every_published_code_maps_to_a_described_condition() {
        for id in PUBLISHED {
            let condition = condition_of(id);
            assert!(
                condition.is_known(),
                "{id} maps to an undescribed condition"
            );
        }
        // 781 (tornado) is published too; it shares the atmosphere family.
        assert!(condition_of(781).is_known());
    }

    #[test]
    fn an_unlisted_code_stays_unknown() {
        assert!(!condition_of(999).is_known());
        assert!(!condition_of(0).is_known());
    }

    #[test]
    fn the_families_follow_the_documented_groups() {
        assert_eq!(condition_of(200).description_en(), "Thunderstorm");
        assert_eq!(condition_of(300).description_en(), "Light drizzle");
        assert_eq!(condition_of(502).description_en(), "Heavy rain");
        assert_eq!(condition_of(511).description_en(), "Light freezing rain");
        assert_eq!(condition_of(600).description_en(), "Slight snow fall");
        assert_eq!(condition_of(741).description_en(), "Fog");
        assert_eq!(condition_of(800).description_en(), "Clear sky");
        assert_eq!(condition_of(804).description_en(), "Overcast");
    }
}
