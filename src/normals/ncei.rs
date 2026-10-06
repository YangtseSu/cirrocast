// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! NOAA NCEI's Global Summary of the Month: the keyless source of the `--normals` comparison.
//!
//! GSOM publishes monthly station summaries, not "normal" products, so a normal is **computed
//! here**: the mean of one calendar month's summaries over the configured reference period (the
//! WMO 1991–2020 window by default) for the station nearest the location. Two requests, both
//! through the shared client and cache with a 30-day TTL:
//!
//! 1. **Which station answers?** The access-services *search* endpoint takes a bounding box and
//!    answers with one entry per station file, each carrying the station's own coordinates and a
//!    nested list of per-datatype coverage records. The bounding box is the **north-west corner
//!    first**, then the south-east one — the `SW,NE` order the parameter looks like it wants
//!    answers `HTTP 500` (measured 2026-10-06), and a regression test pins the order. The entries
//!    are **not** sorted by distance either, so every entry is measured and the nearest one inside
//!    the configured radius wins, preferring a station whose coverage names all four datatypes.
//! 2. **What does that station say?** The *data* endpoint answers with one row per `YYYY-MM` of
//!    the period, projected to `TAVG`, `TMAX`, `TMIN` and `PRCP` (the projection is a third of the
//!    bytes of the full row). The requested calendar month's rows are averaged.
//!
//! Three conditions make a normal impossible, and each is a first-class answer (`Ok(None)` with
//! one `-v` line naming the reason) rather than an error: no station inside the radius, a month
//! with fewer than twenty complete years, or a month the station's rows do not cover. Transport
//! and decoding failures stay [`Err`]s and are the caller's to degrade.
//!
//! A *complete* year is a row carrying all four values: the station's record has real holes (the
//! recorded Beijing station reports a 2020-10 precipitation total and no temperatures), and
//! averaging the temperatures over one denominator and the precipitation over another would make
//! the printed `years` count mean two things at once.

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};

use crate::cache::{CacheKey, CacheMode};
use crate::config::parse_period;
use crate::error::{Error, Result};
use crate::http::HttpRequest;
use crate::model::{Location, Normals};
use crate::provider::Env;

/// The source id, as the `-v` lines and error messages spell it.
const PROVIDER: &str = "noaa-ncei";

/// The station-search endpoint.
pub const SEARCH_BASE: &str = "https://www.ncei.noaa.gov/access/services/search/v1/data";

/// The monthly-summary endpoint.
pub const DATA_BASE: &str = "https://www.ncei.noaa.gov/access/services/data/v1";

/// The dataset both requests name.
pub const DATASET: &str = "global-summary-of-the-month";

/// How many station files the search may answer with; every one of them is measured.
pub const SEARCH_LIMIT: u8 = 5;

/// The four datatypes a normal needs, in the fixed order the projection asks for them.
pub const DATATYPES: [&str; 4] = ["TAVG", "TMAX", "TMIN", "PRCP"];

/// Fewest contributing years before a mean is called a normal.
pub const MIN_YEARS: u16 = 20;

/// Both answers change by the decade, so they get the long TTL the grid mapping uses.
pub const NORMALS_TTL_SECS: u64 = 30 * 24 * 60 * 60;

/// Kilometres one degree of latitude spans, the factor that converts the configured radius into
/// the search box. A degree is 110.57 km at the equator and 111.69 km at the poles, so the box is
/// the radius circle to within the fourth decimal of a degree; the distance gate below — not the
/// box — is what decides which stations answer.
const KM_PER_DEGREE: f64 = 111.0;

