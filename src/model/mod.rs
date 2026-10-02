// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The canonical data model: the one shape every provider fills and every renderer reads.
//!
//! The rules that make the rest of the program small:
//!
//! * **Canonical metric/SI everywhere.** Values are stored as `temp_c`, `wind_kmh`, `precip_mm`,
//!   `pressure_hpa` and `visibility_km`; two `distance`-style fields (`lat`/`lon`, `elevation_m`)
//!   are not weather readings and keep their natural unit. Only [`units`] converts, at display
//!   time, which is why cache entries are unit independent.
//! * **Canonical conditions.** Everything is a [`Condition`] (a WMO 4677 code); providers own the
//!   mapping from their native codes and nothing downstream ever branches on those.
//! * **Day parts are four.** [`DayPartKind`] enumerates exactly `Morning | Noon | Evening | Night`
//!   and [`DayForecast::parts`] is an array, so a missing part cannot be represented; the provider
//!   aggregates hourly data into them using the location's time zone.
//! * **Serde on everything.** Every type round-trips through serde: providers build them,
//!   fixtures load them, and the `json` renderer projects them with its own key names. The cache
//!   itself stores the raw upstream body, never a serialised `Report`, so a provider schema
//!   change heals by refetching instead of failing on an old document.

pub mod condition;
pub mod units;

pub use condition::Condition;

use chrono::{
    DateTime, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, TimeDelta, TimeZone as _, Utc,
};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Where a [`Location`] came from.
///
/// This is provenance for debugging and for the "never silently guess twice" rule of step 04: a
/// location resolved from explicit coordinates may have its time zone corrected by the forecast
/// response (step 06), a geocoded one may not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LocationSource {
    /// Resolved by a provider's own geocoding service (Open-Meteo).
    Geocoder,
    /// Resolved through OpenStreetMap/Nominatim (`~query`).
    Osm,
    /// Given by the user as `@lat,lon`.
    Coordinates,
    /// Derived from the public IP address.
    Ip,
    /// Taken from the configured default location.
    Config,
    /// A METAR station identifier (`--station`, `[providers.metar] station`), resolved by the
    /// station table or the `stationinfo` endpoint.
    Station,
}

/// A place a forecast is for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Location {
    /// Display name, e.g. `Beijing`.
    pub name: String,
    /// First level administrative division, when the geocoder reports one.
    pub admin1: Option<String>,
    /// Country name, e.g. `China`.
    pub country: String,
    /// ISO 3166-1 alpha 2 country code, when known.
    pub country_code: Option<String>,
    /// Latitude in degrees, WGS 84.
    pub lat: f64,
    /// Longitude in degrees, WGS 84.
    pub lon: f64,
    /// IANA time zone the forecast times are expressed in.
    pub tz: Tz,
    /// Elevation above sea level in metres.
    pub elevation_m: Option<f64>,
    /// Population, used only to rank ambiguous geocoder matches; never rendered.
    pub population: Option<u64>,
    /// Which resolver produced this location.
    pub source: LocationSource,
    /// The METAR station identifier this location stands for, when it came from a station.
    ///
    /// A station has no other way to carry its identity: the display name is the site name and the
    /// coordinates are the airport's, so a backend that needs the identifier (`metar`, for its
    /// cache key and its request) reads it here. `None` for every non-station location.
    pub station: Option<String>,
}

