// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `json` format: one stable document per report.
//!
//! # The stability promise
//!
//! The document carries `"schema_version": 1`, and within one schema version the changes are
//! **additive only**: new keys may appear, and an existing key keeps its name, its type and its
//! unit. A consumer must ignore keys it does not know — that is what makes adding one a
//! non-breaking change. Removing a key, renaming one, changing a unit or a nullability is a
//! breaking change: it bumps `schema_version`, and the release notes say so. Every field is
//! written out in `docs/schema.md`, whose key index the test suite checks against the rendered
//! document.
//!
//! The rules that make the document usable by a script:
//!
//! * every key is **always present**; a value the provider did not report is `null`, never an
//!   omitted key, so `jq -r '.current.uv_index'` cannot fail with a missing path;
//! * values are **canonical metric** (`temp_c`, `wind_kmh`, `precip_mm`, `pressure_hpa`,
//!   `visibility_km`) with the unit in the key name, so `--units`, `--color` and `--width` have no
//!   effect on this format — the same report renders to the same bytes in every mode;
//! * every number is **finite and has no negative zero**. A non-finite value is rejected where the
//!   report is built (the provider boundary), never folded into the `null` that means "the provider
//!   did not report it"; `-0.0` is written as `0.0`, like every display path in the crate;
//! * timestamps are ISO 8601 with the location's offset (`2026-09-30T12:15:00+08:00`), and
//!   `attribution.retrieved_at` is UTC (`…Z`);
//! * `days` ascends from the location-local today, oldest first.
//!
//! The structs below are the schema: `serde` writes the fields in declaration order, and every
//! `Option` is written as `null` (no `skip_serializing_if`), which is what keeps the two rules
//! above true by construction rather than by review.

use std::borrow::Cow;

use serde::Serialize;

use super::{RenderContext, Renderer};
use crate::air::aqi::AqiCategory;
use crate::error::{Error, Result};
use crate::geo::attribution_line;
use crate::model::ReportCapabilities as Capabilities;
use crate::model::air::{POLLEN_UNIT, POLLUTANT_UNIT};
use crate::model::astro::{Astro, Moon, Sun};
use crate::model::units::{normalise_zero, normalise_zero_f64};
use crate::model::{
    AirQuality, Attribution, Condition, Current, DayForecast, DayPart, DayPartKind, Location,
    Pollen, Report,
};

/// The schema version this build emits; see the module documentation for what may change within
/// one version.
pub const SCHEMA_VERSION: u32 = 2;

/// The JSON renderer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Json;

impl Renderer for Json {
    fn render(&self, report: &Report, ctx: &RenderContext<'_>) -> Result<String> {
        let document = Document::of(report, ctx);
        serde_json::to_string_pretty(&document)
            .map_err(|error| Error::Other(format!("cannot render the report as JSON: {error}")))
    }
}

/// The whole document.
#[derive(Debug, Serialize)]
struct Document<'a> {
    /// Schema version; consumers switch on this.
    schema_version: u32,
    /// Where the forecast is for.
    location: LocationJson<'a>,
    /// Current conditions, `null` when the provider has none.
    current: Option<CurrentJson<'a>>,
    /// Forecast days, oldest first.
    days: Vec<DayJson<'a>>,
    /// Air quality, `null` when the run did not ask for it or the best-effort fetch degraded.
    air: Option<AirJson>,
    /// Moon phase, sun times and the next phase instants, `null` unless the run asked (`--moon`).
    astro: Option<AstroJson<'a>>,
    /// Severe-weather warnings in force, strongest first.
    alerts: Vec<AlertJson<'a>>,
    /// What the backend offers, so a consumer can tell "no days because it is an observation"
    /// from "no days because the request asked for none". `null` when the provider is unknown.
    capabilities: Option<&'a Capabilities>,
    /// Where the data came from and what has to be credited.
    attribution: AttributionJson<'a>,
    /// The credits the alert sources require, one line each; empty when there are no alerts or
    /// none of the sources asks for a credit.
    alert_credits: &'a [String],
}

