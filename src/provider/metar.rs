// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! METAR observations from `aviationweather.gov`: the station-based, keyless aviation backend.
//!
//! This backend answers with an *observation*, not a forecast: [`Capabilities::current`] is true,
//! `hourly`/`daily` are false and `max_days` is `0`, so `--days` is ignored (with one warning) and
//! the renderers show the observation age instead of a day table. The registry row carries the same
//! facts, and no renderer branches on the `metar` id — they ask
//! [`crate::provider::capabilities_of`].
//!
//! # Upstream
//!
//! | resource | request | cache |
//! |---|---|---|
//! | observation | `metar?ids=<ICAO>&format=json` | `weather/metar-<ICAO>-current.json`, TTL `cache.weather_ttl_secs` |
//! | station metadata | `stationinfo?ids=<ICAO>&format=json` | `station/<ICAO>.json`, TTL 30 days |
//! | TAF (only under `-v`) | `taf?ids=<ICAO>&format=raw` | `weather/metar-<ICAO>-taf.json`, TTL `cache.weather_ttl_secs` |
//!
//! The observation request is the only one a normal run makes: its JSON carries `obsTime` and the
//! raw report (`rawOb`), which is what the decoder reads. `format=raw` is the same text in a bare
//! body — the recording recipe in the step file uses it to pin the fixture — so the provider does
//! not spend a second request on it. The TAF is fetched only when `--verbose` asked for it, and a
//! TAF that cannot be fetched is a note, never a failure: the observation is the answer.
//!
//! # Station resolution
//!
//! A METAR names a station, so the location is a station: `--station KJFK`,
//! `[providers.metar] station`, or `@lat,lon` (the nearest row of
//! [`station_table::STATIONS`]). The order is documented on [`resolve_station`].
//!
//! # Licence
//!
//! NOAA/NWS data is US government public domain, which asks for no credit; the registry row still
//! names the source (`aviationweather.gov (NOAA/NWS, public domain)`) and the renderers print it,
//! because a user reading an observation deserves to know where it came from.

pub mod decode;
pub mod station_table;

use std::time::Duration;

use chrono::{DateTime, Datelike as _, TimeZone as _, Timelike as _, Utc};
use chrono_tz::Tz;
use serde::Deserialize;

use self::decode::{Decoded, decode_metar};
use self::station_table::Station;
use super::{Capabilities, Env, FetchRequest, Provider, ProviderId, requested_days};
use crate::cache::{CacheKey, CacheMode};
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::{Attribution, Current, Location, LocationSource, Report};

/// The provider id, as the registry and every error message spell it.
const PROVIDER: &str = "metar";

/// The `aviationweather.gov` data API root.
pub const BASE: &str = "https://aviationweather.gov/api/data";

/// How long a station's metadata stays fresh: station coordinates and elevations do not move.
const STATION_TTL: Duration = Duration::from_hours(30 * 24);

/// The METAR backend. Stateless: one value serves every fetch.
#[derive(Debug, Clone, Copy, Default)]
pub struct Metar;

impl Provider for Metar {
    fn id(&self) -> ProviderId {
        ProviderId::Metar
    }

    fn capabilities(&self) -> Capabilities {
        ProviderId::Metar.metadata().capabilities()
    }

    fn fetch(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
        // An observation-only backend serves no forecast: the request is clamped to zero days and
        // the CLI has already warned about `--days` (this call keeps a hand-built request honest).
        let _days = requested_days(req.days, self.capabilities().max_days, PROVIDER, env.quiet);

        let icao = station_of(loc, env)?;
        let station = resolve_station(&icao, env)?;
        let observation = observation(&icao, env)?;
        let decoded = decode_metar(&observation.raw_ob)?;

        if env.verbose > 0 {
            eprintln!("metar: {} report for {icao}", observation.icao_id);
        }
        if env.verbose > 1 {
            for note in cross_check(&observation, &decoded) {
                eprintln!("metar: {note}");
            }
        }

        let tz = station.tz;
        let observed_at = observed_at(&observation, &decoded, tz, env)?;
        let current = current_of(&decoded, observed_at, tz);
        let raw = if env.verbose > 0 {
            let mut text = observation.raw_ob.clone();
            if let Some(taf) = taf(&icao, env) {
                text.push('\n');
                text.push_str(&taf);
            }
            Some(text)
        } else {
            None
        };

        Ok(Report {
            location: station,
            current: Some(current),
            days: Vec::new(),
            attribution: Attribution {
                provider: PROVIDER.to_owned(),
                url: observation_request(&icao).redacted_url(),
                fetched_at: env.cache.clock().now().into(),
                raw,
            },
        })
    }
}