/// Current conditions at the location.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Current {
    /// Observation time, in the location's local offset.
    pub observed_at: DateTime<FixedOffset>,
    /// Air temperature in °C.
    pub temp_c: f32,
    /// Apparent temperature in °C, when the provider reports one.
    ///
    /// Optional for the same reason the day parts' is: a backend that has no apparent temperature
    /// (SMHI publishes none) leaves it `None` and the renderers omit it rather than inventing a
    /// number by copying the air temperature.
    pub feels_like_c: Option<f32>,
    /// Relative humidity in percent.
    pub humidity_pct: u8,
    /// Precipitation in the last hour, in mm.
    pub precip_mm: f32,
    /// The condition now.
    pub weather: Condition,
    /// Total cloud cover in percent.
    pub cloud_cover_pct: u8,
    /// Sea level pressure in hPa.
    pub pressure_hpa: f32,
    /// Wind speed in km/h.
    pub wind_kmh: f32,
    /// Direction the wind blows *from*, in degrees clockwise from north.
    pub wind_dir_deg: u16,
    /// Gust speed in km/h.
    pub wind_gust_kmh: Option<f32>,
    /// Horizontal visibility in km.
    pub visibility_km: Option<f32>,
    /// UV index at the observation, when the provider reports one (0 = none, 11+ = extreme).
    ///
    /// Optional because not every backend offers it (the METAR rows of step 11 carry none); a
    /// renderer that has no value prints it as missing rather than as zero, which is also a real
    /// UV reading.
    pub uv_index: Option<f32>,
    /// Whether the location is in daylight right now.
    pub is_day: bool,
}

/// The four parts of a day, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DayPartKind {
    /// 06:00–12:00 local.
    Morning,
    /// 12:00–18:00 local.
    Noon,
    /// 18:00–24:00 local.
    Evening,
    /// 00:00–06:00 local.
    Night,
}

impl DayPartKind {
    /// The four parts in display order, for iterating a [`DayForecast::parts`] array.
    pub const ALL: [Self; 4] = [Self::Morning, Self::Noon, Self::Evening, Self::Night];

    /// Index into [`DayForecast::parts`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Morning => 0,
            Self::Noon => 1,
            Self::Evening => 2,
            Self::Night => 3,
        }
    }

    /// The location-local hours this part aggregates.
    #[must_use]
    pub const fn hours(self) -> std::ops::Range<u8> {
        match self {
            Self::Morning => 6..12,
            Self::Noon => 12..18,
            Self::Evening => 18..24,
            Self::Night => 0..6,
        }
    }

    /// The local hour used to sample hourly data for this part.
    #[must_use]
    pub const fn midpoint_hour(self) -> u8 {
        match self {
            Self::Morning => 9,
            Self::Noon => 15,
            Self::Evening => 21,
            Self::Night => 3,
        }
    }

    /// The English label, for the plain output, diagnostics and tests.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Morning => "Morning",
            Self::Noon => "Noon",
            Self::Evening => "Evening",
            Self::Night => "Night",
        }
    }
}

/// One aggregated part of a day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DayPart {
    /// Which part of the day this is.
    pub kind: DayPartKind,
    /// Representative temperature in °C.
    pub temp_c: f32,
    /// Apparent temperature in °C, when the provider offers it.
    pub feels_like_c: Option<f32>,
    /// Precipitation total for the part, in mm.
    pub precip_mm: f32,
    /// Precipitation probability in percent, when the provider offers it.
    pub precip_prob_pct: Option<u8>,
    /// The most significant condition in the part.
    pub weather: Condition,
    /// Wind speed in km/h.
    pub wind_kmh: f32,
    /// Direction the wind blows *from*, in degrees clockwise from north.
    pub wind_dir_deg: Option<u16>,
    /// Relative humidity in percent.
    pub humidity_pct: Option<u8>,
    /// Horizontal visibility in km.
    pub visibility_km: Option<f32>,
}

/// One forecast day, starting at the location-local midnight of `date`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DayForecast {
    /// The location-local calendar date.
    pub date: NaiveDate,
    /// The four parts, in [`DayPartKind::ALL`] order.
    ///
    /// Deserialisation rejects any other order: the array position and each part's `kind` must
    /// agree, so a renderer may index either way without contradicting the other.
    #[serde(deserialize_with = "parts_in_order")]
    pub parts: [DayPart; 4],
    /// Daily minimum temperature in °C.
    pub temp_min_c: f32,
    /// Daily maximum temperature in °C.
    pub temp_max_c: f32,
    /// Local sunrise, when the provider reports it.
    pub sunrise: Option<DateTime<FixedOffset>>,
    /// Local sunset, when the provider reports it.
    pub sunset: Option<DateTime<FixedOffset>>,
}