impl<'a> Document<'a> {
    /// Projects the report onto the schema.
    fn of(report: &'a Report, ctx: &'a RenderContext<'_>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            location: LocationJson::of(&report.location),
            current: report
                .current
                .as_ref()
                .map(|current| CurrentJson::of(current, ctx)),
            days: report
                .days
                .iter()
                .map(|day| DayJson::of(day, ctx))
                .collect(),
            air: report.air.as_ref().map(AirJson::of),
            astro: report.astro.as_ref().map(|astro| AstroJson::of(astro, ctx)),
            alerts: report.alerts.iter().map(AlertJson::of).collect(),
            capabilities: report.attribution.capabilities.as_ref(),
            attribution: AttributionJson::of(&report.attribution, &report.location),
            alert_credits: ctx.alert_credits,
        }
    }
}

/// The place the report is for.
#[derive(Debug, Serialize)]
struct LocationJson<'a> {
    /// Display name.
    name: &'a str,
    /// First level administrative division, when known.
    admin1: Option<&'a str>,
    /// Country name, when known.
    country: &'a str,
    /// ISO 3166-1 alpha 2 code, when known.
    country_code: Option<&'a str>,
    /// Latitude in degrees, WGS 84.
    lat: f64,
    /// Longitude in degrees, WGS 84.
    lon: f64,
    /// IANA time zone the forecast times are expressed in.
    timezone: String,
    /// Elevation above sea level in metres, when known.
    elevation_m: Option<f64>,
    /// Which resolver produced the location: `geocoder`, `osm`, `coordinates`, `ip`, `config` or
    /// `station`.
    source: &'static str,
    /// The METAR station identifier, `null` for every non-station location.
    station: Option<&'a str>,
}

impl<'a> LocationJson<'a> {
    /// Projects a location.
    fn of(location: &'a Location) -> Self {
        Self {
            name: &location.name,
            admin1: location.admin1.as_deref(),
            country: &location.country,
            country_code: location.country_code.as_deref(),
            lat: location.lat,
            lon: location.lon,
            timezone: location.tz.name().to_owned(),
            elevation_m: location.elevation_m,
            source: source_name(location.source),
            station: location.station.as_deref(),
        }
    }
}

/// How a [`crate::model::LocationSource`] is spelled in the document.
///
/// Spelled out here rather than derived, so the JSON spelling is part of this module's contract
/// and cannot drift with a `serde` rename on the model.
const fn source_name(source: crate::model::LocationSource) -> &'static str {
    use crate::model::LocationSource;
    match source {
        LocationSource::Geocoder => "geocoder",
        LocationSource::Offline => "offline",
        LocationSource::Osm => "osm",
        LocationSource::Coordinates => "coordinates",
        LocationSource::Ip => "ip",
        LocationSource::Config => "config",
        LocationSource::Station => "station",
    }
}

/// Current conditions.
#[derive(Debug, Serialize)]
struct CurrentJson<'a> {
    /// Observation time, ISO 8601 with the location's offset.
    time: String,
    /// The condition now.
    condition: ConditionJson<'a>,
    /// Air temperature in °C.
    temp_c: f32,
    /// Apparent temperature in °C; `null` when the provider does not report one.
    feels_like_c: Option<f32>,
    /// Relative humidity in percent (0–100).
    humidity_pct: u8,
    /// Precipitation in the last hour, in mm.
    precip_mm: f32,
    /// Sea level pressure in hPa.
    pressure_hpa: f32,
    /// Horizontal visibility in km.
    visibility_km: Option<f32>,
    /// Wind speed in km/h.
    wind_kmh: f32,
    /// Direction the wind blows *from*, in degrees clockwise from north.
    wind_dir_deg: u16,
    /// Gust speed in km/h.
    wind_gust_kmh: Option<f32>,
    /// Total cloud cover in percent (0–100).
    cloud_cover_pct: u8,
    /// UV index; `null` when the provider does not report one.
    uv_index: Option<f32>,
    /// Whether the location is in daylight now.
    is_day: bool,
}