/// Fetches the normal for `month` (`1`–`12`) at `loc`, or the reason there is none.
pub fn normals(loc: &Location, month: u8, env: &Env<'_>) -> Result<Option<Normals>> {
    if !(1..=12).contains(&month) {
        return Err(Error::Usage(format!(
            "climate normals need a calendar month, got {month}"
        )));
    }
    let config = &env.config.normals;
    let Some((start_year, end_year)) = parse_period(&config.period) else {
        return Err(Error::Config(format!(
            "normals.period: `{}` is not a period of two four-digit years, e.g. `1991-2020`",
            config.period
        )));
    };
    let radius_km = f64::from(config.max_distance_km);
    let place = place(loc);

    let search: SearchResponse = match cached(
        env,
        &CacheKey::normals_search(loc.lat, loc.lon, config.max_distance_km),
        "station search",
        &place,
        search_request(loc, config.max_distance_km),
    )? {
        Some(search) => search,
        None => return Ok(None),
    };

    let station = match nearest_candidate(&search, loc.lat, loc.lon, radius_km) {
        Ok(station) => station,
        Err(missing) => {
            note(env, &missing.reason(loc, radius_km));
            return Ok(None);
        }
    };

    let rows: Vec<Row> = match cached(
        env,
        &CacheKey::normals_month(&station.id, &config.period, month),
        "monthly summaries",
        &place,
        values_request(&station.id, start_year, end_year),
    )? {
        Some(rows) => rows,
        None => return Ok(None),
    };

    let values = match summarise(&rows, month, (start_year, end_year))? {
        MonthData::Usable(values) => values,
        MonthData::NoRows => {
            note(
                env,
                &format!("{} has no rows in {}", station.id, config.period),
            );
            return Ok(None);
        }
        MonthData::NoMonth => {
            note(
                env,
                &format!(
                    "{} has no rows for month {month} in {}",
                    station.id, config.period
                ),
            );
            return Ok(None);
        }
        MonthData::Thin { years } => {
            note(
                env,
                &format!(
                    "only {years} usable years for month {month} at {} in {}; a normal needs {MIN_YEARS}",
                    station.id, config.period
                ),
            );
            return Ok(None);
        }
    };

    Ok(Some(Normals {
        station: station.id,
        station_name: station.name,
        distance_km: station.distance_km,
        period: config.period.clone(),
        month,
        temp_mean_c: mean_f32(values.tavg, values.years),
        temp_max_c: mean_f32(values.tmax, values.years),
        temp_min_c: mean_f32(values.tmin, values.years),
        precip_mm: mean_f32(values.prcp, values.years),
        years: values.years,
    }))
}

// ---------------------------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------------------------

/// The station search, in a fixed parameter order (the tests assert the URL verbatim).
fn search_request(loc: &Location, radius_km: u16) -> HttpRequest {
    HttpRequest::get(SEARCH_BASE)
        .query("dataset", DATASET)
        .query("bbox", bbox(loc.lat, loc.lon, radius_km))
        .query("limit", SEARCH_LIMIT.to_string())
}

/// The search box around `(lat, lon)`: **`maxLat,minLon,minLat,maxLon`** — the north-west corner
/// first. The longitude half-width is the latitude one divided by `cos(lat)`, so the box is as wide
/// in kilometres as it is tall; the result is clamped at a pole and at the antimeridian.
fn bbox(lat: f64, lon: f64, radius_km: u16) -> String {
    let half_lat = f64::from(radius_km) / KM_PER_DEGREE;
    let half_lon = (half_lat / lat.to_radians().cos()).min(180.0);
    let (north, south) = (lat + half_lat, lat - half_lat);
    let (west, east) = ((lon - half_lon).max(-180.0), (lon + half_lon).min(180.0));
    format!("{north:.4},{west:.4},{south:.4},{east:.4}")
}

/// The month's summaries for one station and the configured period, projected to the four values.
fn values_request(station: &str, start_year: u16, end_year: u16) -> HttpRequest {
    HttpRequest::get(DATA_BASE)
        .query("dataset", DATASET)
        .query("stations", station)
        .query("startDate", format!("{start_year}-01-01"))
        .query("endDate", format!("{end_year}-12-31"))
        .query("format", "json")
        .query("units", "metric")
        .query("dataTypes", DATATYPES.join(","))
}

