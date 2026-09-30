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
//! breaking change: it bumps `schema_version`, and the release notes say so (the changelog file
//! itself arrives with step 13, `docs/schema.md` with it).
//!
//! The rules that make the document usable by a script:
//!
//! * every key is **always present**; a value the provider did not report is `null`, never an
//!   omitted key, so `jq -r '.current.uv_index'` cannot fail with a missing path;
//! * values are **canonical metric** (`temp_c`, `wind_kmh`, `precip_mm`, `pressure_hpa`,
//!   `visibility_km`) with the unit in the key name, so `--units`, `--color` and `--width` have no
//!   effect on this format — the same report renders to the same bytes in every mode;
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
use crate::error::{Error, Result};
use crate::geo::attribution_line;
use crate::model::{Attribution, Condition, Current, DayForecast, DayPart, Location, Report};
use crate::provider::licence_line;

/// The schema version this build emits; see the module documentation for what may change within
/// one version.
pub const SCHEMA_VERSION: u32 = 1;

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
    /// Where the data came from and what has to be credited.
    attribution: AttributionJson<'a>,
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
            attribution: AttributionJson::of(&report.attribution, &report.location),
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
    /// Which resolver produced the location: `geocoder`, `osm`, `coordinates`, `ip` or `config`.
    source: &'static str,
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
        LocationSource::Osm => "osm",
        LocationSource::Coordinates => "coordinates",
        LocationSource::Ip => "ip",
        LocationSource::Config => "config",
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
            temp_c: current.temp_c,
            feels_like_c: current.feels_like_c,
            humidity_pct: current.humidity_pct,
            precip_mm: current.precip_mm,
            pressure_hpa: current.pressure_hpa,
            visibility_km: current.visibility_km,
            wind_kmh: current.wind_kmh,
            wind_dir_deg: current.wind_dir_deg,
            wind_gust_kmh: current.wind_gust_kmh,
            cloud_cover_pct: current.cloud_cover_pct,
            uv_index: current.uv_index,
            is_day: current.is_day,
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
            min_c: day.temp_min_c,
            max_c: day.temp_max_c,
            parts: PartsJson::of(&day.parts, ctx),
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
    fn of(parts: &'a [DayPart; 4], ctx: &'a RenderContext<'_>) -> Self {
        Self {
            morning: PartJson::of(&parts[0], ctx),
            noon: PartJson::of(&parts[1], ctx),
            evening: PartJson::of(&parts[2], ctx),
            night: PartJson::of(&parts[3], ctx),
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
            temp_c: part.temp_c,
            feels_like_c: part.feels_like_c,
            precip_mm: part.precip_mm,
            precip_prob_pct: part.precip_prob_pct,
            humidity_pct: part.humidity_pct,
            visibility_km: part.visibility_km,
            wind_kmh: part.wind_kmh,
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

/// Where the data came from, and the credits the licences require.
#[derive(Debug, Serialize)]
struct AttributionJson<'a> {
    /// Registry id of the backend that answered, e.g. `open-meteo`.
    provider: &'a str,
    /// The endpoint the answer came from, without query parameters or any API key.
    url: &'a str,
    /// The data licence credit, `null` when the backend has no verified one.
    notice: Option<&'static str>,
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
            notice: licence_line(&attribution.provider),
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
            },
            current,
            days,
            attribution: Attribution {
                provider: "open-meteo".to_owned(),
                url: "https://api.open-meteo.com/v1/forecast?latitude=39.9042&longitude=116.4074"
                    .to_owned(),
                fetched_at: Utc
                    .with_ymd_and_hms(2026, 9, 30, 4, 15, 0)
                    .single()
                    .expect("an instant"),
                raw: None,
            },
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
        assert!(
            day["parts"]["noon"]["visibility_km"].is_null(),
            "the part's own visibility is null when upstream has none"
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