impl<'a> CurrentJson<'a> {
    /// Projects the current conditions.
    fn of(current: &'a Current, ctx: &'a RenderContext<'_>) -> Self {
        Self {
            time: iso_local(current.observed_at),
            condition: ConditionJson::of(current.weather, ctx),
            temp_c: normalise_zero(current.temp_c),
            feels_like_c: current.feels_like_c.map(normalise_zero),
            humidity_pct: current.humidity_pct,
            precip_mm: normalise_zero(current.precip_mm),
            pressure_hpa: normalise_zero(current.pressure_hpa),
            visibility_km: current.visibility_km.map(normalise_zero),
            wind_kmh: normalise_zero(current.wind_kmh),
            wind_dir_deg: current.wind_dir_deg,
            wind_gust_kmh: current.wind_gust_kmh.map(normalise_zero),
            cloud_cover_pct: current.cloud_cover_pct,
            uv_index: current.uv_index.map(normalise_zero),
            is_day: current.is_day,
        }
    }
}

/// One air-quality reading.
///
/// Every key is always present and nullable like the rest of the document: `null` means the
/// source did not report the value (or, for the whole object, that the run fetched none), never
/// zero. The units are part of the document because they are not the display units of the
/// weather: `--units us` does not apply here.
#[derive(Debug, Serialize)]
struct AirJson {
    /// Observation time, ISO 8601 with the location's offset.
    time: String,
    /// The source id, e.g. `open-meteo`.
    source: &'static str,
    /// US AQI, exactly as the source reports it.
    aqi_us: Option<u16>,
    /// European AQI, exactly as the source reports it.
    aqi_european: Option<u16>,
    /// Each index's category, derived from the raw number.
    category: AirCategoryJson,
    /// Fine particulate matter.
    pm2_5: Option<f64>,
    /// Coarse particulate matter.
    pm10: Option<f64>,
    /// Ground-level ozone.
    o3: Option<f64>,
    /// Nitrogen dioxide.
    no2: Option<f64>,
    /// Sulphur dioxide.
    so2: Option<f64>,
    /// Carbon monoxide.
    co: Option<f64>,
    /// Pollen forecast; `null` outside the source's pollen domain.
    pollen: Option<PollenJson>,
    /// The units the numbers are in.
    units: AirUnitsJson,
}

impl AirJson {
    /// Projects a reading.
    fn of(air: &AirQuality) -> Self {
        Self {
            time: iso_local(air.time),
            source: air.source.as_str(),
            aqi_us: air.aqi_us,
            aqi_european: air.aqi_european,
            category: AirCategoryJson {
                us: air.aqi_us.map(|index| AqiCategory::from_us(index).as_str()),
                european: air
                    .aqi_european
                    .map(|index| AqiCategory::from_european(index).as_str()),
            },
            pm2_5: air.pm2_5.map(normalise_zero_f64),
            pm10: air.pm10.map(normalise_zero_f64),
            o3: air.o3.map(normalise_zero_f64),
            no2: air.no2.map(normalise_zero_f64),
            so2: air.so2.map(normalise_zero_f64),
            co: air.co.map(normalise_zero_f64),
            pollen: air.pollen.as_ref().map(PollenJson::of),
            units: AirUnitsJson {
                pollutants: POLLUTANT_UNIT,
                pollen: POLLEN_UNIT,
            },
        }
    }
}

/// The two indices' categories.
#[derive(Debug, Serialize)]
struct AirCategoryJson {
    /// The US scale's category, e.g. `good`.
    us: Option<&'static str>,
    /// The European scale's category, e.g. `fair`.
    european: Option<&'static str>,
}

/// The pollen forecast, in grains/m³.
#[derive(Debug, Serialize)]
struct PollenJson {
    /// Alder pollen.
    alder: f64,
    /// Birch pollen.
    birch: f64,
    /// Grass pollen.
    grass: f64,
    /// Mugwort pollen.
    mugwort: f64,
    /// Olive pollen.
    olive: f64,
    /// Ragweed pollen.
    ragweed: f64,
}

impl PollenJson {
    /// Projects a pollen forecast.
    fn of(pollen: &Pollen) -> Self {
        Self {
            alder: normalise_zero_f64(pollen.alder),
            birch: normalise_zero_f64(pollen.birch),
            grass: normalise_zero_f64(pollen.grass),
            mugwort: normalise_zero_f64(pollen.mugwort),
            olive: normalise_zero_f64(pollen.olive),
            ragweed: normalise_zero_f64(pollen.ragweed),
        }
    }
}