/// Reads one answer through the shared cache.
///
/// An offline miss is not an error here: `--normals` is a best-effort surface, and a run that asked
/// to stay offline simply has no normal, so the cache's own message travels as a `-v` note. Every
/// other failure keeps its taxonomy for the caller.
fn cached<T: DeserializeOwned>(
    env: &Env<'_>,
    key: &CacheKey,
    what: &str,
    place: &str,
    request: HttpRequest,
) -> Result<Option<T>> {
    if env.verbose > 1 {
        eprintln!(
            "provider: {PROVIDER} {what} (cache {})",
            env.cache.mode().name()
        );
    }
    let result = env.cache.read_or_fetch_json(
        key,
        Duration::from_secs(NORMALS_TTL_SECS),
        PROVIDER,
        what,
        place,
        move || {
            let response = env.http.send(&request)?;
            Ok((response.status(), response.body().to_owned()))
        },
    );
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error)
            if env.cache.mode() == CacheMode::Offline && matches!(error, Error::Network(_)) =>
        {
            note(env, &error.to_string());
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// One `-v` line naming why there is no normal; `-q` silences it, like every other note.
fn note(env: &Env<'_>, message: &str) {
    if env.verbose > 0 && !env.quiet {
        eprintln!("normals: {message}");
    }
}

/// The place spelling every note and error uses, matching the provider helper.
fn place(loc: &Location) -> String {
    format!("{} ({:.2}, {:.2})", loc.name, loc.lat, loc.lon)
}

// ---------------------------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------------------------

/// The search answer, in the subset `cirrocast` consumes.
///
/// Unknown fields are ignored on purpose: the response embeds per-datatype coverage, date ranges
/// and file metadata, and a change to any of them never invalidates a cached body.
#[derive(Debug, Deserialize)]
pub struct SearchResponse {
    /// One entry per station file in the box.
    #[serde(default)]
    pub results: Vec<SearchResult>,
}

/// One station file the search found.
#[derive(Debug, Deserialize)]
pub struct SearchResult {
    /// The file's name, e.g. `CHM00054511.csv`.
    #[serde(default)]
    name: String,
    /// The station's own point, as the plain `[lon, lat]` array.
    #[serde(default)]
    centroid: Option<Point>,
    /// The same point as a `GeoJSON` object.
    #[serde(default)]
    location: Option<Point>,
    /// The station file's bounding points.
    #[serde(default, rename = "boundingPoints")]
    bounding_points: Vec<Point>,
    /// The stations inside the file, each with its per-datatype coverage records.
    #[serde(default)]
    stations: Vec<StationRecord>,
}

impl SearchResult {
    /// The station's point as `(lat, lon)`: `centroid` first, then the bounding points, then the
    /// `location` object — the order the service actually fills them in.
    fn point(&self) -> Option<(f64, f64)> {
        self.centroid
            .as_ref()
            .or_else(|| self.bounding_points.first())
            .or(self.location.as_ref())
            .map(Point::lat_lon)
    }

    /// The station record to query: the first with an identifier, preferring one whose coverage
    /// names all four datatypes.
    fn station(&self) -> Option<&StationRecord> {
        self.stations
            .iter()
            .filter(|station| !station.id.trim().is_empty())
            .min_by_key(|station| u8::from(!station.covers_all()))
    }
}

/// A point in either of the two spellings the service uses.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Point {
    /// `[lon, lat]`, the shape `centroid` uses.
    Plain([f64; 2]),
    /// `{"coordinates": [lon, lat], …}`, the shape `location` and `boundingPoints[]` use.
    GeoJson { coordinates: [f64; 2] },
}

impl Point {
    /// The point as `(lat, lon)`, the order the distance and the model use.
    fn lat_lon(&self) -> (f64, f64) {
        match self {
            Self::Plain([lon, lat])
            | Self::GeoJson {
                coordinates: [lon, lat],
            } => (*lat, *lon),
        }
    }
}

