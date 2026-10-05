// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Visual Crossing's Timeline API: the BYOK backend that also carries its own weather alerts.
//!
//! One request per fetch against the timeline endpoint, asking for metric values, the current
//! conditions, the daily blocks and their hours:
//!
//! ```text
//! …/timeline/<lat>,<lon>/next<days>days?unitGroup=metric&include=current,days,hours&key=<KEY>
//! ```
//!
//! What this module knows that the payload does not spell out:
//!
//! 1. **`next<days>days` is the dynamic-period keyword** the API documents; if the server rejects
//!    it with a `400` the backend retries once with an explicit `date1`/`date2` range computed from
//!    the location's local today (the same span, so the same positions). Only if that also fails is
//!    the `400` reported.
//! 2. **`datetimeEpoch` is authoritative.** The API returns local wall-clock `datetime` values —
//!    a bare `HH:MM:SS` for the hourly records (the date lives in the parent day) and for
//!    `currentConditions` — alongside a UTC epoch, which is the only unambiguous instant the
//!    payload carries. The epoch is preferred; the text is the fallback, and a text-only hour is
//!    joined to its parent day's date before being resolved in the location's zone.
//! 3. **`timezone` names the zone the timestamps are in.** A location resolved from raw coordinates
//!    or OSM carries a provisional `UTC`; the response replaces it, exactly as it does for
//!    Open-Meteo.
//! 4. **`icon` is the machine-readable condition.** The fixed vocabulary below maps to WMO 4677;
//!    an icon outside it becomes WMO 3 ("Overcast") and one `--verbose` line names it, matching
//!    the other backends' unknown-code rule.
//! 5. **`alerts[]` carries the payload's own warnings.** The published alert object has five
//!    properties (`event`, `headline`, `description`, `onset`, `ends`); this decoder also reads the
//!    CAP triple (`severity`, `urgency`, `certainty`) and the optional `expires` when a source
//!    supplies them, mapping each through the model's own `from_cap` parsers so a vendor spelling
//!    this build does not know degrades to `Unknown` instead of being invented. An alert whose
//!    timestamp cannot be read is skipped with a `--verbose` note, never a hard failure — a warning
//!    is worth showing even when one source's dates are malformed.
//! 6. **`is_day` comes from `currentConditions.solarradiation`** (zero at night), the payload's own
//!    daylight signal; when it is absent the local civil day is the stand-in.
//!
//! Licence: per-account Visual Crossing terms (local display only, no redistribution of cached
//! data); the registry row carries the credit line the renderers print.

use std::collections::BTreeSet;
use std::time::Duration;

use chrono::{
    DateTime, Days, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Timelike as _, Utc,
};
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
use crate::model::{
    Alert, AlertSource, Certainty, Condition, Current, Location, LocationSource, Report,
    ReportMode, Severity, Urgency, resolve_local,
};

/// The provider id, as the registry and every error message spell it.
const PROVIDER: &str = "visualcrossing";

/// The Timeline API root; the path continues with `<lat>,<lon>/<period>`.
pub const BASE: &str =
    "https://weather.visualcrossing.com/VisualCrossingWebServices/rest/services/timeline";

/// What to include when a forecast span is requested.
const INCLUDE_FORECAST: &str = "current,days,hours";

/// What to include for a current-conditions-only request.
const INCLUDE_CURRENT: &str = "current";

/// The `unitGroup` the request pins; the canonical model is metric, so the API must be too.
const UNIT_GROUP: &str = "metric";

/// The fixed `icon` vocabulary and the WMO 4677 code each icon stands for.
///
/// The list is the one the plan fixes; an icon outside it is not a condition this model can name,
/// so it becomes [`UNKNOWN_ICON_CODE`] and a `--verbose` line names it.
const ICONS: [(&str, u8); 16] = [
    ("clear-day", 0),
    ("clear-night", 0),
    ("partly-cloudy-day", 2),
    ("partly-cloudy-night", 2),
    ("cloudy", 3),
    ("wind", 3),
    ("fog", 45),
    ("rain", 63),
    ("showers-day", 81),
    ("showers-night", 81),
    ("snow", 73),
    ("sleet", 68),
    ("freezing-rain", 66),
    ("hail", 96),
    ("thunderstorm", 95),
    ("thunder-rain", 95),
];

/// The WMO code an icon outside [`ICONS`] maps to: "Overcast", the neutral degraded condition.
const UNKNOWN_ICON_CODE: u8 = 3;