/// The units of the air object, fixed by the contract and never the display units.
#[derive(Debug, Serialize)]
struct AirUnitsJson {
    /// Pollutant unit (`μg/m³`).
    pollutants: &'static str,
    /// Pollen unit (`grains/m³`).
    pollen: &'static str,
}

/// The moon and sun block, computed locally.
///
/// Present only when the run asked for it (`--moon` or `--format moon`), like the air object;
/// every key inside it is always present and nullable where the value can be absent, and no
/// timestamp is ever a clamped `00:00` — an event that does not happen is `null`, and a polar
/// state is named in `sun.polar`.
#[derive(Debug, Serialize)]
struct AstroJson<'a> {
    /// When the block was computed, UTC.
    computed_at: String,
    /// The Moon.
    moon: MoonJson<'a>,
    /// The Sun.
    sun: SunJson,
}

impl<'a> AstroJson<'a> {
    /// Projects the block.
    fn of(astro: &'a Astro, ctx: &'a RenderContext<'_>) -> Self {
        Self {
            computed_at: astro
                .computed_at
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            moon: MoonJson::of(&astro.moon, ctx),
            sun: SunJson::of(&astro.sun),
        }
    }
}

/// The Moon: phase, illumination, age, the day's rise/set and the next phase instants.
#[derive(Debug, Serialize)]
struct MoonJson<'a> {
    /// The phase's name in the report's language.
    phase: Cow<'a, str>,
    /// The phase's stable slug, e.g. `waxing-crescent`.
    phase_key: &'static str,
    /// Illuminated fraction of the disc, 0..=1 (geocentric).
    illuminated_fraction: f64,
    /// Days since the preceding New Moon.
    age_days: f64,
    /// Moonrise on the location-local day, ISO 8601 at the location's offset; `null` when the
    /// event does not happen on the day.
    moonrise: Option<String>,
    /// Moonset on the location-local day; `null` when the event does not happen on the day.
    moonset: Option<String>,
    /// The next four phase instants after the run's clock.
    next: Vec<NextPhaseJson<'a>>,
}

impl<'a> MoonJson<'a> {
    /// Projects the Moon.
    fn of(moon: &'a Moon, ctx: &'a RenderContext<'_>) -> Self {
        Self {
            phase: ctx.i18n.moon_phase(moon.phase),
            phase_key: moon.phase.as_str(),
            illuminated_fraction: normalise_zero_f64(moon.illuminated_fraction),
            age_days: normalise_zero_f64(moon.age_days),
            moonrise: moon.moonrise.map(iso_local),
            moonset: moon.moonset.map(iso_local),
            next: moon
                .next
                .iter()
                .map(|(phase, at)| NextPhaseJson {
                    phase: ctx.i18n.moon_phase(*phase),
                    phase_key: phase.as_str(),
                    at: iso_local(*at),
                })
                .collect(),
        }
    }
}

/// One upcoming phase instant.
#[derive(Debug, Serialize)]
struct NextPhaseJson<'a> {
    /// The phase's name in the report's language.
    phase: Cow<'a, str>,
    /// The phase's stable slug, the same vocabulary as `moon.phase_key`.
    phase_key: &'static str,
    /// The instant, ISO 8601 at the location's offset.
    at: String,
}

/// The Sun: the day's rise/set, the daylight span and where the times came from.
#[derive(Debug, Serialize)]
struct SunJson {
    /// Sunrise on the location-local day; `null` when the Sun does not rise.
    sunrise: Option<String>,
    /// Sunset on the location-local day; `null` when the Sun does not set.
    sunset: Option<String>,
    /// Seconds of daylight; `86400` for a polar day and `0` for a polar night.
    daylight_secs: Option<u32>,
    /// `day` or `night` inside the polar circles; `null` otherwise.
    polar: Option<&'static str>,
    /// `provider` when the backend supplied the times, `local` when they were computed here.
    source: &'static str,
}

impl SunJson {
    /// Projects the Sun.
    fn of(sun: &Sun) -> Self {
        Self {
            sunrise: sun.sunrise.map(iso_local),
            sunset: sun.sunset.map(iso_local),
            daylight_secs: sun.daylight_secs,
            polar: sun.polar.map(super::super::model::astro::Polar::as_str),
            source: sun.source.as_str(),
        }
    }
}