/// One station inside a station file.
#[derive(Debug, Deserialize)]
pub struct StationRecord {
    /// The station's identifier, e.g. `CHM00054511` — what the values request asks for.
    #[serde(default)]
    pub id: String,
    /// The station's human name, e.g. `BEIJING, CH`.
    #[serde(default)]
    pub name: Option<String>,
    /// One record per datatype the station file covers.
    #[serde(default, rename = "dataTypes")]
    data_types: Vec<Coverage>,
}

impl StationRecord {
    /// Whether the coverage records name every datatype a normal needs.
    fn covers_all(&self) -> bool {
        DATATYPES.iter().all(|wanted| {
            self.data_types
                .iter()
                .any(|coverage| coverage.id == *wanted)
        })
    }

    /// The name the rendered line prints: the station's own, or the file name without `.csv`.
    fn display_name(&self, result: &SearchResult) -> String {
        match &self.name {
            Some(name) if !name.trim().is_empty() => name.clone(),
            _ => result.name.trim_end_matches(".csv").to_owned(),
        }
    }
}

/// One per-datatype coverage record.
#[derive(Debug, Deserialize)]
struct Coverage {
    /// The datatype's id, e.g. `TAVG`.
    #[serde(default)]
    id: String,
}

/// One `YYYY-MM` summary row, projected to the four values a normal needs.
///
/// A value is a JSON string in the recorded payloads (`"2.7"`), may be a JSON number, and a missing
/// measurement is an empty string, `null` or an absent key — all three mean "not measured". A
/// value that is neither a number nor a numeric string is an upstream error: a schema change must
/// not silently thin a record.
#[derive(Debug, Deserialize)]
struct Row {
    /// The row's month, `YYYY-MM`.
    #[serde(rename = "DATE")]
    date: String,
    /// Mean temperature, °C.
    #[serde(rename = "TAVG", default, deserialize_with = "optional_number")]
    tavg: Option<f64>,
    /// Mean daily maximum, °C.
    #[serde(rename = "TMAX", default, deserialize_with = "optional_number")]
    tmax: Option<f64>,
    /// Mean daily minimum, °C.
    #[serde(rename = "TMIN", default, deserialize_with = "optional_number")]
    tmin: Option<f64>,
    /// Monthly precipitation total, mm.
    #[serde(rename = "PRCP", default, deserialize_with = "optional_number")]
    prcp: Option<f64>,
}

/// A GSOM value: a JSON number, a numeric string, `""`, `null` or an absent key.
fn optional_number<'de, D>(deserializer: D) -> std::result::Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error as _;
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(number)) => Ok(number.as_f64()),
        Some(serde_json::Value::String(text)) => {
            let text = text.trim();
            if text.is_empty() {
                return Ok(None);
            }
            text.parse::<f64>()
                .map(Some)
                .map_err(|_| D::Error::custom(format!("`{text}` is not a number")))
        }
        Some(other) => Err(D::Error::custom(format!("`{other}` is not a number"))),
    }
}

// ---------------------------------------------------------------------------------------------
// Station choice and averaging
// ---------------------------------------------------------------------------------------------

/// The station a query uses, with what the rendered line needs.
struct Candidate {
    /// The station's identifier.
    id: String,
    /// The station's name.
    name: String,
    /// Great-circle distance from the requested point, kilometres.
    distance_km: f64,
}

/// Why the search answer has no station to query; the reason travels on the `-v` stream.
enum Missing {
    /// The box holds no station at all.
    Empty,
    /// Entries came back but none carried a coordinate.
    NoCoordinates,
    /// Every measured entry sits beyond the configured radius.
    OutOfRange { nearest_km: f64 },
    /// Something is inside the radius but carries no station record to query.
    NoStation,
}