/// The Visual Crossing backend. Stateless: one value serves every fetch.
#[derive(Debug, Clone, Copy, Default)]
pub struct VisualCrossing;

impl Provider for VisualCrossing {
    fn id(&self) -> ProviderId {
        ProviderId::VisualCrossing
    }

    fn capabilities(&self) -> Capabilities {
        ProviderId::VisualCrossing.metadata().capabilities()
    }

    fn fetch_report(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
        // The registry declares no archive (`history_days: 0`), so the CLI refuses `--date` and
        // `--history` before this point; this is the backstop for a hand-built request.
        if req.window.is_some() {
            return Err(Error::Usage(
                "visualcrossing has no archive; drop --date/--history".to_owned(),
            ));
        }

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

        let today = local_today(env, loc.tz);
        let cache_key = CacheKey::weather(PROVIDER, loc.lat, loc.lon, days, today);
        let ttl = Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs));

        let (response, url) = timeline(env, loc, days, &key, today, cache_key, ttl)?;
        report(&response, loc, url, days, env)
    }
}

/// Fetches one timeline answer, falling back to an explicit date range when the server rejects the
/// `next<days>days` keyword.
///
/// Returns the decoded response and the redacted URL it came from, so the attribution names the
/// request that actually answered.
fn timeline(
    env: &Env<'_>,
    loc: &Location,
    days: u8,
    key: &str,
    today: NaiveDate,
    cache_key: CacheKey,
    ttl: Duration,
) -> Result<(TimelineResponse, String)> {
    let request = timeline_request(loc, days, key);
    let fetch = JsonFetch {
        provider: ProviderId::VisualCrossing,
        request: request.clone(),
        key: cache_key.clone(),
        ttl,
        what: "timeline",
    };
    match fetch_json(env, loc, &fetch) {
        Ok(response) => Ok((response, request.redacted_url())),
        Err(error) if days > 0 && rejected_keyword(&error) => {
            if env.verbose > 0 && !env.quiet {
                eprintln!(
                    "note: {PROVIDER} rejected `next{days}days`; retrying with an explicit date range"
                );
            }
            let fallback = window_request(loc, days, today, key);
            let fetch = JsonFetch {
                provider: ProviderId::VisualCrossing,
                request: fallback.clone(),
                key: cache_key,
                ttl,
                what: "timeline",
            };
            let response = fetch_json(env, loc, &fetch)?;
            Ok((response, fallback.redacted_url()))
        }
        Err(error) => Err(error),
    }
}

/// Whether an error is the server refusing the request's shape (`400`), which is the one failure
/// the explicit date range can answer.
fn rejected_keyword(error: &Error) -> bool {
    matches!(
        error,
        Error::Upstream {
            status: Some(400),
            ..
        }
    )
}

/// The request for a forecast span, assembled in a fixed parameter order (the tests assert the URL
/// verbatim). `days == 0` asks for the current conditions only.
fn timeline_request(loc: &Location, days: u8, key: &str) -> HttpRequest {
    let period = if days > 0 {
        format!("next{days}days")
    } else {
        "today".to_owned()
    };
    let include = if days > 0 {
        INCLUDE_FORECAST
    } else {
        INCLUDE_CURRENT
    };
    HttpRequest::get(format!("{BASE}/{:.4},{:.4}/{period}", loc.lat, loc.lon))
        .query("unitGroup", UNIT_GROUP)
        .query("include", include)
        .query("key", key)
        .secret(key)
}