/// The station identifier a fetch is for.
///
/// A location that carries one (the CLI's `--station` and the configured default station) uses it.
/// Anything else — `@lat,lon`, or a city name a user forced onto this backend with `-p metar` — is
/// mapped to the nearest row of the embedded table, with the chosen station and the distance
/// reported under `-v`: the observation is the airport's, and the user has to be able to see which
/// airport that is.
fn station_of(loc: &Location, env: &Env<'_>) -> Result<String> {
    if let Some(icao) = loc.station.as_deref() {
        return Ok(icao.trim().to_ascii_uppercase());
    }
    let (station, distance) =
        station_table::nearest_with_distance(loc.lat, loc.lon).ok_or_else(|| Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: "the embedded station table is empty; this build cannot resolve a coordinate"
                .to_owned(),
        })?;
    if env.verbose > 0 {
        eprintln!(
            "metar: nearest station to {:.2},{:.2} is {} ({distance:.0} km)",
            loc.lat, loc.lon, station.icao
        );
    }
    Ok(station.icao.to_owned())
}

/// The location a station identifier stands for.
///
/// Resolution order, cheapest first:
///
/// 1. the embedded [`station_table`] — no I/O at all;
/// 2. the cached `station/<ICAO>.json` entry (30-day TTL), which [`fetch_json`] serves without a
///    request when it is fresh;
/// 3. the live `stationinfo` endpoint.
///
/// An identifier none of the three knows is [`Error::LocationNotFound`] (exit 5), naming the
/// identifier and pointing at `cirrocast location search` — the station is a location the user has
/// to fix, not an upstream failure a chain could fall through.
fn resolve_station(icao: &str, env: &Env<'_>) -> Result<Location> {
    if let Some(station) = station_table::lookup(icao) {
        if env.verbose > 0 {
            eprintln!("metar: {icao} is in the embedded station table");
        }
        return Ok(location_of(station));
    }

    let rows = station_rows(icao, env)?;
    let Some(row) = rows.into_iter().next() else {
        return Err(unknown_station(icao));
    };
    Ok(location_of_info(&row, icao, env))
}

/// The station metadata rows upstream answers for an identifier.
///
/// `stationinfo` answers an identifier it does not know with `204 No Content` and an empty body —
/// not with an empty array — so this cannot go through [`super::fetch_json`]: an empty body is a
/// *valid* answer here (it means "no such station") and must become `Ok(Vec::new())`, so that the
/// caller raises the location error the taxonomy asks for (exit 5) instead of a decode failure
/// (exit 3). The cache policy is the same one `fetch_json` applies.
fn station_rows(icao: &str, env: &Env<'_>) -> Result<Vec<StationInfo>> {
    cached_json(
        env,
        &stationinfo_request(icao),
        &CacheKey::station(icao),
        STATION_TTL,
        "station metadata",
        &format!("station {icao}"),
        |body| {
            if body.trim().is_empty() {
                return Ok(Vec::new());
            }
            serde_json::from_str(body).map_err(|error| Error::Upstream {
                provider: PROVIDER.to_owned(),
                status: None,
                message: format!("the station metadata for {icao} does not parse as JSON: {error}"),
            })
        },
    )
}

/// One upstream body through the cache, with the caller deciding what it means.
///
/// The same policy [`super::fetch_json`] implements — a fresh entry is served without a request, a
/// cached body that no longer parses is fetched again, offline mode turns a miss into
/// [`Error::Network`] — with the decoding left to `parse`, because the aviation endpoints use an
/// empty body for "nothing to report" and only the caller knows whether that is an answer or an
/// error.
fn cached_json<T>(
    env: &Env<'_>,
    request: &HttpRequest,
    key: &CacheKey,
    ttl: Duration,
    what: &str,
    place: &str,
    parse: impl Fn(&str) -> Result<T>,
) -> Result<T> {
    if env.verbose > 1 {
        eprintln!(
            "provider: {PROVIDER} {what} (cache {})",
            env.cache.mode().name()
        );
    }
    if let Some(entry) = env.cache.read(key)? {
        match parse(&entry.body) {
            Ok(value) => return Ok(value),
            Err(_) if env.verbose > 1 => {
                eprintln!(
                    "{}: body no longer parses, fetching again",
                    key.path().display()
                );
            }
            Err(_) => {}
        }
    }
    if env.cache.mode() == CacheMode::Offline {
        return Err(Error::Network(format!(
            "offline mode: no cached {PROVIDER} {what} for {place} at {}; \
             rerun without `--offline` to fetch it",
            key.path().display()
        )));
    }

    let response = env.http.send(request)?;
    let body = response.body().to_owned();
    let value = parse(&body)?;
    env.cache.write(key, response.status(), &body, ttl)?;
    Ok(value)
}