impl Missing {
    /// The one-line reason.
    fn reason(&self, loc: &Location, radius_km: f64) -> String {
        let place = place(loc);
        match self {
            Self::Empty => format!("no GSOM station within {radius_km:.0} km of {place}"),
            Self::NoCoordinates => {
                format!("the GSOM search for {place} carried no station coordinates")
            }
            Self::OutOfRange { nearest_km } => format!(
                "no GSOM station within {radius_km:.0} km of {place} (nearest {nearest_km:.1} km)"
            ),
            Self::NoStation => format!(
                "the GSOM station within {radius_km:.0} km of {place} carries no station record"
            ),
        }
    }
}

/// The nearest station inside `radius_km`, preferring one whose coverage names all four datatypes.
///
/// Coverage beats distance on purpose: a station that cannot answer for `TAVG` yields no normal at
/// all, so an incomplete record is only worth querying when nothing better is in range. Entries
/// are measured one by one because the search response does not sort them by distance.
fn nearest_candidate(
    search: &SearchResponse,
    lat: f64,
    lon: f64,
    radius_km: f64,
) -> std::result::Result<Candidate, Missing> {
    let measured: Vec<(f64, &SearchResult)> = search
        .results
        .iter()
        .filter_map(|result| {
            result.point().map(|(result_lat, result_lon)| {
                (haversine_km(lat, lon, result_lat, result_lon), result)
            })
        })
        .collect();
    if search.results.is_empty() {
        return Err(Missing::Empty);
    }
    if measured.is_empty() {
        return Err(Missing::NoCoordinates);
    }

    let mut best: Option<(u8, f64, Candidate)> = None;
    for (distance, result) in &measured {
        if *distance > radius_km {
            continue;
        }
        let Some(station) = result.station() else {
            continue;
        };
        let rank = (u8::from(!station.covers_all()), *distance);
        let better = match &best {
            None => true,
            Some((best_rank, best_distance, _)) => rank < (*best_rank, *best_distance),
        };
        if better {
            best = Some((
                rank.0,
                rank.1,
                Candidate {
                    id: station.id.clone(),
                    name: station.display_name(result),
                    distance_km: *distance,
                },
            ));
        }
    }
    if let Some((_, _, candidate)) = best {
        return Ok(candidate);
    }
    let nearest_km = measured
        .iter()
        .map(|(distance, _)| *distance)
        .fold(f64::INFINITY, f64::min);
    if nearest_km > radius_km {
        Err(Missing::OutOfRange { nearest_km })
    } else {
        Err(Missing::NoStation)
    }
}

/// What the requested month's rows say.
enum MonthData {
    /// At least [`MIN_YEARS`] complete rows; the sums the means are built from.
    Usable(Values),
    /// The station has no rows inside the period at all.
    NoRows,
    /// The station has rows but none for the requested month.
    NoMonth,
    /// The month's rows are too few to call a normal.
    Thin { years: u16 },
}

/// The un-averaged sums of one month's complete rows.
struct Values {
    /// Sum of the mean daily temperatures.
    tavg: f64,
    /// Sum of the mean daily maxima.
    tmax: f64,
    /// Sum of the mean daily minima.
    tmin: f64,
    /// Sum of the precipitation totals.
    prcp: f64,
    /// The number of rows that contributed, i.e. the divisor of every sum.
    years: u16,
}

/// Reduces the period's rows to the requested month's summary.
fn summarise(rows: &[Row], month: u8, (start_year, end_year): (u16, u16)) -> Result<MonthData> {
    let mut values = Values {
        tavg: 0.0,
        tmax: 0.0,
        tmin: 0.0,
        prcp: 0.0,
        years: 0,
    };
    let mut any_row = false;
    let mut any_month = false;
    for row in rows {
        any_row = true;
        let (year, row_month) = row_year_month(&row.date)?;
        if !(start_year..=end_year).contains(&year) || row_month != month {
            continue;
        }
        any_month = true;
        let (Some(tavg), Some(tmax), Some(tmin), Some(prcp)) =
            (row.tavg, row.tmax, row.tmin, row.prcp)
        else {
            continue;
        };
        values.tavg += tavg;
        values.tmax += tmax;
        values.tmin += tmin;
        values.prcp += prcp;
        values.years += 1;
    }
    if !any_row {
        return Ok(MonthData::NoRows);
    }
    if !any_month {
        return Ok(MonthData::NoMonth);
    }
    if values.years < MIN_YEARS {
        return Ok(MonthData::Thin {
            years: values.years,
        });
    }
    Ok(MonthData::Usable(values))
}

