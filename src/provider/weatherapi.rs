// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! WeatherAPI.com: one call per fetch, hourly data for the free plan's three days.
//!
//! What this module knows that the payload does not spell out:
//!
//! 1. **`tz_id` is an IANA zone name**, so a provisional location is corrected from the response
//!    (like Open-Meteo) instead of being refused.
//! 2. **`*_epoch` fields are the ones to compute with**; the string timestamps are local wall clock
//!    without an offset, and the hour strings are unpadded.
//! 3. **`astro.sunrise`/`sunset` are 12-hour clock strings** (`"06:10 AM"`) with no date, so they are
//!    joined to the day's local date.
//! 4. **`lang` is pinned to `en`**: the provider's translation is deliberately unused, our Fluent
//!    catalogs own the wording, and the condition codes are what map.
//! 5. **The free plan is three days and 100 000 calls/month**; `days` above the plan's horizon is
//!    refused by the provider, so the registry's `max_days: 3` is the clamp.
//! 6. **The terms add obligations beyond attribution**: a mandatory end-user disclaimer, a 60-minute
//!    cache ceiling for current conditions and 24 hours for forecast data, no resale, one key per
//!    application (see `docs/providers.md`; the cache TTL comes from `[cache]`, so the defaults sit
//!    far inside the ceiling).
//!
//! Licence: proprietary (Zoomash Ltd). Free keys must credit WeatherAPI.com by name or logo; the
//! registry row carries the line the renderers print.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone as _, Utc};
use chrono_tz::Tz;
use serde::Deserialize;

use super::dayparts::{HourSample, aggregate_day};
use super::{
    Capabilities, Env, FetchRequest, JsonFetch, Provider, ProviderId, fetch_json, local_today,
    requested_days,
};
use crate::cache::CacheKey;
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::{Attribution, Condition, Current, Location, LocationSource, Report};

/// The provider id, as the registry and every error message spell it.
const PROVIDER: &str = "weatherapi";

/// The forecast endpoint; `key`, `q`, `days` and `lang` go in the query string.
pub const FORECAST_URL: &str = "https://api.weatherapi.com/v1/forecast.json";

/// The `WeatherAPI` backend. Stateless: one value serves every fetch.
#[derive(Debug, Clone, Copy, Default)]
pub struct WeatherApi;

impl Provider for WeatherApi {
    fn id(&self) -> ProviderId {
        ProviderId::WeatherApi
    }

    fn capabilities(&self) -> Capabilities {
        ProviderId::WeatherApi.metadata().capabilities()
    }