/// The rejection for an identifier no station metadata describes.
fn unknown_station(icao: &str) -> Error {
    Error::LocationNotFound(format!(
        "unknown station `{icao}`; check the identifier or find a nearby one with `cirrocast location search <place>`"
    ))
}

/// Whether `value` is an ICAO station identifier: exactly four characters, the first a letter, the
/// rest letters or digits.
///
/// One rule shared by `--station`, `[providers.metar] station` and the backend, so the CLI cannot
/// accept an identifier the station lookup would then refuse (a three-letter IATA code or a
/// five-digit WMO number is a different vocabulary).
#[must_use]
pub fn is_icao_station(value: &str) -> bool {
    let mut characters = value.chars();
    let first = characters.next();
    let rest: Vec<char> = characters.collect();
    first.is_some_and(|first| first.is_ascii_alphabetic())
        && rest.len() == 3
        && rest.iter().all(char::is_ascii_alphanumeric)
}

/// The current observation for a station, served from the cache when it is fresh.
///
/// The body is the upstream JSON array; a station that exists but has no current report comes back
/// as an empty array, which is [`Error::Upstream`] (exit 3) naming the station and the URL — the
/// station is fine, upstream has nothing to show.
fn observation(icao: &str, env: &Env<'_>) -> Result<MetarReport> {
    let request = observation_request(icao);
    let reports = cached_json(
        env,
        &request,
        &CacheKey::station_resource(PROVIDER, icao, "current"),
        Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs)),
        "current observation",
        &format!("station {icao}"),
        |body| {
            if body.trim().is_empty() {
                return Ok(Vec::new());
            }
            serde_json::from_str(body).map_err(|error| Error::Upstream {
                provider: PROVIDER.to_owned(),
                status: None,
                message: format!(
                    "the observation for {icao} does not parse as JSON: {error} ({})",
                    request.redacted_url()
                ),
            })
        },
    )?;
    reports.into_iter().next().ok_or_else(|| Error::Upstream {
        provider: PROVIDER.to_owned(),
        status: Some(204),
        message: format!(
            "no current observation for {icao}; upstream answered an empty body for {}",
            request.redacted_url()
        ),
    })
}

/// The raw TAF for a station, fetched only when `--verbose` asked for it.
///
/// A missing TAF is never an error — many stations issue none, and `--offline` must not turn a
/// verbose run into a failure — so every failure path (no cache entry, transport error, a non-2xx
/// status) yields `None` and the caller prints the observation alone.
fn taf(icao: &str, env: &Env<'_>) -> Option<String> {
    let key = CacheKey::station_resource(PROVIDER, icao, "taf");
    let ttl = Duration::from_secs(u64::from(env.config.cache.weather_ttl_secs));
    if let Ok(Some(entry)) = env.cache.read(&key) {
        return Some(entry.body);
    }
    if env.cache.mode() == CacheMode::Offline {
        return None;
    }
    let request = taf_request(icao);
    let response = env.http.send(&request).ok()?;
    if response.status() != 200 {
        return None;
    }
    let body = response.body().trim_end().to_owned();
    if body.is_empty() {
        return None;
    }
    let _ = env.cache.write(&key, response.status(), &body, ttl);
    Some(body)
}

// ---------------------------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------------------------

/// The current-observation request for a station.
fn observation_request(icao: &str) -> HttpRequest {
    HttpRequest::get(format!("{BASE}/metar"))
        .query("ids", icao)
        .query("format", "json")
        .header("Accept", "application/json")
}

/// The station-metadata request for a station.
fn stationinfo_request(icao: &str) -> HttpRequest {
    HttpRequest::get(format!("{BASE}/stationinfo"))
        .query("ids", icao)
        .query("format", "json")
        .header("Accept", "application/json")
}