/// Splits a GSOM `DATE` (`YYYY-MM`) into its year and month.
///
/// A value that is not a `YYYY-MM` date is an upstream error rather than a row to skip: silently
/// dropping rows would quietly thin the mean.
fn row_year_month(date: &str) -> Result<(u16, u8)> {
    let malformed = || Error::Upstream {
        provider: PROVIDER.to_owned(),
        status: None,
        message: format!("a monthly summary row carries `{date}`, not a `YYYY-MM` date"),
    };
    if date.len() != 7 {
        return Err(malformed());
    }
    let year = date
        .get(..4)
        .and_then(|year| year.parse().ok())
        .ok_or_else(malformed)?;
    let month = date
        .get(5..7)
        .and_then(|month| month.parse().ok())
        .ok_or_else(malformed)?;
    if !(1..=12).contains(&month) {
        return Err(malformed());
    }
    Ok((year, month))
}

/// The mean of a sum over its contributing years, narrowed to the model's `f32`.
///
/// The sums are `f64` — the three temperatures are degree-scale readings and the precipitation a
/// monthly total, and thirty of either accumulate smaller error in `f64` — and the model's `f32` is
/// the precision a displayed normal is worth.
#[allow(clippy::cast_possible_truncation)]
fn mean_f32(sum: f64, years: u16) -> f32 {
    (sum / f64::from(years)) as f32
}