    fn fetch(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
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
        // The endpoint always wants a `days` value; a current-only request asks for one day and
        // ignores the forecast block.
        let request = HttpRequest::get(FORECAST_URL)
            .query("key", &key)
            .query("q", format!("{:.4},{:.4}", loc.lat, loc.lon))
            .query("days", days.max(1).to_string())
            .query("lang", "en")
            .secret(&key);

        let response: Forecast = fetch_json(
            env,
            &JsonFetch {
                provider: ProviderId::WeatherApi,
                request: request.clone(),
                key: CacheKey::weather(PROVIDER, loc.lat, loc.lon, days, local_today(env, loc.tz)),
                ttl,
                what: "forecast",
            },
        )?;

        report(&response, loc, request.full_url(), days, env)
    }
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// `forecast.json`, in the subset this backend consumes.
#[derive(Debug, Clone, Deserialize)]
pub struct Forecast {
    /// The resolved place, including its IANA zone.
    pub location: LocationBlock,
    /// Current conditions.
    pub current: CurrentBlock,
    /// The forecast days.
    pub forecast: ForecastDays,
}

/// The `location` object.
#[derive(Debug, Clone, Deserialize)]
pub struct LocationBlock {
    /// Place name.
    #[serde(default)]
    pub name: String,
    /// Region or state.
    #[serde(default)]
    pub region: String,
    /// Country name.
    #[serde(default)]
    pub country: String,
    /// IANA zone name.
    pub tz_id: String,
    /// Latitude upstream matched.
    pub lat: f64,
    /// Longitude upstream matched.
    pub lon: f64,
    /// Local time as unix seconds.
    pub localtime_epoch: i64,
}

/// The `current` object.
#[derive(Debug, Clone, Deserialize)]
pub struct CurrentBlock {
    /// Observation time as unix seconds.
    pub last_updated_epoch: i64,
    /// Air temperature in °C.
    pub temp_c: f32,
    /// Apparent temperature in °C.
    pub feelslike_c: f32,
    /// Relative humidity in percent.
    pub humidity: f32,
    /// Sea level pressure in hPa.
    pub pressure_mb: f32,
    /// Wind speed in km/h.
    pub wind_kph: f32,
    /// Direction the wind blows from, in degrees.
    pub wind_degree: f32,
    /// Gust speed in km/h.
    #[serde(default)]
    pub gust_kph: Option<f32>,
    /// Horizontal visibility in km.
    #[serde(default)]
    pub vis_km: Option<f32>,
    /// UV index.
    #[serde(default)]
    pub uv: Option<f32>,
    /// Precipitation in the last hour, in mm.
    #[serde(default)]
    pub precip_mm: f32,
    /// `1` in daylight, `0` at night.
    pub is_day: u8,
    /// Total cloud cover in percent.
    #[serde(default)]
    pub cloud: Option<f32>,
    /// The primary condition.
    pub condition: ConditionBlock,
}

/// A `condition` object.
#[derive(Debug, Clone, Deserialize)]
pub struct ConditionBlock {
    /// `WeatherAPI`'s condition code.
    pub code: u16,
}

/// The `forecast` object.
#[derive(Debug, Clone, Deserialize)]
pub struct ForecastDays {
    /// One entry per day, oldest first.
    #[serde(default)]
    pub forecastday: Vec<ForecastDay>,
}

/// One `forecastday` entry.
#[derive(Debug, Clone, Deserialize)]
pub struct ForecastDay {
    /// The local date (`YYYY-MM-DD`).
    pub date: String,
    /// Daily aggregates.
    pub day: DayBlock,
    /// Sun and moon times.
    #[serde(default)]
    pub astro: Option<AstroBlock>,
    /// The day's hours.
    #[serde(default)]
    pub hour: Vec<HourBlock>,
}

/// The `day` object.
#[derive(Debug, Clone, Deserialize)]
pub struct DayBlock {
    /// Maximum temperature in °C.
    pub maxtemp_c: f32,
    /// Minimum temperature in °C.
    pub mintemp_c: f32,
    /// The day's primary condition.
    pub condition: ConditionBlock,
    /// UV index.
    #[serde(default)]
    pub uv: Option<f32>,
}

/// The `astro` object.
#[derive(Debug, Clone, Deserialize)]
pub struct AstroBlock {
    /// Sunrise, 12-hour clock.
    #[serde(default)]
    pub sunrise: Option<String>,
    /// Sunset, 12-hour clock.
    #[serde(default)]
    pub sunset: Option<String>,
}

/// One `hour` entry.
#[derive(Debug, Clone, Deserialize)]
pub struct HourBlock {
    /// The hour as unix seconds.
    pub time_epoch: i64,
    /// Air temperature in °C.
    pub temp_c: f32,
    /// Apparent temperature in °C.
    pub feelslike_c: f32,
    /// Relative humidity in percent.
    #[serde(default)]
    pub humidity: Option<f32>,
    /// Chance of rain in percent.
    #[serde(default)]
    pub chance_of_rain: Option<f32>,
    /// Precipitation in mm.
    #[serde(default)]
    pub precip_mm: f32,
    /// Wind speed in km/h.
    pub wind_kph: f32,
    /// Direction the wind blows from, in degrees.
    pub wind_degree: f32,
    /// Gust speed in km/h.
    #[serde(default)]
    pub gust_kph: Option<f32>,
    /// Horizontal visibility in km.
    #[serde(default)]
    pub vis_km: Option<f32>,
    /// The hour's condition.
    pub condition: ConditionBlock,
}

// ---------------------------------------------------------------------------------------------
// Response → canonical model
// ---------------------------------------------------------------------------------------------

/// Turns one response into a [`Report`], correcting the location's zone from `tz_id`.
fn report(
    response: &Forecast,
    loc: &Location,
    url: String,
    days: u8,
    env: &Env<'_>,
) -> Result<Report> {
    let tz = response
        .location
        .tz_id
        .parse::<Tz>()
        .map_err(|_| Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!("`{}` is not a known time zone", response.location.tz_id),
        })?;

    let mut location = loc.clone();
    if matches!(
        loc.source,
        LocationSource::Coordinates | LocationSource::Osm
    ) {
        // A coordinate or an OSM place carries a provisional zone; the provider's answer replaces
        // it, and the header then prints the zone the data is in.
        location.tz = tz;
    }

    let current = Some(current_of(&response.current, tz));