/// The fallback request: the same span spelled as `date1`/`date2`, both `YYYY-MM-DD`.
fn window_request(loc: &Location, days: u8, today: NaiveDate, key: &str) -> HttpRequest {
    let date1 = today.format("%Y-%m-%d");
    let last = today + Days::new(u64::from(days.saturating_sub(1)));
    let date2 = last.format("%Y-%m-%d");
    HttpRequest::get(format!(
        "{BASE}/{:.4},{:.4}/{date1}/{date2}",
        loc.lat, loc.lon
    ))
    .query("unitGroup", UNIT_GROUP)
    .query("include", INCLUDE_FORECAST)
    .query("key", key)
    .secret(key)
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// The timeline response, in the subset `circoast` consumes.
///
/// Unknown fields are ignored on purpose: the API returns a large, documented element set and adds
/// to it regularly, and a new one never invalidates a cached body.
#[derive(Debug, Clone, Deserialize)]
pub struct TimelineResponse {
    /// The resolved place name, for the `-v` summary.
    #[serde(rename = "resolvedAddress", default)]
    pub resolved_address: String,
    /// The IANA zone the timestamps are expressed in.
    #[serde(default)]
    pub timezone: Option<String>,
    /// Current conditions.
    #[serde(rename = "currentConditions", default)]
    pub current_conditions: Option<CurrentBlock>,
    /// The daily blocks, each with its own hours when requested.
    #[serde(default)]
    pub days: Vec<DayBlock>,
    /// Active weather alerts for the requested location and period.
    #[serde(default)]
    pub alerts: Vec<AlertBlock>,
}

/// The `currentConditions` object.
#[derive(Debug, Clone, Deserialize)]
pub struct CurrentBlock {
    /// Local wall clock time (`HH:MM:SS`).
    #[serde(default)]
    pub datetime: Option<String>,
    /// The same instant as a UTC epoch, seconds.
    #[serde(rename = "datetimeEpoch", default)]
    pub datetime_epoch: Option<i64>,
    /// Air temperature in °C.
    #[serde(default)]
    pub temp: Option<f32>,
    /// Apparent temperature in °C.
    #[serde(default)]
    pub feelslike: Option<f32>,
    /// Relative humidity in percent.
    #[serde(default)]
    pub humidity: Option<f32>,
    /// Precipitation in the last period, in mm.
    #[serde(default)]
    pub precip: Option<f32>,
    /// Wind speed in km/h.
    #[serde(default)]
    pub windspeed: Option<f32>,
    /// Gust speed in km/h.
    #[serde(default)]
    pub windgust: Option<f32>,
    /// Direction the wind blows from, in degrees.
    #[serde(default)]
    pub winddir: Option<f32>,
    /// Sea level pressure in hPa.
    #[serde(default)]
    pub pressure: Option<f32>,
    /// Total cloud cover in percent.
    #[serde(default)]
    pub cloudcover: Option<f32>,
    /// Horizontal visibility in km.
    #[serde(default)]
    pub visibility: Option<f32>,
    /// Solar radiation in W/m²; zero at night, which is what `is_day` reads.
    #[serde(default)]
    pub solarradiation: Option<f32>,
    /// UV index.
    #[serde(default)]
    pub uvindex: Option<f32>,
    /// The condition icon.
    #[serde(default)]
    pub icon: Option<String>,
}

/// One `days[]` entry.
#[derive(Debug, Clone, Deserialize)]
pub struct DayBlock {
    /// The local calendar date (`YYYY-MM-DD`).
    #[serde(default)]
    pub datetime: String,
    /// Maximum temperature in °C.
    #[serde(default)]
    pub tempmax: Option<f32>,
    /// Minimum temperature in °C.
    #[serde(default)]
    pub tempmin: Option<f32>,
    /// Local sunrise (`HH:MM:SS`, or a full datetime on some responses).
    #[serde(default)]
    pub sunrise: Option<String>,
    /// Sunrise as a UTC epoch, seconds.
    #[serde(rename = "sunriseEpoch", default)]
    pub sunrise_epoch: Option<i64>,
    /// Local sunset.
    #[serde(default)]
    pub sunset: Option<String>,
    /// Sunset as a UTC epoch, seconds.
    #[serde(rename = "sunsetEpoch", default)]
    pub sunset_epoch: Option<i64>,
    /// The day's hourly records.
    #[serde(default)]
    pub hours: Vec<HourBlock>,
}

/// One `days[].hours[]` entry.
#[derive(Debug, Clone, Deserialize)]
pub struct HourBlock {
    /// Local wall clock time (`HH:MM:SS`; the date is the parent day's).
    #[serde(default)]
    pub datetime: Option<String>,
    /// The same instant as a UTC epoch, seconds.
    #[serde(rename = "datetimeEpoch", default)]
    pub datetime_epoch: Option<i64>,
    /// Air temperature in °C.
    #[serde(default)]
    pub temp: Option<f32>,
    /// Apparent temperature in °C.
    #[serde(default)]
    pub feelslike: Option<f32>,
    /// Precipitation over the hour, in mm.
    #[serde(default)]
    pub precip: Option<f32>,
    /// Probability of precipitation in percent.
    #[serde(default)]
    pub precipprob: Option<f32>,
    /// Wind speed in km/h.
    #[serde(default)]
    pub windspeed: Option<f32>,
    /// Direction the wind blows from, in degrees.
    #[serde(default)]
    pub winddir: Option<f32>,
    /// Relative humidity in percent.
    #[serde(default)]
    pub humidity: Option<f32>,
    /// Horizontal visibility in km.
    #[serde(default)]
    pub visibility: Option<f32>,
    /// The condition icon.
    #[serde(default)]
    pub icon: Option<String>,
}

/// One `alerts[]` entry.
#[derive(Debug, Clone, Deserialize)]
pub struct AlertBlock {
    /// The source's own identifier, when the payload carries one.
    #[serde(default)]
    pub id: Option<String>,
    /// The type of alert, e.g. `Flood Watch`.
    #[serde(default)]
    pub event: Option<String>,
    /// A short description.
    #[serde(default)]
    pub headline: Option<String>,
    /// The full description.
    #[serde(default)]
    pub description: Option<String>,
    /// When the event starts, at its own offset (or local).
    #[serde(default)]
    pub onset: Option<String>,
    /// `onset` as a UTC epoch, seconds.
    #[serde(rename = "onsetEpoch", default)]
    pub onset_epoch: Option<i64>,
    /// When the event ends, at its own offset (or local).
    #[serde(default)]
    pub ends: Option<String>,
    /// `ends` as a UTC epoch, seconds.
    #[serde(rename = "endsEpoch", default)]
    pub ends_epoch: Option<i64>,
    /// The message's validity end, when the source carries one.
    #[serde(default)]
    pub expires: Option<String>,
    /// `expires` as a UTC epoch, seconds.
    #[serde(rename = "expiresEpoch", default)]
    pub expires_epoch: Option<i64>,
    /// CAP `severity`.
    #[serde(default)]
    pub severity: Option<String>,
    /// CAP `urgency`.
    #[serde(default)]
    pub urgency: Option<String>,
    /// CAP `certainty`.
    #[serde(default)]
    pub certainty: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Response → canonical model
// ---------------------------------------------------------------------------------------------

/// Turns one response into a [`Report`], correcting the location's zone when it was provisional.
fn report(
    response: &TimelineResponse,
    loc: &Location,
    url: String,
    days: u8,
    env: &Env<'_>,
) -> Result<Report> {
    let tz = response_zone(response, loc)?;
    let mut location = loc.clone();
    if matches!(
        loc.source,
        LocationSource::Coordinates | LocationSource::Osm
    ) {
        // A coordinate or an OSM place has no zone of its own (`UTC` was a placeholder); the
        // response's `timezone` replaces it, and the header then prints the zone the data is in.
        location.tz = tz;
    }

    let block = response
        .current_conditions
        .as_ref()
        .ok_or_else(|| upstream("the response has no `currentConditions` block".to_owned()))?;
    let current = current_of(block, tz)?;

    let mut forecasts = Vec::new();
    if days > 0 {
        let samples = samples(response, tz)?;
        if samples.is_empty() {
            return Err(upstream(
                "the response carries no usable hourly record".to_owned(),
            ));
        }
        for date in covered_days(&samples, tz, days) {
            let day = response
                .days
                .iter()
                .find(|day| parse_date(&day.datetime).ok() == Some(date));
            let (temp_min_c, temp_max_c) = day
                .and_then(|day| day.tempmin.zip(day.tempmax))
                .map_or_else(
                    || extremes(&samples, date, tz, PROVIDER),
                    |(min, max)| Ok((min, max)),
                )?;
            let (sunrise, sunset) = match day {
                Some(day) => (
                    event(
                        day.sunrise_epoch,
                        day.sunrise.as_deref(),
                        date,
                        tz,
                        "sunrise",
                    )?,
                    event(day.sunset_epoch, day.sunset.as_deref(), date, tz, "sunset")?,
                ),
                None => (None, None),
            };
            forecasts.push(aggregate_day(
                &samples, date, tz, PROVIDER, temp_min_c, temp_max_c, sunrise, sunset,
            )?);
        }
        if forecasts.is_empty() {
            return Err(upstream(format!(
                "the response covers no complete local day in {tz}"
            )));
        }
        note_short_series(forecasts.len(), days, PROVIDER, env);
    }

    if env.verbose > 0 && !env.quiet {
        for icon in unknown_icons(response) {
            eprintln!("note: {PROVIDER} has no WMO code for icon `{icon}`; showing overcast");
        }
    }

    Ok(Report {
        location,
        current: Some(current),
        days: forecasts,
        alerts: alerts_of(response, tz, env),
        air: None,
        astro: None,
        marine: None,
        mode: ReportMode::Forecast,
        attribution: attribution(
            ProviderId::VisualCrossing,
            url,
            env.cache.clock().now().into(),
            (env.verbose > 0).then(|| raw_summary(response)),
        ),
    })
}

/// The zone the response is expressed in, falling back to the location's own when absent.
fn response_zone(response: &TimelineResponse, loc: &Location) -> Result<Tz> {
    match response
        .timezone
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        Some(name) => name
            .parse::<Tz>()
            .map_err(|_| upstream(format!("`{name}` is not a known time zone"))),
        None => Ok(loc.tz),
    }
}

/// The current conditions, requiring every field the canonical model has no `Option` for.
fn current_of(block: &CurrentBlock, tz: Tz) -> Result<Current> {
    let observed_at = require_instant(
        block.datetime_epoch,
        block.datetime.as_deref(),
        None,
        tz,
        "currentConditions",
    )?;
    Ok(Current {
        observed_at: observed_at.fixed_offset(),
        temp_c: require(block.temp, "currentConditions.temp")?,
        feels_like_c: block.feelslike,
        humidity_pct: block.humidity.map(percent),
        precip_mm: block.precip.unwrap_or(0.0),
        weather: icon_condition(block.icon.as_deref().unwrap_or_default()),
        cloud_cover_pct: block.cloudcover.map(percent),
        pressure_hpa: require(block.pressure, "currentConditions.pressure")?,
        wind_kmh: require(block.windspeed, "currentConditions.windspeed")?,
        wind_dir_deg: block.winddir.map(degrees),
        wind_gust_kmh: block.windgust,
        // The API's `unitGroup=metric` reports visibility in kilometres, like the model.
        visibility_km: block.visibility,
        uv_index: block.uvindex,
        // Solar radiation is zero at night; without it the local civil day is the stand-in.
        is_day: match block.solarradiation {
            Some(radiation) => radiation > 0.0,
            None => matches!(observed_at.hour(), 6..=17),
        },
    })
}

/// Decodes every day's hourly records into usable samples.
///
/// An hour missing a value the canonical model cannot express as "missing" (temperature,
/// precipitation, wind speed) is dropped rather than filled with a zero; its condition is read
/// from `icon`.
fn samples(response: &TimelineResponse, tz: Tz) -> Result<Vec<HourSample>> {
    let mut samples = Vec::new();
    for day in &response.days {
        let date = parse_date(&day.datetime)?;
        for hour in &day.hours {
            let at = require_instant(
                hour.datetime_epoch,
                hour.datetime.as_deref(),
                Some(date),
                tz,
                "hours",
            )?;
            let (Some(temp_c), Some(precip_mm), Some(wind_kmh)) =
                (hour.temp, hour.precip, hour.windspeed)
            else {
                continue;
            };
            samples.push(HourSample {
                at,
                temp_c,
                feels_like_c: hour.feelslike,
                precip_mm,
                precip_prob_pct: hour.precipprob.map(percent),
                weather: icon_condition(hour.icon.as_deref().unwrap_or_default()),
                wind_kmh,
                wind_dir_deg: hour.winddir.map(degrees),
                humidity_pct: hour.humidity.map(percent),
                visibility_km: hour.visibility,
            });
        }
    }
    samples.sort_by_key(|sample| sample.at);
    Ok(samples)
}

/// Decodes the payload's alerts, skipping (with a `-v` note) any whose timestamps cannot be read.
fn alerts_of(response: &TimelineResponse, tz: Tz, env: &Env<'_>) -> Vec<Alert> {
    let mut alerts = Vec::new();
    for raw in &response.alerts {
        match alert_of(raw, tz) {
            Ok(alert) => alerts.push(alert),
            Err(reason) => {
                if env.verbose > 0 && !env.quiet {
                    eprintln!("note: {PROVIDER} skipped an alert: {reason}");
                }
            }
        }
    }
    alerts
}

/// One alert as the canonical model, or the reason it cannot be rendered.
///
/// `Err` is a skip reason, never a run failure: a malformed timestamp drops one warning, it does
/// not lose the forecast.
fn alert_of(raw: &AlertBlock, tz: Tz) -> std::result::Result<Alert, String> {
    let event = raw
        .event
        .as_deref()
        .map(str::trim)
        .filter(|event| !event.is_empty())
        .ok_or_else(|| "the alert carries no `event`".to_owned())?;
    let onset = alert_instant(raw.onset_epoch, raw.onset.as_deref(), tz, "onset")?;
    let ends = alert_instant(raw.ends_epoch, raw.ends.as_deref(), tz, "ends")?;
    let expires = alert_instant(raw.expires_epoch, raw.expires.as_deref(), tz, "expires")?;

    // Stable identity: the source's own id when it has one, else the event and its onset, so the
    // same warning deduplicates across runs and two different warnings never collide.
    let id = raw
        .id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map_or_else(
            || {
                format!(
                    "{PROVIDER}:{event}@{}",
                    onset.map_or_else(|| "none".to_owned(), |at| at.to_rfc3339())
                )
            },
            str::to_owned,
        );
    let headline = raw
        .headline
        .as_deref()
        .map(str::trim)
        .filter(|headline| !headline.is_empty())
        .unwrap_or(event)
        .to_owned();

    Ok(Alert {
        id,
        source: AlertSource::VisualCrossing,
        event: event.to_owned(),
        severity: Severity::from_cap(raw.severity.as_deref().unwrap_or_default()),
        urgency: Urgency::from_cap(raw.urgency.as_deref().unwrap_or_default()),
        certainty: Certainty::from_cap(raw.certainty.as_deref().unwrap_or_default()),
        onset,
        expires,
        ends,
        // The payload's alert object carries no area list.
        areas: Vec::new(),
        headline,
        description: raw.description.clone(),
        instruction: None,
        sender: None,
    })
}

/// One alert timestamp: the epoch when present, else the text at its own offset, else local in
/// `tz`; `None` when the field is absent, `Err` when it is present but unreadable.
fn alert_instant(
    epoch: Option<i64>,
    text: Option<&str>,
    tz: Tz,
    field: &str,
) -> std::result::Result<Option<DateTime<FixedOffset>>, String> {
    if let Some(seconds) = epoch {
        return DateTime::<Utc>::from_timestamp(seconds, 0)
            .map(|utc| Some(utc.fixed_offset()))
            .ok_or_else(|| format!("the `{field}` epoch {seconds} is out of range"));
    }
    let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    if let Some(at) = parse_offset(text) {
        return Ok(Some(at));
    }
    for format in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            // The API returns local wall-clock times without an offset; the response's zone is
            // what makes them instants.
            return resolve_local(tz, naive)
                .map(|at| Some(at.fixed_offset()))
                .map_err(|_| format!("the `{field}` timestamp `{text}` does not exist in {tz}"));
        }
    }
    Err(format!(
        "the `{field}` timestamp `{text}` is not a recognised date and time"
    ))
}

