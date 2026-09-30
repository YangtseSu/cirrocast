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
//! * **Serde on everything.** The same types are the cache payload and, later, the `json` output
//!   schema, so a field is added once.

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
}

/// Current conditions at the location.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Current {
    /// Observation time, in the location's local offset.
    pub observed_at: DateTime<FixedOffset>,
    /// Air temperature in °C.
    pub temp_c: f32,
    /// Apparent temperature in °C.
    pub feels_like_c: f32,
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

/// Which backend produced a [`Report`], and when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attribution {
    /// Registry id of the provider, e.g. `open-meteo`.
    pub provider: String,
    /// The request URL, without any API key.
    pub url: String,
    /// When the data was fetched (or read from cache).
    pub fetched_at: DateTime<Utc>,
    /// The provider's raw condition codes, only when `--verbose` asked for them; debug aid, never
    /// read by rendering.
    pub raw: Option<String>,
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
            let shifted = naive + TimeDelta::hours(1);
            match tz.from_local_datetime(&shifted) {
                LocalResult::Single(at) => Ok(at),
                LocalResult::Ambiguous(first, _second) => Ok(first),
                LocalResult::None => Err(Error::Upstream {
                    provider: "tzdata".to_owned(),
                    status: None,
                    message: format!("local time {naive} does not exist in {tz}"),
                }),
            }
        }
    }
}