impl DayForecast {
    /// The part of the day `kind`, by [`DayPartKind::index`].
    ///
    /// Pressing `parts` into a by-kind lookup keeps the array position and the per-part `kind`
    /// field from disagreeing: this accessor is what renderers use when they want one part.
    #[must_use]
    pub fn part(&self, kind: DayPartKind) -> &DayPart {
        &self.parts[kind.index()]
    }
}

/// Deserialises the four day parts and rejects an array whose `kind` fields do not match their
/// positions.
fn parts_in_order<'de, D>(deserializer: D) -> std::result::Result<[DayPart; 4], D::Error>
where
    D: serde::Deserializer<'de>,
{
    let parts = <[DayPart; 4]>::deserialize(deserializer)?;
    if let Some((index, part)) = parts
        .iter()
        .enumerate()
        .find(|(index, part)| part.kind.index() != *index)
    {
        let expected = DayPartKind::ALL
            .iter()
            .map(|kind| kind.label())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(serde::de::Error::custom(format!(
            "day parts must be in {expected} order; element {index} carries `{}`",
            part.kind.label()
        )));
    }
    Ok(parts)
}

/// Which backend produced a [`Report`], and when.
///
/// The registry-derived fields (`display_name`, `licence`, `capabilities`) travel *with the report*
/// rather than being looked up from the provider id at render time: the renderers must shape their
/// output from what the answering backend declared, and `src/render` may not import
/// `src/provider`. A report built by hand (a fixture, a future non-registry source) leaves them
/// empty, and the renderers then draw the neutral shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attribution {
    /// Registry id of the provider, e.g. `open-meteo`.
    pub provider: String,
    /// Human-readable provider name (`Open-Meteo`), for the footers; empty for an unknown source.
    #[serde(default)]
    pub display_name: String,
    /// The credit line the data licence requires, when the registry knows one.
    pub licence: Option<String>,
    /// What the backend declared it offers; `None` for an unknown source.
    pub capabilities: Option<ReportCapabilities>,
    /// The request URL, without any API key.
    pub url: String,
    /// When the data was fetched (or read from cache).
    pub fetched_at: DateTime<Utc>,
    /// The provider's raw condition codes, only when `--verbose` asked for them; debug aid, never
    /// read by rendering.
    pub raw: Option<String>,
}

impl Attribution {
    /// Provenance for a source the registry does not know: the provider id, the URL and the fetch
    /// time, with the registry-derived fields empty.
    ///
    /// Hand-built reports (fixtures, future non-registry sources) use this, and the renderers then
    /// draw the neutral shape: no capability-driven layout, no display-name substitution and no
    /// credit line.
    #[must_use]
    pub fn unregistered(
        provider: impl Into<String>,
        url: impl Into<String>,
        fetched_at: DateTime<Utc>,
        raw: Option<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            display_name: String::new(),
            licence: None,
            capabilities: None,
            url: url.into(),
            fetched_at,
            raw,
        }
    }
}

/// The capability flags of the backend behind a report, as the renderers see them.
///
/// Mirrors the registry row's shape: the provider module converts its `Capabilities` value into
/// this one when it builds the [`Attribution`], and a test in that module keeps the two in step.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportCapabilities {
    /// Current conditions are available.
    pub current: bool,
    /// Hourly data is available.
    pub hourly: bool,
    /// Daily data is available; `false` means `days` is empty by nature, not by request.
    pub daily: bool,
    /// Weather alerts are available.
    pub alerts: bool,
    /// Longest forecast the backend serves, in days (`0` = observations only).
    pub max_days: u8,
    /// Whether an API key is required.
    pub requires_key: bool,
    /// Environment variable that supplies the key, when there is one.
    pub key_env: Option<String>,
    /// Which location forms the backend accepts.
    pub locations: ReportLocationKinds,
}