/// One instant from the payload: the epoch when present, else the text.
///
/// The text forms accepted are the CAP reader's offset spellings (a relaxed ISO 8601 datetime) plus
/// the two local forms this API emits — a full `YYYY-MM-DDTHH:MM[:SS]` and the bare `HH:MM[:SS]`
/// hourly value, which needs `date` (the parent day) to become an instant.
fn instant(
    epoch: Option<i64>,
    text: Option<&str>,
    date: Option<NaiveDate>,
    tz: Tz,
    field: &str,
) -> Result<Option<DateTime<Tz>>> {
    if let Some(seconds) = epoch {
        let Some(utc) = DateTime::<Utc>::from_timestamp(seconds, 0) else {
            return Err(upstream(format!(
                "the `{field}` epoch {seconds} is out of range"
            )));
        };
        return Ok(Some(utc.with_timezone(&tz)));
    }
    let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    if let Some(at) = parse_offset(text) {
        return Ok(Some(at.with_timezone(&tz)));
    }
    for format in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            return resolve_local(tz, naive).map(Some);
        }
    }
    if let Some(date) = date {
        for format in ["%H:%M:%S", "%H:%M"] {
            if let Ok(time) = NaiveTime::parse_from_str(text, format) {
                return resolve_local(tz, date.and_time(time)).map(Some);
            }
        }
    }
    Err(upstream(format!(
        "the `{field}` timestamp `{text}` is not a recognised date and time"
    )))
}