/// One forecast day.
#[derive(Debug, Serialize)]
struct DayJson<'a> {
    /// Location-local calendar date (`YYYY-MM-DD`).
    date: String,
    /// Local sunrise (`HH:MM`), `null` when the sun does not rise.
    sunrise: Option<String>,
    /// Local sunset (`HH:MM`), `null` when the sun does not set.
    sunset: Option<String>,
    /// Daily minimum temperature in °C.
    min_c: f32,
    /// Daily maximum temperature in °C.
    max_c: f32,
    /// The four parts of the day, keyed by part.
    parts: PartsJson<'a>,
}

impl<'a> DayJson<'a> {
    /// Projects one day and its four parts.
    fn of(day: &'a DayForecast, ctx: &'a RenderContext<'_>) -> Self {
        Self {
            date: day.date.to_string(),
            sunrise: day.sunrise.map(clock_time),
            sunset: day.sunset.map(clock_time),
            min_c: normalise_zero(day.temp_min_c),
            max_c: normalise_zero(day.temp_max_c),
            parts: PartsJson::of(day, ctx),
        }
    }
}

/// The four parts of a day, always all four keys.
///
/// The canonical model represents the parts as an array of exactly four, so a part is never
/// missing; the keys stay nullable for consumers that also read documents written by other
/// producers against the same schema.
#[derive(Debug, Serialize)]
struct PartsJson<'a> {
    /// 06:00–12:00 local.
    morning: PartJson<'a>,
    /// 12:00–18:00 local.
    noon: PartJson<'a>,
    /// 18:00–24:00 local.
    evening: PartJson<'a>,
    /// 00:00–06:00 local.
    night: PartJson<'a>,
}

impl<'a> PartsJson<'a> {
    /// Projects the four parts in display order.
    ///
    /// Each part is looked up by its [`DayPartKind`] rather than by array position, so the day
    /// model owns which part is which and no renderer can disagree with it.
    fn of(day: &'a DayForecast, ctx: &'a RenderContext<'_>) -> Self {
        Self {
            morning: PartJson::of(day.part(DayPartKind::Morning), ctx),
            noon: PartJson::of(day.part(DayPartKind::Noon), ctx),
            evening: PartJson::of(day.part(DayPartKind::Evening), ctx),
            night: PartJson::of(day.part(DayPartKind::Night), ctx),
        }
    }
}

/// One part of a day.
#[derive(Debug, Serialize)]
struct PartJson<'a> {
    /// The most significant condition in the part.
    condition: ConditionJson<'a>,
    /// Representative temperature in °C.
    temp_c: f32,
    /// Apparent temperature in °C.
    feels_like_c: Option<f32>,
    /// Precipitation total for the part, in mm.
    precip_mm: f32,
    /// Precipitation probability in percent (0–100).
    precip_prob_pct: Option<u8>,
    /// Relative humidity in percent (0–100).
    humidity_pct: Option<u8>,
    /// Horizontal visibility in km.
    visibility_km: Option<f32>,
    /// Wind speed in km/h.
    wind_kmh: f32,
    /// Direction the wind blows *from*, in degrees clockwise from north.
    wind_dir_deg: Option<u16>,
}

impl<'a> PartJson<'a> {
    /// Projects one part.
    fn of(part: &'a DayPart, ctx: &'a RenderContext<'_>) -> Self {
        Self {
            condition: ConditionJson::of(part.weather, ctx),
            temp_c: normalise_zero(part.temp_c),
            feels_like_c: part.feels_like_c.map(normalise_zero),
            precip_mm: normalise_zero(part.precip_mm),
            precip_prob_pct: part.precip_prob_pct,
            humidity_pct: part.humidity_pct,
            visibility_km: part.visibility_km.map(normalise_zero),
            wind_kmh: normalise_zero(part.wind_kmh),
            wind_dir_deg: part.wind_dir_deg,
        }
    }
}

/// A condition: the canonical WMO code and its text.
#[derive(Debug, Serialize)]
struct ConditionJson<'a> {
    /// WMO 4677 code, `0..=99`.
    code: u8,
    /// The condition text, in the report's language.
    text: Cow<'a, str>,
}

impl<'a> ConditionJson<'a> {
    /// Projects a condition through the catalog, so the text follows `--lang`.
    fn of(condition: Condition, ctx: &'a RenderContext<'_>) -> Self {
        Self {
            code: condition.code(),
            text: ctx.i18n.condition(condition),
        }
    }
}