/// The location forms a backend accepts, mirrored for the renderers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportLocationKinds {
    /// Resolved place names.
    pub city: bool,
    /// METAR station identifiers.
    pub station: bool,
    /// Raw `lat,lon` coordinates.
    pub lat_lon: bool,
}

#[cfg(test)]
impl ReportCapabilities {
    /// An `Open-Meteo`-shaped capability set for unit tests in the renderers, which may not import
    /// the provider registry (step 12's layering gate). `src/provider` has a test that mirrors the
    /// real row onto [`crate::provider::Capabilities::report`], so the two cannot drift unnoticed.
    #[must_use]
    pub fn open_meteo_test() -> Self {
        Self {
            current: true,
            hourly: true,
            daily: true,
            alerts: false,
            max_days: 16,
            requires_key: false,
            key_env: None,
            locations: ReportLocationKinds {
                city: true,
                station: false,
                lat_lon: true,
            },
        }
    }
}

/// Everything a renderer needs for one location.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// Where the forecast is for.
    pub location: Location,
    /// Current conditions, when the provider offers them.
    pub current: Option<Current>,
    /// Forecast days, oldest first, starting at the location-local today.
    pub days: Vec<DayForecast>,
    /// Where the data came from.
    pub attribution: Attribution,
}

/// Turns a location-local wall clock time into an instant in `tz`.
///
/// Hourly provider data arrives as local wall clock timestamps, and daylight saving transitions
/// make those ambiguous twice a year. The policy the day-part aggregation relies on:
///
/// * an unambiguous local time is used as is;
/// * an ambiguous one (the hour repeated when clocks fall back) resolves to the **earliest**
///   offset, i.e. the first occurrence;
/// * a local time inside a spring-forward gap is retried one hour later, the usual reading of
///   "02:30 does not exist, it became 03:30"; if even that instant is not a real local time the
///   data is unusable and the call fails with [`Error::Upstream`] (the `provider` field carries
///   `tzdata`, since it is the time zone database that rejects the timestamp).
pub fn resolve_local(tz: Tz, naive: NaiveDateTime) -> Result<DateTime<Tz>> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(at) => Ok(at),
        LocalResult::Ambiguous(first, _second) => Ok(first),
        LocalResult::None => {
            let shifted = naive
                .checked_add_signed(TimeDelta::hours(1))
                .ok_or_else(|| nonexistent(naive, tz))?;
            match tz.from_local_datetime(&shifted) {
                LocalResult::Single(at) => Ok(at),
                LocalResult::Ambiguous(first, _second) => Ok(first),
                LocalResult::None => Err(nonexistent(naive, tz)),
            }
        }
    }
}

/// The error for a local wall clock that no offset makes real, including the one-hour-shifted
/// retry: the `provider` field carries `tzdata`, since it is the time zone database that rejects
/// the timestamp.
fn nonexistent(naive: NaiveDateTime, tz: Tz) -> Error {
    Error::Upstream {
        provider: "tzdata".to_owned(),
        status: None,
        message: format!("local time {naive} does not exist in {tz}"),
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDateTime;
    use chrono_tz::Tz;

    use super::resolve_local;

    #[test]
    fn an_extreme_timestamp_fails_instead_of_overflowing() {
        // `NaiveDateTime::MAX` is inside a zone gap for this zone, so the spring-forward retry
        // runs `naive + 1h`, which overflows and panicked before `checked_add_signed` was used.
        let error = resolve_local(Tz::America__Santiago, NaiveDateTime::MAX)
            .expect_err("an unrepresentable local time is an upstream error");
        assert!(error.to_string().contains("does not exist"), "{error}");
    }
}