/// [`instant`] for a field the canonical model requires.
fn require_instant(
    epoch: Option<i64>,
    text: Option<&str>,
    date: Option<NaiveDate>,
    tz: Tz,
    field: &str,
) -> Result<DateTime<Tz>> {
    instant(epoch, text, date, tz, field)?
        .ok_or_else(|| upstream(format!("the response carries no `{field}` timestamp")))
}

/// One optional sun time, in the location's zone.
fn event(
    epoch: Option<i64>,
    text: Option<&str>,
    date: NaiveDate,
    tz: Tz,
    field: &str,
) -> Result<Option<DateTime<FixedOffset>>> {
    Ok(instant(epoch, text, Some(date), tz, field)?.map(|at| at.fixed_offset()))
}

/// The offset-bearing timestamp spellings the CAP reader accepts, shared so the two readers cannot
/// drift: a relaxed ISO 8601 datetime (`T` or a space, `+02:00` or `+0200`).
fn parse_offset(text: &str) -> Option<DateTime<FixedOffset>> {
    if let Ok(at) = text.parse::<DateTime<FixedOffset>>() {
        return Some(at);
    }
    for format in ["%Y-%m-%dT%H:%M%z", "%Y-%m-%d %H:%M%z"] {
        if let Ok(at) = DateTime::parse_from_str(text, format) {
            return Some(at);
        }
    }
    None
}