    let mut forecasts = Vec::new();
    if days > 0 {
        for day in &response.forecast.forecastday {
            forecasts.push(day_of(day, tz)?);
        }
        if forecasts.is_empty() {
            return Err(Error::Upstream {
                provider: PROVIDER.to_owned(),
                status: None,
                message: "the response has no forecast day".to_owned(),
            });
        }
    }

    Ok(Report {
        location,
        current,
        days: forecasts,
        attribution: Attribution {
            provider: PROVIDER.to_owned(),
            url,
            fetched_at: env.cache.clock().now().into(),
            raw: (env.verbose > 0).then(|| {
                format!(
                    "upstream matched {} ({:.2},{:.2}) in {}; local time epoch {}",
                    response.location.name,
                    response.location.lat,
                    response.location.lon,
                    response.location.tz_id,
                    response.location.localtime_epoch
                )
            }),
        },
    })
}

/// The current conditions.
fn current_of(block: &CurrentBlock, tz: Tz) -> Current {
    Current {
        observed_at: local_time(block.last_updated_epoch, tz).fixed_offset(),
        temp_c: block.temp_c,
        feels_like_c: Some(block.feelslike_c),
        humidity_pct: percent(block.humidity),
        precip_mm: block.precip_mm,
        weather: condition_of(block.condition.code),
        cloud_cover_pct: percent(block.cloud.unwrap_or(0.0)),
        pressure_hpa: block.pressure_mb,
        wind_kmh: block.wind_kph,
        wind_dir_deg: degrees(block.wind_degree),
        wind_gust_kmh: block.gust_kph,
        visibility_km: block.vis_km,
        uv_index: block.uv,
        is_day: block.is_day == 1,
    }
}

/// One forecast day: the daily aggregates and sun times, and the four parts from its hours.
fn day_of(day: &ForecastDay, tz: Tz) -> Result<crate::model::DayForecast> {
    let date = NaiveDate::parse_from_str(day.date.trim(), "%Y-%m-%d").map_err(|error| {
        Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!("`{}` is not a calendar date: {error}", day.date),
        }
    })?;

    let samples: Vec<HourSample> = day.hour.iter().map(|hour| sample(hour, tz)).collect();
    if samples.is_empty() {
        return Err(Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!("the response has no hours for {date}"),
        });
    }

    let sunrise = astro_time(
        day.astro.as_ref().and_then(|a| a.sunrise.as_deref()),
        date,
        tz,
    );
    let sunset = astro_time(
        day.astro.as_ref().and_then(|a| a.sunset.as_deref()),
        date,
        tz,
    );

    aggregate_day(
        &samples,
        date,
        tz,
        PROVIDER,
        day.day.mintemp_c,
        day.day.maxtemp_c,
        sunrise,
        sunset,
    )
}

/// One hour as a sample.
fn sample(hour: &HourBlock, tz: Tz) -> HourSample {
    HourSample {
        at: local_time(hour.time_epoch, tz),
        temp_c: hour.temp_c,
        feels_like_c: Some(hour.feelslike_c),
        precip_mm: hour.precip_mm,
        precip_prob_pct: hour.chance_of_rain.map(percent),
        weather: condition_of(hour.condition.code),
        wind_kmh: hour.wind_kph,
        wind_dir_deg: Some(degrees(hour.wind_degree)),
        humidity_pct: hour.humidity.map(percent),
        visibility_km: hour.vis_km,
    }
}

/// A unix UTC instant in the location's zone.
fn local_time(unix_seconds: i64, tz: Tz) -> DateTime<Tz> {
    let utc = DateTime::<Utc>::from_timestamp(unix_seconds, 0)
        .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default());
    utc.with_timezone(&tz)
}