/// The raw-TAF request for a station.
fn taf_request(icao: &str) -> HttpRequest {
    HttpRequest::get(format!("{BASE}/taf"))
        .query("ids", icao)
        .query("format", "raw")
        .header("Accept", "text/plain")
}

// ---------------------------------------------------------------------------------------------
// Wire shapes
// ---------------------------------------------------------------------------------------------

/// One observation as the `metar?format=json` endpoint reports it.
///
/// Only `icaoId`, `obsTime` and `rawOb` are read for values; every other field exists for the
/// `-vv` cross-check, and they are [`serde_json::Value`] on purpose: upstream types drift (`wdir`
/// is a number for a real direction and the string `"VRB"` for a variable one, `visib` is a number
/// of statute miles or a string like `"10+"`), and a field that only cross-checks must never be
/// able to make a decodable report undecodable.
#[derive(Debug, Clone, Deserialize)]
pub struct MetarReport {
    /// The station the report belongs to.
    #[serde(rename = "icaoId")]
    pub icao_id: String,
    /// The observation instant, as a Unix epoch in seconds.
    #[serde(rename = "obsTime", default)]
    pub obs_time: Option<i64>,
    /// The raw METAR/SPECI report, verbatim.
    #[serde(rename = "rawOb")]
    pub raw_ob: String,
    /// Air temperature in °C.
    #[serde(default)]
    pub temp: Option<serde_json::Value>,
    /// Dew point in °C.
    #[serde(default)]
    pub dewp: Option<serde_json::Value>,
    /// Wind direction: degrees, or `"VRB"`.
    #[serde(default)]
    pub wdir: Option<serde_json::Value>,
    /// Wind speed, in knots.
    #[serde(default)]
    pub wspd: Option<serde_json::Value>,
    /// Gust speed, in knots.
    #[serde(default)]
    pub wgst: Option<serde_json::Value>,
    /// Visibility: statute miles, or a string like `"10+"`.
    #[serde(default)]
    pub visib: Option<serde_json::Value>,
    /// Altimeter setting in hPa.
    #[serde(default)]
    pub altim: Option<serde_json::Value>,
    /// The present-weather summary, when there is weather.
    #[serde(default, rename = "wxString")]
    pub wx_string: Option<serde_json::Value>,
}