/// An icon as the canonical condition; an icon outside [`ICONS`] becomes [`UNKNOWN_ICON_CODE`].
fn icon_condition(icon: &str) -> Condition {
    let code = ICONS
        .iter()
        .find(|(name, _)| *name == icon)
        .map_or(UNKNOWN_ICON_CODE, |(_, code)| *code);
    Condition::from_u8(code)
}

/// Whether `icon` is one of the vocabulary's icons.
fn is_known_icon(icon: &str) -> bool {
    ICONS.iter().any(|(name, _)| *name == icon)
}

/// The distinct condition icons the response uses that [`ICONS`] does not name, for the `-v` note.
fn unknown_icons(response: &TimelineResponse) -> BTreeSet<&str> {
    let icons = response
        .current_conditions
        .iter()
        .filter_map(|block| block.icon.as_deref())
        .chain(
            response
                .days
                .iter()
                .flat_map(|day| day.hours.iter().filter_map(|hour| hour.icon.as_deref())),
        );
    icons
        .map(str::trim)
        .filter(|icon| !icon.is_empty() && !is_known_icon(icon))
        .collect()
}

/// Parses a `YYYY-MM-DD` date.
fn parse_date(text: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
        .map_err(|error| upstream(format!("`{text}` is not a calendar date: {error}")))
}