/// One severe-weather alert, projected onto the schema.
///
/// `expires` is the instant the alert stops being live — CAP `ends` when the source carries one,
/// else CAP `expires` — so a consumer's own filtering matches the renderers' liveness rule. Every
/// key is always present; the ones the source did not report are `null`.
#[derive(Debug, Serialize)]
struct AlertJson<'a> {
    /// The source's own identifier.
    id: &'a str,
    /// Which source reported it: `nws`, `meteoalarm`, `qweather`, `hko`, `wmoswic`, `fpas` or
    /// `visualcrossing`.
    source: &'static str,
    /// The event name, e.g. `Tornado Warning`.
    event: &'a str,
    /// CAP severity: `unknown`, `minor`, `moderate`, `severe` or `extreme`.
    severity: &'static str,
    /// CAP urgency: `unknown`, `past`, `future`, `expected` or `immediate`.
    urgency: &'static str,
    /// CAP certainty: `unknown`, `unobserved`, `possible`, `unlikely`, `likely` or `observed`.
    certainty: &'static str,
    /// When the event starts, at its own offset; `null` when unreported.
    onset: Option<String>,
    /// When the alert stops being live, at its own offset; `null` when unreported.
    expires: Option<String>,
    /// The affected areas, de-duplicated across `info` blocks.
    areas: &'a [String],
    /// A one-line summary.
    headline: &'a str,
    /// The full description; `null` when the source carries none.
    description: Option<&'a str>,
    /// What the reader is told to do; `null` when the source carries none.
    instruction: Option<&'a str>,
    /// The issuing agency; `null` when unreported.
    sender: Option<&'a str>,
}

impl<'a> AlertJson<'a> {
    /// Projects one alert.
    fn of(alert: &'a crate::model::Alert) -> Self {
        Self {
            id: &alert.id,
            source: alert.source.as_str(),
            event: &alert.event,
            severity: alert.severity.as_str(),
            urgency: alert.urgency.as_str(),
            certainty: alert.certainty.as_str(),
            onset: alert.onset.map(iso_local),
            expires: alert.effective_end().map(iso_local),
            areas: &alert.areas,
            headline: &alert.headline,
            description: alert.description.as_deref(),
            instruction: alert.instruction.as_deref(),
            sender: alert.sender.as_deref(),
        }
    }
}

/// Where the data came from, and the credits the licences require.
#[derive(Debug, Serialize)]
struct AttributionJson<'a> {
    /// Registry id of the backend that answered, e.g. `open-meteo`.
    provider: &'a str,
    /// The endpoint the answer came from, without query parameters or any API key.
    url: &'a str,
    /// The data licence credit, `null` when the backend has no verified one.
    notice: Option<&'a str>,
    /// The place-data credit, `null` when the location source asks for none.
    location_notice: Option<&'static str>,
    /// When the data was fetched (or read from the cache), in UTC.
    retrieved_at: String,
}

impl<'a> AttributionJson<'a> {
    /// Projects the attribution.
    ///
    /// The request URL keeps its path but drops the query string: the query carries coordinates
    /// and variables (and never a key, which the model strips at the source), while the endpoint
    /// is what identifies the service that has to be credited.
    fn of(attribution: &'a Attribution, location: &'a Location) -> Self {
        Self {
            provider: &attribution.provider,
            url: endpoint(&attribution.url),
            notice: attribution.licence.as_deref(),
            location_notice: attribution_line(location),
            retrieved_at: attribution
                .fetched_at
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        }
    }
}