/// One row of the `stationinfo?format=json` response.
#[derive(Debug, Clone, Deserialize)]
pub struct StationInfo {
    /// The station identifier.
    #[serde(rename = "icaoId")]
    pub icao_id: String,
    /// The place name, without the state and country suffixes of `name`.
    #[serde(default)]
    pub site: Option<String>,
    /// Latitude in degrees.
    pub lat: f64,
    /// Longitude in degrees.
    pub lon: f64,
    /// Elevation above sea level in metres; absent for a few stations.
    #[serde(default)]
    pub elev: Option<f64>,
    /// First level administrative division, e.g. `NY`.
    #[serde(default)]
    pub state: Option<String>,
    /// ISO 3166-1 alpha 2 country code, e.g. `US`.
    #[serde(default)]
    pub country: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Wire → canonical model
// ---------------------------------------------------------------------------------------------

/// The [`Location`] of a table row.
fn location_of(station: &Station) -> Location {
    Location {
        name: station.name.to_owned(),
        admin1: Some(station.state)
            .filter(|state| !state.is_empty())
            .map(str::to_owned),
        country: station.country.to_owned(),
        country_code: Some(station.country)
            .filter(|code| !code.is_empty())
            .map(str::to_owned),
        lat: station.lat,
        lon: station.lon,
        tz: station.tz.parse().unwrap_or(chrono_tz::UTC),
        elevation_m: station.elev_m,
        population: None,
        source: LocationSource::Station,
        station: Some(station.icao.to_owned()),
    }
}

/// The [`Location`] of an upstream station row.
fn location_of_info(row: &StationInfo, icao: &str, env: &Env<'_>) -> Location {
    let tz = crate::geo::tz::lookup(row.lat, row.lon).unwrap_or_else(|| {
        if env.verbose > 0 {
            eprintln!(
                "metar: no time zone for {icao} ({:.2},{:.2}); using UTC",
                row.lat, row.lon
            );
        }
        chrono_tz::UTC
    });
    Location {
        name: row
            .site
            .clone()
            .filter(|site| !site.trim().is_empty())
            .unwrap_or_else(|| icao.to_owned()),
        admin1: row.state.clone().filter(|state| !state.is_empty()),
        country: row.country.clone().unwrap_or_default(),
        country_code: row.country.clone().filter(|code| !code.is_empty()),
        lat: row.lat,
        lon: row.lon,
        tz,
        elevation_m: row.elev,
        population: None,
        source: LocationSource::Station,
        station: Some(icao.to_owned()),
    }
}

/// The provisional location the CLI hands to a station-based run.
///
/// The provider replaces it with the resolved station before anything is rendered, so this value
/// only has to carry the identifier and — when the table knows the station — usable coordinates
/// for a fallback backend in an explicit chain. An identifier the table does not know gets `0,0`:
/// it never reaches the output, because resolution either succeeds (and rewrites the location) or
/// fails with [`Error::LocationNotFound`].
#[must_use]
pub fn placeholder_location(icao: &str) -> Location {
    let icao = icao.trim().to_ascii_uppercase();
    match station_table::lookup(&icao) {
        Some(station) => location_of(station),
        None => Location {
            name: icao.clone(),
            admin1: None,
            country: String::new(),
            country_code: None,
            lat: 0.0,
            lon: 0.0,
            tz: chrono_tz::UTC,
            elevation_m: None,
            population: None,
            source: LocationSource::Station,
            station: Some(icao),
        },
    }
}

/// The observation instant, in the station's zone.
///
/// `obsTime` is authoritative. When it is missing (or not a usable epoch) the instant is rebuilt
/// from the report's own `ddHHMMZ` and the run's clock: METAR timestamps carry no month or year,
/// so the candidate nearest to *now* among this, the previous and the next month wins. That is the
/// documented fallback, and `-v` says when it is used.
fn observed_at(
    observation: &MetarReport,
    decoded: &Decoded,
    tz: Tz,
    env: &Env<'_>,
) -> Result<DateTime<chrono::FixedOffset>> {
    if let Some(seconds) = observation.obs_time
        && let Some(at) = DateTime::from_timestamp(seconds, 0)
    {
        return Ok(at.with_timezone(&tz).fixed_offset());
    }

    let now: DateTime<Utc> = env.cache.clock().now().into();
    let day = u32::from(decoded.day_of_month.clamp(1, 31));
    let today = now.date_naive();
    let first_of_month = today.with_day(1).unwrap_or(today);
    let months = [
        first_of_month - chrono::Months::new(1),
        first_of_month,
        first_of_month + chrono::Months::new(1),
    ];
    let mut best: Option<DateTime<Utc>> = None;
    for month_first in months {
        let Some(date) = month_first.with_day(day) else {
            continue;
        };
        let Some(naive) = date.and_hms_opt(u32::from(decoded.hour), u32::from(decoded.minute), 0)
        else {
            continue;
        };
        let candidate = Utc.from_utc_datetime(&naive);
        let closer = best.is_none_or(|current| {
            (candidate - now).num_seconds().abs() < (current - now).num_seconds().abs()
        });
        if closer {
            best = Some(candidate);
        }
    }

    let Some(at) = best else {
        return Err(Error::Upstream {
            provider: PROVIDER.to_owned(),
            status: None,
            message: format!(
                "cannot date the report `{}`: obsTime is missing and the timestamp does not name a real date",
                observation.raw_ob
            ),
        });
    };
    if env.verbose > 0 {
        eprintln!("metar: obsTime missing; dating the report from its own timestamp");
    }
    Ok(at.with_timezone(&tz).fixed_offset())
}

/// The canonical current conditions of a decoded report.
///
/// Fields a METAR does not carry stay empty rather than being invented: `feels_like_c` (no
/// apparent temperature), `uv_index` (no UV), `precip_mm` is `0.0` unless the remark group reports
/// otherwise, and the day/night flag is the local civil day (06:00–18:00), which step 17's astro
/// work can replace with real sun times.
fn current_of(decoded: &Decoded, observed_at: DateTime<chrono::FixedOffset>, tz: Tz) -> Current {
    let local_hour = observed_at.with_timezone(&tz).hour();
    Current {
        observed_at,
        temp_c: decoded.temp_c,
        feels_like_c: None,
        humidity_pct: relative_humidity(decoded.temp_c, decoded.dewpoint_c),
        precip_mm: decoded.precip_mm.unwrap_or(0.0),
        weather: decoded.condition,
        cloud_cover_pct: decoded.cloud_cover_pct,
        pressure_hpa: decoded.pressure_hpa,
        wind_kmh: decoded.wind_kmh,
        wind_dir_deg: decoded.wind_dir_deg.unwrap_or(0),
        wind_gust_kmh: decoded.wind_gust_kmh,
        visibility_km: decoded.visibility_km,
        uv_index: None,
        is_day: (6..18).contains(&local_hour),
    }
}

/// Relative humidity from the temperature and dew point, with the Magnus formula.
///
/// METAR carries no humidity, but the dew point *is* the humidity: the formula
/// `RH = 100 · exp(17.625·Td/(243.04+Td)) / exp(17.625·T/(243.04+T))` is the standard approximation
/// (Alduchov & Eskridge), accurate to a few tenths of a percent in the ranges weather happens in.
/// The result is clamped into `0..=100` and rounded, because the canonical field is a whole
/// percent; the value is *derived*, and the doc comment is what says so.
///
/// The cast is safe by construction — the clamp runs first, so the value is an integral `f32`
/// inside the target range — which is exactly what the truncation and sign-loss lints cannot see.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn relative_humidity(temp_c: f32, dewpoint_c: f32) -> u8 {
    let saturation = |temperature: f32| (17.625 * temperature / (243.04 + temperature)).exp();
    let ratio = saturation(dewpoint_c) / saturation(temp_c);
    (ratio * 100.0).round().clamp(0.0, 100.0) as u8
}