/// A `"06:10 AM"` sun time joined to `date`; absent, empty and placeholder values become `None`.
fn astro_time(
    text: Option<&str>,
    date: NaiveDate,
    tz: Tz,
) -> Option<DateTime<chrono::FixedOffset>> {
    let text = text?.trim();
    let time = NaiveTime::parse_from_str(text, "%I:%M %p").ok()?;
    tz.from_local_datetime(&date.and_time(time))
        .single()
        .map(|local| local.fixed_offset())
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

/// `WeatherAPI`'s condition code as a WMO 4677 code.
///
/// The families follow the provider's own code list (1000–1282); the mapping is lossy where WMO has
/// no equivalent, and an unlisted code stays unknown instead of being clamped into a neighbour.
///
/// The arms are deliberately kept one per published code (or per published family) even where two
/// map to the same WMO code: the comments carry the provider's own names, which is what a future
/// re-verification compares against.
#[allow(clippy::match_same_arms)]
fn condition_of(code: u16) -> Condition {
    Condition::from_u8(match code {
        1000 => 0, // sunny / clear
        1003 => 1, // partly cloudy
        1006 => 2, // cloudy
        1009 => 3, // overcast
        1012 | 1015 | 1018 | 1021 | 1024 | 1027 | 1030 | 1033 | 1036 | 1039 | 1042 | 1045
        | 1048 => 45, // haze, dust, sand, smoke, smog, mist
        1063 => 51, // patchy rain possible
        1066 => 71, // patchy snow possible
        1069 => 66, // patchy sleet possible
        1072 => 56, // patchy freezing drizzle possible
        1087 => 95, // thundery outbreaks possible
        1114 => 85, // blowing snow
        1117 => 75, // blizzard
        1135 => 45, // fog
        1147 => 48, // freezing fog
        1150 | 1153 => 51, // patchy light drizzle, light drizzle
        1168 | 1171 => 56, // freezing drizzle, heavy freezing drizzle
        1180 | 1183 => 61, // patchy light rain, light rain
        1186 | 1189 => 63, // moderate rain at times, moderate rain
        1192 | 1195 => 65, // heavy rain at times, heavy rain
        1198 => 66, // light freezing rain
        1201 => 67, // moderate or heavy freezing rain
        1204 | 1207 => 66, // light sleet, moderate or heavy sleet
        1210 | 1213 => 71, // patchy light snow, light snow
        1216 | 1219 => 73, // patchy moderate snow, moderate snow
        1222 | 1225 => 75, // patchy heavy snow, heavy snow
        1237 => 77, // ice pellets
        1240 | 1243 => 80, // light rain shower, moderate or heavy rain shower
        1246 => 82, // torrential rain shower
        1249 | 1252 => 66, // light sleet showers, moderate or heavy sleet showers
        1255 | 1258 => 85, // light snow showers, moderate or heavy snow showers
        1261 | 1264 => 77, // light / moderate or heavy showers of ice pellets
        1273 | 1276 => 95, // patchy light rain with thunder, heavy rain with thunder
        1279 | 1282 => 95, // light / heavy snow with thunder
        _ => 255,  // undescribed: `Condition::from_u8` keeps 255 as unknown
    })
}

#[cfg(test)]
mod tests {
    use super::condition_of;

    /// Every code the provider publishes, so a missing family arm cannot hide.
    const PUBLISHED: [u16; 53] = [
        1000, 1003, 1006, 1009, 1012, 1015, 1018, 1021, 1024, 1027, 1030, 1033, 1036, 1039, 1042,
        1045, 1048, 1063, 1066, 1069, 1072, 1087, 1114, 1117, 1135, 1147, 1150, 1153, 1168, 1171,
        1180, 1183, 1186, 1189, 1192, 1195, 1198, 1201, 1204, 1207, 1210, 1213, 1216, 1219, 1222,
        1225, 1237, 1240, 1243, 1246, 1249, 1252, 1255,
    ];

    #[test]
    fn every_published_code_maps_to_a_described_condition() {
        for code in PUBLISHED {
            let condition = condition_of(code);
            assert!(
                condition.is_known(),
                "{code} maps to an undescribed condition"
            );
        }
        // The remaining published codes: the shower and thunder tail of the list.
        for code in [1258, 1261, 1264, 1273, 1276, 1279, 1282] {
            assert!(condition_of(code).is_known(), "{code} is undescribed");
        }
    }

    #[test]
    fn an_unlisted_code_stays_unknown() {
        assert!(!condition_of(999).is_known());
        assert!(!condition_of(1001).is_known());
    }

    #[test]
    fn the_families_follow_the_published_groups() {
        assert_eq!(condition_of(1000).description_en(), "Clear sky");
        assert_eq!(condition_of(1003).description_en(), "Mainly clear");
        assert_eq!(condition_of(1009).description_en(), "Overcast");
        assert_eq!(condition_of(1135).description_en(), "Fog");
        assert_eq!(condition_of(1147).description_en(), "Depositing rime fog");
        assert_eq!(condition_of(1183).description_en(), "Slight rain");
        assert_eq!(condition_of(1225).description_en(), "Heavy snow fall");
        assert_eq!(condition_of(1276).description_en(), "Thunderstorm");
    }
}