/// The endpoint of a request URL: everything before the query string.
fn endpoint(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

/// An instant at the location's own offset, ISO 8601 with seconds.
fn iso_local(at: chrono::DateTime<chrono::FixedOffset>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// `HH:MM` at the instant's own offset.
fn clock_time(at: chrono::DateTime<chrono::FixedOffset>) -> String {
    at.format("%H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, TimeZone as _, Utc};
    use chrono_tz::Tz;
    use serde_json::Value;

    use super::{Json, SCHEMA_VERSION, endpoint};
    use crate::config::UnitOverrides;
    use crate::i18n::{I18n, LanguageId, LanguageRequest};
    use crate::model::units::UnitSystem;
    use crate::model::{
        Attribution, Condition, Current, DayForecast, DayPart, DayPartKind, Location,
        LocationSource, Report,
    };
    use crate::render::{ColorMode, RenderContext, Renderer, TermCaps};

    /// The English catalog, loaded the way the CLI loads an unconfigured run.
    fn english() -> I18n {
        I18n::load(&LanguageRequest::Auto, |_| None)
    }
    fn moment(hour: u32, minute: u32) -> chrono::DateTime<FixedOffset> {
        FixedOffset::east_opt(8 * 3600)
            .expect("a valid offset")
            .with_ymd_and_hms(2026, 9, 30, hour, minute, 0)
            .single()
            .expect("a valid local time")
    }

    fn part(kind: DayPartKind, temp_c: f32, code: u8, visibility_km: Option<f32>) -> DayPart {
        DayPart {
            kind,
            temp_c,
            feels_like_c: Some(temp_c),
            precip_mm: 0.0,
            precip_prob_pct: Some(10),
            weather: Condition::from_u8(code),
            wind_kmh: 8.0,
            wind_dir_deg: Some(180),
            humidity_pct: Some(60),
            visibility_km,
        }
    }

    /// The provenance a provider-built report carries: the `open-meteo` row, spelled out because
    /// the renderers may not import the provider registry (step 12's layering gate).
    fn attribution() -> Attribution {
        Attribution {
            provider: "open-meteo".to_owned(),
            display_name: "Open-Meteo".to_owned(),
            licence: Some("Open-Meteo.com (CC BY 4.0)".to_owned()),
            capabilities: Some(crate::model::ReportCapabilities::open_meteo_test()),
            url: "https://api.open-meteo.com/v1/forecast?latitude=39.9042&longitude=116.4074"
                .to_owned(),
            fetched_at: Utc
                .with_ymd_and_hms(2026, 9, 30, 4, 15, 0)
                .single()
                .expect("an instant"),
            raw: None,
        }
    }

    fn report(current: Option<Current>, days: Vec<DayForecast>) -> Report {
        Report {
            location: Location {
                name: "Beijing".to_owned(),
                admin1: Some("Beijing".to_owned()),
                country: "China".to_owned(),
                country_code: Some("CN".to_owned()),
                lat: 39.9042,
                lon: 116.4074,
                tz: Tz::Asia__Shanghai,
                elevation_m: Some(44.0),
                population: None,
                source: LocationSource::Geocoder,
                station: None,
            },
            current,
            days,
            alerts: Vec::new(),
            air: None,
            astro: None,
            attribution: attribution(),
        }
    }

    fn current() -> Current {
        Current {
            observed_at: moment(12, 15),
            temp_c: 21.5,
            feels_like_c: Some(22.0),
            humidity_pct: 52,
            precip_mm: 0.0,
            weather: Condition::from_u8(1),
            cloud_cover_pct: 25,
            pressure_hpa: 1015.0,
            wind_kmh: 10.0,
            wind_dir_deg: 30,
            wind_gust_kmh: None,
            visibility_km: Some(14.0),
            uv_index: Some(5.0),
            is_day: true,
        }
    }

    fn document(report: &Report) -> Value {
        let i18n = english();
        let ctx = RenderContext {
            units: UnitSystem::Metric
                .resolve(&UnitOverrides::default())
                .expect("the default overrides resolve"),
            color: ColorMode::Never,
            width: 80,
            term: TermCaps::default(),
            now: moment(12, 30),
            tz: Tz::Asia__Shanghai,
            lang: LanguageId::EN_US,
            i18n: &i18n,
            alert_credits: &[],
            aqi_index: crate::air::aqi::AqiIndex::Us,
        };
        let text = Json
            .render(report, &ctx)
            .expect("the report renders as JSON");
        serde_json::from_str(&text).expect("the output is valid JSON")
    }

    #[test]
    fn every_documented_key_is_present_even_when_its_value_is_missing() {
        let document = document(&report(None, Vec::new()));

        assert_eq!(document["schema_version"], SCHEMA_VERSION);
        assert!(document["current"].is_null());
        assert_eq!(document["days"].as_array().expect("an array").len(), 0);

        for key in [
            "name",
            "admin1",
            "country",
            "country_code",
            "lat",
            "lon",
            "timezone",
            "elevation_m",
            "source",
        ] {
            assert!(
                !document["location"][key].is_null() || key == "elevation_m",
                "location.{key} is missing"
            );
        }
        assert_eq!(document["location"]["timezone"], "Asia/Shanghai");
        assert_eq!(document["location"]["source"], "geocoder");

        for key in [
            "provider",
            "url",
            "notice",
            "location_notice",
            "retrieved_at",
        ] {
            assert!(
                document["attribution"].get(key).is_some(),
                "attribution.{key} is missing"
            );
        }
        assert_eq!(
            document["attribution"]["url"],
            "https://api.open-meteo.com/v1/forecast"
        );
        assert_eq!(
            document["attribution"]["notice"],
            "Open-Meteo.com (CC BY 4.0)"
        );
        assert_eq!(
            document["attribution"]["retrieved_at"],
            "2026-09-30T04:15:00Z"
        );
    }

    #[test]
    fn the_current_block_carries_the_canonical_units_and_drops_no_key() {
        let document = document(&report(Some(current()), Vec::new()));
        let current = &document["current"];

        assert_eq!(current["time"], "2026-09-30T12:15:00+08:00");
        assert_eq!(current["condition"]["code"], 1);
        assert_eq!(current["condition"]["text"], "Mainly clear");
        assert_eq!(current["temp_c"], 21.5);
        assert_eq!(current["feels_like_c"], 22.0);
        assert_eq!(current["humidity_pct"], 52);
        assert_eq!(current["precip_mm"], 0.0);
        assert_eq!(current["pressure_hpa"], 1015.0);
        assert_eq!(current["visibility_km"], 14.0);
        assert_eq!(current["wind_kmh"], 10.0);
        assert_eq!(current["wind_dir_deg"], 30);
        assert_eq!(current["cloud_cover_pct"], 25);
        assert_eq!(current["uv_index"], 5.0);
        assert_eq!(current["is_day"], true);
        assert!(
            current["wind_gust_kmh"].is_null(),
            "a missing gust is null, not absent"
        );
    }

    #[test]
    fn a_day_lists_its_four_parts_in_order_and_its_sun_times_as_clock_times() {
        let day = DayForecast {
            date: chrono::NaiveDate::from_ymd_opt(2026, 9, 30).expect("a date"),
            parts: [
                part(DayPartKind::Morning, 18.0, 1, Some(12.0)),
                part(DayPartKind::Noon, 24.0, 2, None),
                part(DayPartKind::Evening, 21.0, 3, Some(10.0)),
                part(DayPartKind::Night, 17.0, 45, Some(8.0)),
            ],
            temp_min_c: 15.0,
            temp_max_c: 25.0,
            sunrise: Some(moment(6, 5)),
            sunset: Some(moment(17, 58)),
        };
        let document = document(&report(Some(current()), vec![day]));
        let day = &document["days"][0];

        assert_eq!(day["date"], "2026-09-30");
        assert_eq!(day["sunrise"], "06:05");
        assert_eq!(day["sunset"], "17:58");
        assert_eq!(day["min_c"], 15.0);
        assert_eq!(day["max_c"], 25.0);
        assert_eq!(day["parts"]["morning"]["temp_c"], 18.0);
        assert_eq!(day["parts"]["noon"]["temp_c"], 24.0);
        assert_eq!(day["parts"]["evening"]["temp_c"], 21.0);
        assert_eq!(day["parts"]["night"]["condition"]["code"], 45);
        let noon = day["parts"]["noon"]
            .as_object()
            .expect("the noon part is an object");
        assert!(
            noon.contains_key("visibility_km") && noon["visibility_km"].is_null(),
            "the part's own visibility is present and null when upstream has none"
        );
    }

    #[test]
    fn the_endpoint_drops_the_query_string() {
        assert_eq!(
            endpoint("https://api.open-meteo.com/v1/forecast?a=b"),
            "https://api.open-meteo.com/v1/forecast"
        );
        assert_eq!(
            endpoint("https://api.open-meteo.com/v1/forecast"),
            "https://api.open-meteo.com/v1/forecast"
        );
    }
}