/// Differences between the decoded report and the upstream JSON fields, for `-vv`.
///
/// The raw report is authoritative; this exists so a drifting upstream (`visib` going numeric, a
/// station reporting a rounded temperature) is visible in a bug report rather than silently
/// ignored. The tolerances are the report's own resolution: temperatures are whole degrees, wind
/// speeds whole knots, visibility a tenth of the reported value.
fn cross_check(observation: &MetarReport, decoded: &Decoded) -> Vec<String> {
    let mut notes = Vec::new();
    let mut compare = |field: &str, upstream: Option<f64>, decoded: f64, tolerance: f64| {
        let Some(upstream) = upstream else {
            return;
        };
        if (upstream - decoded).abs() > tolerance {
            notes.push(format!(
                "cross-check: {field} upstream {upstream}, decoded {decoded}"
            ));
        }
    };
    compare(
        "temp",
        number(observation.temp.as_ref()),
        f64::from(decoded.temp_c),
        1.0,
    );
    compare(
        "dewp",
        number(observation.dewp.as_ref()),
        f64::from(decoded.dewpoint_c),
        1.0,
    );
    compare(
        "wind",
        number(observation.wspd.as_ref()),
        f64::from(decoded.wind_kmh) / 1.852,
        2.0,
    );
    compare(
        "visibility",
        number(observation.visib.as_ref()),
        f64::from(decoded.visibility_km.unwrap_or(0.0)) / 1.609_344,
        1.0,
    );
    notes
}

/// A JSON number, when the value is one.
fn number(value: Option<&serde_json::Value>) -> Option<f64> {
    value.and_then(serde_json::Value::as_f64)
}

#[cfg(test)]
mod tests {
    use super::{placeholder_location, relative_humidity};
    use crate::model::LocationSource;

    #[test]
    fn humidity_follows_the_dew_point() {
        // Saturation: the dew point is the temperature.
        assert_eq!(relative_humidity(20.0, 20.0), 100);
        // A 5 °C spread at 20 °C is about 73 % relative humidity.
        assert_eq!(relative_humidity(20.0, 15.0), 73);
        // Cold and dry.
        assert_eq!(relative_humidity(-3.0, -3.3), 98);
        // A dew point above the temperature cannot happen, but the field is a percentage.
        assert_eq!(relative_humidity(10.0, 30.0), 100);
    }

    #[test]
    fn the_placeholder_carries_the_identifier() {
        let known = placeholder_location("kjfk");
        assert_eq!(known.source, LocationSource::Station);
        assert_eq!(known.station.as_deref(), Some("KJFK"));
        assert_eq!(known.name, "New York/JF Kennedy Intl");
        assert!((known.lat - 40.639_16).abs() < 0.001);

        // An unknown station keeps the identifier; the provider replaces the rest or fails.
        let unknown = placeholder_location("ZZZZ");
        assert_eq!(unknown.station.as_deref(), Some("ZZZZ"));
        assert_eq!(unknown.name, "ZZZZ");
    }
}