/// The great-circle distance between two points in kilometres (haversine).
fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let radius_km = 6371.0088;
    let delta_phi = (lat2 - lat1).to_radians();
    let delta_lambda = (lon2 - lon1).to_radians();
    let a = (delta_phi / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (delta_lambda / 2.0).sin().powi(2);
    2.0 * radius_km * a.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::{MIN_YEARS, MonthData, Row, bbox, row_year_month, summarise};

    /// The bbox order is the endpoint's trap: `maxLat,minLon,minLat,maxLon` — north-west first.
    /// Swapping the corners answers `HTTP 500` (measured 2026-10-06), so this test is the
    /// regression gate the step asks for.
    #[test]
    fn the_box_puts_the_north_west_corner_first() {
        assert_eq!(
            bbox(39.9042, 116.4074, 60),
            "40.4447,115.7028,39.3637,117.1120"
        );
        let parts: Vec<f64> = bbox(39.9042, 116.4074, 60)
            .split(',')
            .map(|part| part.parse().expect("every part is a number"))
            .collect();
        assert!(
            parts[0] > parts[2],
            "the first latitude is the northern one"
        );
        assert!(
            parts[1] < parts[3],
            "the second longitude is the western one"
        );
    }

    /// Near a pole `cos(lat)` would blow the longitude half-width up without a clamp; the box must
    /// stay a sane, finite box rather than an `inf` the URL cannot carry.
    #[test]
    fn a_polar_box_stays_finite_and_clamped() {
        let box_text = bbox(89.9, 0.0, 60);
        assert!(
            !box_text.contains("inf") && !box_text.contains("NaN"),
            "{box_text}"
        );
        let parts: Vec<f64> = box_text
            .split(',')
            .map(|part| part.parse().expect("every part is a number"))
            .collect();
        assert_eq!((parts[1], parts[3]), (-180.0, 180.0));
    }

    /// The four values arrive as strings, numbers, empty strings or nulls; a value that is none of
    /// those is an upstream error rather than a quietly dropped year.
    #[test]
    fn a_value_may_be_a_string_a_number_a_blank_or_null_but_never_furniture() {
        let row: Row = serde_json::from_str(
            r#"{"DATE":"1991-01","TAVG":"2.7","TMAX":3,"TMIN":"","PRCP":null}"#,
        )
        .expect("the row decodes");
        assert_eq!(row.tavg, Some(2.7));
        assert_eq!(row.tmax, Some(3.0));
        assert_eq!(row.tmin, None);
        assert_eq!(row.prcp, None);

        let error = serde_json::from_str::<Row>(r#"{"DATE":"1991-01","TAVG":"warm"}"#)
            .expect_err("a non-numeric value is an error");
        assert!(error.to_string().contains("is not a number"), "{error}");
    }

    /// A `DATE` is `YYYY-MM`; anything else names the row in an upstream error.
    #[test]
    fn a_malformed_date_is_an_upstream_error() {
        assert_eq!(row_year_month("1991-01").expect("a valid date"), (1991, 1));
        assert!(row_year_month("1991-13").is_err());
        assert!(row_year_month("1991").is_err());
        assert!(row_year_month("Jan 1991").is_err());
    }

    /// The month's mean is the mean of the rows that carry **all four** values: the recorded
    /// stations have years with a precipitation total and no temperatures, and those years must not
    /// make the printed count mean two things at once.
    #[test]
    fn only_complete_rows_contribute_and_twenty_of_them_make_a_normal() {
        let mut rows = fixture_rows(MIN_YEARS - 1);
        assert!(matches!(
            summarise(&rows, 10, (1991, 2020)).expect("the rows are well formed"),
            MonthData::Thin { years: 19 }
        ));

        rows.extend(fixture_rows(MIN_YEARS));
        let MonthData::Usable(values) = summarise(&rows, 10, (1991, 2020)).expect("well formed")
        else {
            panic!("twenty-one complete rows are a normal");
        };
        assert_eq!(values.years, 19 + MIN_YEARS);

        // One incomplete row joins them without contributing.
        rows.push(
            serde_json::from_str(r#"{"DATE":"2020-10","PRCP":"1.0"}"#).expect("the row decodes"),
        );
        let MonthData::Usable(values) = summarise(&rows, 10, (1991, 2020)).expect("well formed")
        else {
            panic!("an incomplete row does not break the normal");
        };
        assert_eq!(values.years, 19 + MIN_YEARS);
    }

    /// A month the station's rows do not cover, and a month outside the configured window, are
    /// both "no normal" — with different reasons for the `-v` line.
    #[test]
    fn a_missing_month_and_a_month_outside_the_window_are_distinguished() {
        let rows = fixture_rows(MIN_YEARS);
        assert!(matches!(
            summarise(&rows, 6, (1991, 2020)).expect("well formed"),
            MonthData::NoMonth
        ));
        // Every row sits before this window, so the month has no rows inside it.
        assert!(matches!(
            summarise(&rows, 10, (2021, 2030)).expect("well formed"),
            MonthData::NoMonth
        ));
        // A window that trims the rows thins the record instead.
        assert!(matches!(
            summarise(&rows, 10, (2000, 2020)).expect("well formed"),
            MonthData::Thin { years: 11 }
        ));
        let empty: Vec<Row> = Vec::new();
        assert!(matches!(
            summarise(&empty, 10, (1991, 2020)).expect("well formed"),
            MonthData::NoRows
        ));
    }

    /// `years` complete October rows, one per year from 1991, with values that accumulate
    /// exactly (`TAVG` 1.0, `TMAX` 2.0, `TMIN` 3.0, `PRCP` 4.0 each).
    fn fixture_rows(years: u16) -> Vec<Row> {
        (0..years)
            .map(|offset| {
                let year = 1991 + i32::from(offset);
                serde_json::from_str(&format!(
                    r#"{{"DATE":"{year}-10","TAVG":"1.0","TMAX":"2.0","TMIN":"3.0","PRCP":"4.0"}}"#
                ))
                .expect("the row decodes")
            })
            .collect()
    }
}