/// A required upstream value: `null` is an upstream error naming the field.
fn require<T>(value: Option<T>, field: &str) -> Result<T> {
    value.ok_or_else(|| upstream(format!("the response carries no `{field}`")))
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

/// The upstream error for a malformed response.
fn upstream(message: String) -> Error {
    Error::Upstream {
        provider: PROVIDER.to_owned(),
        status: None,
        message,
    }
}

/// What `-v` prints: the resolved place, the zone, the day/hour counts and the alert count.
fn raw_summary(response: &TimelineResponse) -> String {
    let hours: usize = response.days.iter().map(|day| day.hours.len()).sum();
    format!(
        "resolved {} timezone {} days {} hours {hours} alerts {}",
        response.resolved_address,
        response.timezone.as_deref().unwrap_or("unknown"),
        response.days.len(),
        response.alerts.len()
    )
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::{
        AlertBlock, ICONS, UNKNOWN_ICON_CODE, alert_of, icon_condition, is_known_icon,
        parse_offset, window_request,
    };
    use crate::model::{
        AlertSource, Certainty, Condition, Location, LocationSource, Severity, Urgency,
    };
    use chrono_tz::Tz;

    fn icon_codes() -> Vec<u8> {
        ICONS.iter().map(|(_, code)| *code).collect()
    }

    #[test]
    fn the_icon_table_covers_the_documented_vocabulary() {
        // Every icon the plan names maps to its WMO code, and every one of them is recognised.
        let expected = [
            ("clear-day", 0),
            ("clear-night", 0),
            ("partly-cloudy-day", 2),
            ("partly-cloudy-night", 2),
            ("cloudy", 3),
            ("wind", 3),
            ("fog", 45),
            ("rain", 63),
            ("showers-day", 81),
            ("showers-night", 81),
            ("snow", 73),
            ("sleet", 68),
            ("freezing-rain", 66),
            ("hail", 96),
            ("thunderstorm", 95),
            ("thunder-rain", 95),
        ];
        for (icon, code) in expected {
            assert!(is_known_icon(icon), "`{icon}` is missing from the table");
            assert_eq!(icon_condition(icon), Condition::from_u8(code), "{icon}");
        }
        assert_eq!(icon_codes().len(), expected.len());
    }

    #[test]
    fn an_unlisted_icon_degrades_to_overcast() {
        for icon in ["", "rain-snow", "snow-showers-day", "not-an-icon"] {
            assert_eq!(
                icon_condition(icon),
                Condition::from_u8(UNKNOWN_ICON_CODE),
                "{icon}"
            );
        }
    }

    #[test]
    fn timestamps_accept_the_cap_offset_spellings() {
        for text in [
            "2026-10-06T12:00:00-04:00",
            "2026-10-06T12:00-04:00",
            "2026-10-06 12:00-04:00",
            "2026-10-06T12:00:00-0400",
        ] {
            assert!(parse_offset(text).is_some(), "{text}");
        }
        assert!(parse_offset("2026-10-06T12:00:00").is_none());
    }

    #[test]
    fn an_alert_maps_every_field_and_falls_back_to_a_stable_id() {
        let raw = AlertBlock {
            id: None,
            event: Some("Flood Watch".to_owned()),
            headline: Some("Flood Watch in effect".to_owned()),
            description: Some("Heavy rainfall.".to_owned()),
            onset: Some("2026-10-06T12:00:00-04:00".to_owned()),
            onset_epoch: None,
            ends: Some("2026-10-07T02:00:00-04:00".to_owned()),
            ends_epoch: None,
            expires: Some("2026-10-06T20:00:00-04:00".to_owned()),
            expires_epoch: None,
            severity: Some("Severe".to_owned()),
            urgency: Some("Expected".to_owned()),
            certainty: Some("Likely".to_owned()),
        };
        let alert = alert_of(&raw, Tz::America__New_York).expect("the alert maps");
        assert_eq!(alert.source, AlertSource::VisualCrossing);
        assert_eq!(alert.event, "Flood Watch");
        assert_eq!(alert.headline, "Flood Watch in effect");
        assert_eq!(alert.description.as_deref(), Some("Heavy rainfall."));
        assert_eq!(alert.severity, Severity::Severe);
        assert_eq!(alert.urgency, Urgency::Expected);
        assert_eq!(alert.certainty, Certainty::Likely);
        assert_eq!(
            alert.onset.map(|at| at.to_rfc3339()),
            Some("2026-10-06T12:00:00-04:00".to_owned())
        );
        assert_eq!(
            alert.ends.map(|at| at.to_rfc3339()),
            Some("2026-10-07T02:00:00-04:00".to_owned())
        );
        assert_eq!(
            alert.expires.map(|at| at.to_rfc3339()),
            Some("2026-10-06T20:00:00-04:00".to_owned())
        );
        // Without a source id the identity is the event and its onset, which is stable.
        assert_eq!(
            alert.id,
            "visualcrossing:Flood Watch@2026-10-06T12:00:00-04:00"
        );
    }

    #[test]
    fn an_alert_without_an_event_or_with_a_bad_timestamp_is_skipped() {
        let base = AlertBlock {
            id: Some("abc".to_owned()),
            event: Some("Flood Watch".to_owned()),
            headline: None,
            description: None,
            onset: Some("not a timestamp".to_owned()),
            onset_epoch: None,
            ends: None,
            ends_epoch: None,
            expires: None,
            expires_epoch: None,
            severity: None,
            urgency: None,
            certainty: None,
        };
        assert!(alert_of(&base, Tz::UTC).is_err());

        let no_event = AlertBlock {
            event: None,
            onset: None,
            ..base
        };
        assert!(alert_of(&no_event, Tz::UTC).is_err());
    }

    #[test]
    fn the_fallback_window_spans_the_requested_days_from_local_today() {
        let loc = Location {
            name: "Reston".to_owned(),
            admin1: None,
            country: "United States".to_owned(),
            country_code: Some("US".to_owned()),
            lat: 38.9697,
            lon: -77.385,
            tz: Tz::America__New_York,
            elevation_m: None,
            population: None,
            source: LocationSource::Coordinates,
            station: None,
        };
        let today = NaiveDate::from_ymd_opt(2026, 10, 6).expect("a valid date");
        let request = window_request(&loc, 3, today, "test-key");
        assert_eq!(
            request.url(),
            format!(
                "{}/{:.4},{:.4}/2026-10-06/2026-10-08",
                super::BASE,
                38.9697,
                -77.385
            )
        );
        // The key travels in the clear (it is a query parameter) but is redacted in every printed
        // spelling.
        assert!(request.full_url().contains("key=test-key"));
        assert!(!request.redacted_url().contains("test-key"));
        assert!(!format!("{request:?}").contains("test-key"));
    }
}
