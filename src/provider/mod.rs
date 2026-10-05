// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Registry metadata for the weather backends `cirrocast` can talk to.
//!
//! This module is the single source of truth for *what* each backend offers: its command line id,
//! the environment variable holding its API key, how many forecast days it serves, which location
//! forms it understands and which data shapes it returns.
//!
//! The numbers below are **declared from provider documentation, not measured**. Each row carries
//! the [`ProviderMeta::verified`] date of its last check against the provider's live documentation;
//! `docs/providers.md` holds the evidence (endpoints, quotas, licence wording, traps) behind every
//! row, and is the file to read before implementing or re-verifying a backend. Step 10's
//! re-verification pass corrected several rows (SMHI's endpoint, WWO's horizon, `QWeather`'s
//! coverage and deprecation status) and stamped the rest; treat the values as dated claims, not as
//! timeless facts.
//!
//! The second half of the module is the behaviour contract: [`Provider`] is what one backend
//! implements, [`select`] turns a `--provider` value into an ordered chain, and [`fetch_chain`]
//! walks that chain with the documented fallback rule. Every backend module above implements the
//! trait; the registry row's `implemented` flag is what [`select`] filters the chain by.

pub mod dayparts;
pub mod met_no;
pub mod metar;
pub mod open_meteo;
pub mod open_meteo_archive;
pub mod open_meteo_marine;
pub mod openweathermap;
pub mod pirateweather;
pub mod qweather;
pub mod smhi;
pub mod visualcrossing;
pub mod weatherapi;
pub mod worldweatheronline;

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use serde::de::DeserializeOwned;

use crate::cache::{Cache, CacheKey};
use crate::config::Config;
use crate::config::keys::KeyStore;
use crate::error::{Error, Result};
use crate::http::{HttpClient, HttpRequest};
use crate::model::{Attribution, Location, Report, ReportCapabilities, ReportLocationKinds};

/// A backend `cirrocast` knows how to talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProviderId {
    /// Open-Meteo, keyless global model data (the default backend).
    OpenMeteo,
    /// MET Norway's `Locationforecast` (keyless, global, step 23).
    MetNo,
    /// Open-Meteo's historical archive (keyless, 1940 onward, step 23).
    OpenMeteoArchive,
    /// Open-Meteo's marine API: waves, swell and sea-surface temperature (step 23).
    OpenMeteoMarine,
    /// Visual Crossing's timeline API (BYOK, supplies its own alerts, step 23).
    VisualCrossing,
    /// `OpenWeatherMap`.
    OpenWeatherMap,
    /// `WeatherAPI.com`.
    WeatherApi,
    /// World Weather Online.
    WorldWeatherOnline,
    /// Pirate Weather (a Dark Sky shaped API).
    PirateWeather,
    /// `QWeather` (China focused).
    QWeather,
    /// `SMHI` open data (Sweden and neighbours).
    Smhi,
    /// `METAR` observations from aviationweather.gov.
    Metar,
}

impl ProviderId {
    /// Every provider, in registry order (this is the order `provider list` prints).
    pub const fn all() -> [Self; 12] {
        [
            Self::OpenMeteo,
            Self::MetNo,
            Self::OpenMeteoArchive,
            Self::OpenMeteoMarine,
            Self::VisualCrossing,
            Self::OpenWeatherMap,
            Self::WeatherApi,
            Self::WorldWeatherOnline,
            Self::PirateWeather,
            Self::QWeather,
            Self::Smhi,
            Self::Metar,
        ]
    }

    /// The canonical command line spelling.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenMeteo => "open-meteo",
            Self::MetNo => "met-no",
            Self::OpenMeteoArchive => "open-meteo-archive",
            Self::OpenMeteoMarine => "open-meteo-marine",
            Self::VisualCrossing => "visualcrossing",
            Self::OpenWeatherMap => "openweathermap",
            Self::WeatherApi => "weatherapi",
            Self::WorldWeatherOnline => "worldweatheronline",
            Self::PirateWeather => "pirateweather",
            Self::QWeather => "qweather",
            Self::Smhi => "smhi",
            Self::Metar => "metar",
        }
    }

    /// The canonical ids of every provider, comma separated, for error messages and help text.
    pub fn known_ids() -> String {
        Self::all()
            .iter()
            .map(Self::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Capability metadata for this provider.
    ///
    /// This function *is* the registry table — one row per provider, kept together so that adding
    /// a backend means editing exactly one place — so it exceeds clippy's default length on
    /// purpose.
    #[allow(clippy::too_many_lines)]
    pub fn metadata(&self) -> ProviderMeta {
        match self {
            Self::OpenMeteo => ProviderMeta {
                id: *self,
                display_name: "Open-Meteo",
                requires_key: false,
                key_env: None,
                docs_url: "https://open-meteo.com/en/docs",
                max_days: 16,
                history_days: 92,
                marine: false,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "keyless, global coverage; free tier 10 000 calls/day",
                auth: "none (keyless; a commercial key exists for the `customer-` host)",
                coverage: "global",
                granularity: "hourly; daily aggregates",
                limits: "free tier 10 000 calls/day, 5 000/hour, 600/minute; non-commercial",
                verified: "2026-09-30",
                implemented: true,
                alerts: false,
                licence: Some("Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/"),
            },
            Self::MetNo => ProviderMeta {
                id: *self,
                display_name: "MET Norway",
                requires_key: false,
                key_env: None,
                docs_url: "https://api.met.no/weatherapi/locationforecast/2.0/documentation",
                max_days: 9,
                history_days: 0,
                marine: false,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "keyless, global; hourly for the first ~52 h then 6-hourly; a descriptive User-Agent with contact information is mandatory (`403` otherwise)",
                auth: "none (keyless); a descriptive User-Agent carrying contact information is required",
                coverage: "global",
                granularity: "hourly for the first ~52 h, 6-hourly beyond; 9-day horizon",
                limits: "20 requests/second per application; re-request only after `Expires`; coordinates at most 4 decimals; no `Yr` in the product name",
                verified: "2026-10-06",
                implemented: true,
                alerts: false,
                licence: Some("Data from MET Norway (CC BY 4.0) — https://www.met.no/"),
            },
            Self::OpenMeteoArchive => ProviderMeta {
                id: *self,
                display_name: "Open-Meteo Archive",
                requires_key: false,
                key_env: None,
                docs_url: "https://open-meteo.com/en/docs/historical-weather-api",
                max_days: 0,
                history_days: 30000,
                marine: false,
                current: false,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "keyless reanalysis (ERA5/ERA5-Land/IFS) from 1940-01-01; ~5-day latency, so a recent date is served by the forecast API instead; archive only — `--days` is a usage error",
                auth: "none (keyless)",
                coverage: "global",
                granularity: "hourly and daily reanalysis",
                limits: "the Open-Meteo free tier (< 10 000 calls/day, non-commercial)",
                verified: "2026-10-06",
                implemented: true,
                alerts: false,
                licence: Some(
                    "Open-Meteo.com (CC BY 4.0, ERA5/Copernicus) — https://open-meteo.com/",
                ),
            },
            Self::OpenMeteoMarine => ProviderMeta {
                id: *self,
                display_name: "Open-Meteo Marine",
                requires_key: false,
                key_env: None,
                docs_url: "https://open-meteo.com/en/docs/marine-weather-api",
                max_days: 8,
                history_days: 0,
                marine: true,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "keyless; waves, swell and sea-surface temperature for the nearest sea cell; supplementary — requested with `--marine`, never a forecast chain entry",
                auth: "none (keyless)",
                coverage: "coastal waters of the global wave models",
                granularity: "hourly; 8 forecast days",
                limits: "the Open-Meteo free tier (< 10 000 calls/day, non-commercial)",
                verified: "2026-10-06",
                implemented: true,
                alerts: false,
                licence: Some(crate::model::MarineSource::OpenMeteoMarine.licence()),
            },
            Self::VisualCrossing => ProviderMeta {
                id: *self,
                display_name: "Visual Crossing",
                requires_key: true,
                key_env: Some("CIRROCAST_VISUALCROSSING_KEY"),
                docs_url: "https://www.visualcrossing.com/resources/documentation/weather-api/timeline-weather-api/",
                max_days: 15,
                history_days: 0,
                marine: false,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "hourly timeline up to 15 days; the payload carries its own `alerts[]`, which the alert layer shows as source `visualcrossing`",
                auth: "API key in the `key` query parameter",
                coverage: "global",
                granularity: "hourly; 15-day horizon",
                limits: "free plan 1 000 records/day (`queryCost` ≈ 1 + 24×hours + days; the default 3-day query ≈ 76 records); keyed to the account's terms, no redistribution of cached data",
                verified: "2026-10-06",
                implemented: true,
                alerts: true,
                licence: Some("Visual Crossing Weather — https://www.visualcrossing.com/"),
            },
            Self::OpenWeatherMap => ProviderMeta {
                id: *self,
                display_name: "OpenWeatherMap",
                requires_key: true,
                key_env: Some("CIRROCAST_OPENWEATHERMAP_KEY"),
                docs_url: "https://openweathermap.org/forecast5",
                max_days: 5,
                history_days: 0,
                marine: false,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "free tier 60 calls/min, 1 000 000 calls/month; 2 calls per fetch, 3-hour steps",
                auth: "API key in the `appid` query parameter",
                coverage: "global",
                granularity: "3-hourly (40 slots ≈ 5 days)",
                limits: "free tier 60 calls/min, 1 000 000 calls/month; 2 calls per fetch",
                verified: "2026-09-30",
                implemented: true,
                alerts: false,
                licence: Some("OpenWeather (ODbL 1.0) — https://openweathermap.org/"),
            },
            Self::WeatherApi => ProviderMeta {
                id: *self,
                display_name: "WeatherAPI",
                requires_key: true,
                key_env: Some("CIRROCAST_WEATHERAPI_KEY"),
                docs_url: "https://www.weatherapi.com/docs/",
                max_days: 3,
                history_days: 0,
                marine: false,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "free tier: 100k calls/month, 3-day forecast (paid plans 14 days)",
                auth: "API key in the `key` query parameter",
                coverage: "global",
                granularity: "hourly (24 entries per day)",
                limits: "free tier 100 000 calls/month, 3-day forecast",
                verified: "2026-09-30",
                implemented: true,
                alerts: false,
                licence: Some(
                    "WeatherAPI.com (free-tier attribution) — https://www.weatherapi.com/",
                ),
            },
            Self::WorldWeatherOnline => ProviderMeta {
                id: *self,
                display_name: "World Weather Online",
                requires_key: true,
                key_env: Some("CIRROCAST_WORLDWEATHERONLINE_KEY"),
                docs_url: "https://www.worldweatheronline.com/weather-api/api/docs/local-city-town-weather-api.aspx",
                max_days: 5,
                history_days: 0,
                marine: false,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "free tier 100 requests/day, 5 forecast days per FAQ; format=json is mandatory",
                auth: "API key in the `key` query parameter",
                coverage: "global",
                granularity: "3-hourly (`tp=3`)",
                limits: "free tier 100 requests/day (the docs also print 500/month); 5 forecast days per FAQ",
                verified: "2026-09-30",
                implemented: true,
                alerts: false,
                licence: Some(
                    "WorldWeatherOnline.com (free-tier attribution) — https://www.worldweatheronline.com/",
                ),
            },
            Self::PirateWeather => ProviderMeta {
                id: *self,
                display_name: "Pirate Weather",
                requires_key: true,
                key_env: Some("CIRROCAST_PIRATEWEATHER_KEY"),
                docs_url: "https://docs.pirateweather.net/",
                max_days: 7,
                history_days: 0,
                marine: false,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "Dark Sky shaped responses; free tier 10 000 calls/month",
                auth: "API key as a path segment (or the `apikey` header)",
                coverage: "global",
                granularity: "hourly (168 with `extend=hourly`); 7 daily",
                limits: "free tier 10 000 calls/month, 1–4 requests/second",
                verified: "2026-09-30",
                implemented: true,
                alerts: false,
                licence: Some("Pirate Weather — https://pirateweather.net/"),
            },
            Self::QWeather => ProviderMeta {
                id: *self,
                display_name: "QWeather",
                requires_key: true,
                key_env: Some("CIRROCAST_QWEATHER_KEY"),
                docs_url: "https://dev.qweather.com/en/docs/api/weather/",
                max_days: 10,
                history_days: 0,
                marine: false,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "global coverage; v1 endpoints, metric-only measures; API host from https://console.qweather.com/setting",
                auth: "API key in the `X-QW-Api-Key` header (or `key=` query)",
                coverage: "global",
                granularity: "hourly (up to 240 h) and daily (up to 10 days)",
                limits: "first 50 000 requests/month at ¥0; QPM 3 000",
                verified: "2026-10-01",
                implemented: true,
                alerts: false,
                licence: Some("QWeather — https://www.qweather.com/"),
            },
            Self::Smhi => ProviderMeta {
                id: *self,
                display_name: "SMHI",
                requires_key: false,
                key_env: None,
                docs_url: "https://opendata.smhi.se/metfcst/snow1gv1",
                max_days: 10,
                history_days: 0,
                marine: false,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "keyless, Nordics and adjacent seas; SNOW1gv1 steps widen to 6 h/12 h beyond day 3",
                auth: "none (keyless open data)",
                coverage: "Nordics and adjacent seas",
                granularity: "1 h, widening to 6 h and 12 h with the horizon",
                limits: "no published quota; SMHI's fair-use rules apply",
                verified: "2026-09-30",
                implemented: true,
                alerts: false,
                licence: Some("SMHI (CC BY 4.0 SE)"),
            },
            Self::Metar => ProviderMeta {
                id: *self,
                display_name: "METAR",
                requires_key: false,
                key_env: None,
                docs_url: "https://aviationweather.gov/data/api/",
                max_days: 0,
                history_days: 0,
                marine: false,
                current: true,
                hourly: false,
                daily: false,
                location_kinds: LocationKinds::STATION_AND_LAT_LON,
                notes: "keyless, station observations only; 100 requests/minute; `--station` selects it",
                auth: "none (keyless)",
                coverage: "worldwide stations",
                granularity: "per observation (≈hourly)",
                limits: "100 requests/minute; 400 entries per response",
                verified: "2026-10-01",
                implemented: true,
                alerts: false,
                licence: Some("aviationweather.gov (NOAA/NWS, public domain)"),
            },
        }
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProviderId {
    type Err = Error;

    /// Case-insensitive, and tolerant of `-`/`_` separators: `Open-Meteo`, `open_meteo` and
    /// `openmeteo` all name the same provider.
    fn from_str(input: &str) -> Result<Self> {
        let normalized: String = input
            .chars()
            .filter(|c| *c != '-' && *c != '_')
            .flat_map(char::to_lowercase)
            .collect();

        match normalized.as_str() {
            "openmeteo" => Ok(Self::OpenMeteo),
            "metno" => Ok(Self::MetNo),
            "openmeteoarchive" => Ok(Self::OpenMeteoArchive),
            "openmeteomarine" => Ok(Self::OpenMeteoMarine),
            "visualcrossing" => Ok(Self::VisualCrossing),
            "openweathermap" => Ok(Self::OpenWeatherMap),
            "weatherapi" => Ok(Self::WeatherApi),
            "worldweatheronline" => Ok(Self::WorldWeatherOnline),
            "pirateweather" => Ok(Self::PirateWeather),
            "qweather" => Ok(Self::QWeather),
            "smhi" => Ok(Self::Smhi),
            "metar" => Ok(Self::Metar),
            _ => Err(Error::Usage(format!(
                "unknown provider `{input}`; known providers: {}",
                Self::known_ids()
            ))),
        }
    }
}

/// What one backend offers, as far as the registry knows.
///
/// The boolean fields are the capability flags of the provider contract; they are deliberately
/// flat instead of being folded into enums, because each one is independent.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderMeta {
    /// The provider this row describes.
    pub id: ProviderId,
    /// Name shown to users.
    pub display_name: &'static str,
    /// Whether an API key has to be present before the provider can be used.
    pub requires_key: bool,
    /// Environment variable that supplies the key, if any.
    pub key_env: Option<&'static str>,
    /// Where the provider documents its API.
    pub docs_url: &'static str,
    /// Longest forecast the provider serves, in days (`0` = observations only).
    pub max_days: u8,
    /// How many days back the provider can answer for (`0` = no archive).
    ///
    /// The machine-readable form of "archive only" is `max_days: 0` with `history_days > 0`; the
    /// CLI refuses `--days` for such an entry and accepts `--date`/`--history` only when some
    /// chain entry's span covers the request.
    pub history_days: u16,
    /// Whether the backend serves marine data (waves, swell, sea-surface temperature).
    ///
    /// The one backend that sets it is a *supplementary* source: it is never a forecast chain
    /// entry, and `--marine` is what requests it (step 23).
    pub marine: bool,
    /// Whether current conditions are available.
    pub current: bool,
    /// Whether hourly data is available.
    pub hourly: bool,
    /// Whether daily data is available.
    pub daily: bool,
    /// Which location forms the provider accepts.
    pub location_kinds: LocationKinds,
    /// Free-form caveats worth showing in `provider info`.
    pub notes: &'static str,
    /// How the credential travels, e.g. ``API key in the `appid` query parameter``.
    pub auth: &'static str,
    /// The area the backend answers for, e.g. `global` or `Nordics and adjacent seas`.
    pub coverage: &'static str,
    /// The native time step, e.g. `3-hourly (40 slots ≈ 5 days)`.
    pub granularity: &'static str,
    /// The documented free-tier quotas and rate limits, in the provider's own words.
    pub limits: &'static str,
    /// The date (`YYYY-MM-DD`) this row was last checked against the provider's live documentation.
    ///
    /// The evidence behind the date — endpoints, quotas with their wording, licence obligations and
    /// the traps the implementation must respect — lives in `docs/providers.md`. A row whose date is
    /// old is a claim to re-check, not a fact to trust.
    pub verified: &'static str,
    /// Whether a [`Provider`] implementation exists yet.
    ///
    /// A registry row can be complete before its backend is written; `select` refuses to hand out
    /// an id whose row says `false`, so no code path can end up in the factory's "not implemented"
    /// arm by accident.
    pub implemented: bool,
    /// Whether the provider reports weather alerts (step 15 fills the first `true`).
    pub alerts: bool,
    /// The credit line the data licence requires, e.g. `Open-Meteo.com (CC BY 4.0)`.
    ///
    /// `None` until the row's provider is implemented, so that no renderer can print a credit for a
    /// backend that never fetches. The licence *obligation* of every backend — implemented or not —
    /// is recorded in `docs/providers.md`; this field is only the line the renderers print.
    pub licence: Option<&'static str>,
}

/// Which location forms a provider can answer for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocationKinds {
    /// Resolved place names.
    pub city: bool,
    /// METAR station identifiers.
    pub station: bool,
    /// Raw `lat,lon` coordinates.
    pub lat_lon: bool,
}

impl LocationKinds {
    /// Resolved place names only.
    pub const CITY: Self = Self {
        city: true,
        station: false,
        lat_lon: false,
    };

    /// Resolved place names and coordinates — the usual case for gridded models.
    pub const CITY_AND_LAT_LON: Self = Self {
        city: true,
        station: false,
        lat_lon: true,
    };

    /// Station identifiers only.
    pub const STATION: Self = Self {
        city: false,
        station: true,
        lat_lon: false,
    };

    /// Stations and raw coordinates — the aviation backend, which snaps a coordinate pair to its
    /// nearest embedded station.
    pub const STATION_AND_LAT_LON: Self = Self {
        city: false,
        station: true,
        lat_lon: true,
    };

    /// Human readable summary of the accepted forms, e.g. `city, lat/lon`.
    pub fn summary(self) -> String {
        let mut labels = Vec::new();
        if self.city {
            labels.push("city");
        }
        if self.station {
            labels.push("station");
        }
        if self.lat_lon {
            labels.push("lat/lon");
        }
        labels.join(", ")
    }
}

// ---------------------------------------------------------------------------------------------
// The behaviour contract
// ---------------------------------------------------------------------------------------------

impl ProviderMeta {
    /// This row as the contract's [`Capabilities`] value.
    ///
    /// Deriving instead of hand-writing the struct in each implementation is what keeps
    /// `provider list` honest: there is exactly one place that says how many days a backend can
    /// serve, and it is the row a user reads.
    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            current: self.current,
            hourly: self.hourly,
            daily: self.daily,
            alerts: self.alerts,
            max_days: self.max_days,
            history_days: self.history_days,
            marine: self.marine,
            requires_key: self.requires_key,
            key_env: self.key_env,
            location_kinds: self.location_kinds,
        }
    }
}

/// What one backend offers, in the shape the provider contract fixes.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Current conditions are available.
    pub current: bool,
    /// Hourly data is available.
    pub hourly: bool,
    /// Daily data is available.
    pub daily: bool,
    /// Weather alerts are available (step 15).
    pub alerts: bool,
    /// Longest forecast the backend serves, in days (`0` = observations only).
    pub max_days: u8,
    /// How many days back the backend can answer for (`0` = no archive).
    pub history_days: u16,
    /// Whether the backend serves marine data; the one that sets it is supplementary (step 23).
    pub marine: bool,
    /// Whether an API key has to be present before the backend can be used.
    pub requires_key: bool,
    /// Environment variable that supplies the key, when there is one.
    pub key_env: Option<&'static str>,
    /// Which location forms the backend accepts.
    pub location_kinds: LocationKinds,
}

/// How much detail a caller wants from the hourly data.
///
/// A backend that only serves coarser steps ignores the requested resolution and answers with what
/// it has; the day-part aggregation works on whatever arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HourlyResolution {
    /// One sample per hour.
    Hourly,
}

/// One fetch: for how many days, at what resolution, and over which absolute window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchRequest {
    /// Forecast days requested; `0` means current conditions only.
    pub days: u8,
    /// Detail requested from the hourly data.
    pub hourly_resolution: HourlyResolution,
    /// An absolute window (`--date`, `--history`), which replaces the forecast span.
    ///
    /// Only a backend whose registry row declares `history_days > 0` receives one: the CLI refuses
    /// the flags otherwise. Such a backend serves exactly `start..=end` and ignores `max_days`
    /// (the archive is not a forecast); `days` still carries the window's length so the cache key
    /// stays honest.
    pub window: Option<DateWindow>,
}

/// An absolute date window, inclusive on both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateWindow {
    /// First local date the answer must cover.
    pub start: NaiveDate,
    /// Last local date the answer must cover.
    pub end: NaiveDate,
}

impl DateWindow {
    /// A window covering exactly one date.
    #[must_use]
    pub const fn day(date: NaiveDate) -> Self {
        Self {
            start: date,
            end: date,
        }
    }

    /// The window's length in days, inclusive.
    #[must_use]
    pub fn days(self) -> u8 {
        let span = self.end.signed_duration_since(self.start).num_days();
        u8::try_from(span.clamp(0, i64::from(u8::MAX)) + 1).unwrap_or(u8::MAX)
    }

    /// Whether the window ends before `today`, i.e. whether its data is historical.
    #[must_use]
    pub fn is_archive(self, today: NaiveDate) -> bool {
        self.end < today
    }
}

impl FetchRequest {
    /// A request for `days` days of hourly data (`0` = current conditions only).
    #[must_use]
    pub const fn new(days: u8, hourly_resolution: HourlyResolution) -> Self {
        Self {
            days,
            hourly_resolution,
            window: None,
        }
    }

    /// The same request, restricted to an absolute window.
    ///
    /// `days` becomes the window's length: the caller cannot ask for a span and a day count at
    /// once, and the cache key is built from the span that is actually requested.
    #[must_use]
    pub fn for_window(window: DateWindow, hourly_resolution: HourlyResolution) -> Self {
        Self {
            days: window.days(),
            hourly_resolution,
            window: Some(window),
        }
    }

    /// Whether this request asked for historical data, given the location's local today.
    #[must_use]
    pub fn mode(&self, today: NaiveDate) -> crate::model::ReportMode {
        match self.window {
            Some(window) if window.is_archive(today) => crate::model::ReportMode::Archive,
            _ => crate::model::ReportMode::Forecast,
        }
    }
}

/// Everything a backend may touch while fetching.
///
/// Providers never open sockets or read files themselves: the shared [`HttpClient`] carries the
/// retry policy, [`Cache`] the on-disk answers and [`KeyStore`] the credentials. `quiet`/`verbose`
/// are the run's output flags — the chain warns about a fallback and a provider reports a clamp,
/// and both must respect `-q`/`-v`.
pub struct Env<'a> {
    /// The shared HTTP client.
    pub http: &'a HttpClient,
    /// The on-disk cache, already in the run's mode.
    pub cache: &'a Cache,
    /// The validated configuration.
    pub config: &'a Config,
    /// The key store, for the backends that need one.
    pub keys: &'a KeyStore,
    /// `-q`: suppress non-essential notes and warnings.
    pub quiet: bool,
    /// `-v` level.
    pub verbose: u8,
}

/// One upstream JSON request plus the cache policy it runs under.
///
/// Built by a provider — only it knows the URL, the parameters and the cache key shape — and
/// executed by [`fetch_json`]. A credential in the request is marked with
/// [`HttpRequest::secret`] so the redaction reaches the log lines, the error messages and the
/// cache envelope.
pub struct JsonFetch<'a> {
    /// The backend the request belongs to; names the provider in errors.
    pub provider: ProviderId,
    /// The request to send.
    pub request: HttpRequest,
    /// Where the answer is cached. The caller owns the shape: it knows the location, the day count
    /// and the location-local date the key is built from.
    pub key: CacheKey,
    /// How long the cached answer stays fresh.
    pub ttl: Duration,
    /// The one HTTP call behind this fetch, for the `-v` line the provider prints.
    pub what: &'a str,
}

/// Fetches and decodes one JSON answer, keeping the cross-cutting policy in one place.
///
/// * the cache decides whether a request happens at all (`--no-cache`, `--refresh`, `--offline`
///   and the TTL come from [`Cache`]); a cached body that no longer parses is refetched;
/// * [`HttpClient`] owns retries, backoff and `Retry-After`, and refuses a non-identity
///   `Content-Encoding` (the transparent decoder is disabled);
/// * a `401` becomes [`Error::InvalidKey`] (exit 6) because the credential is the user's to fix and
///   neither a retry nor a fallback can change it. A `403` deliberately stays
///   [`Error::Upstream`]: it carries quota, plan, permission and host-mismatch refusals whose body
///   text is the actionable part, and a provider that can tell "the key is invalid" apart from
///   "the plan is exhausted" refines it in its own decoder;
/// * `429`, `5xx` and transport failures keep their taxonomy ([`Error::Upstream`] /
///   [`Error::Network`]), which is what lets a chain fall through to the next backend;
/// * decoding happens here, so a schema change names the provider rather than the cache.
pub fn fetch_json<T: DeserializeOwned>(
    env: &Env<'_>,
    loc: &Location,
    fetch: &JsonFetch<'_>,
) -> Result<T> {
    if env.verbose > 1 {
        eprintln!(
            "provider: {} {} (cache {})",
            fetch.provider,
            fetch.what,
            env.cache.mode().name()
        );
    }
    let place = format!("{} ({:.2}, {:.2})", loc.name, loc.lat, loc.lon);
    env.cache
        .read_or_fetch_json(
            &fetch.key,
            fetch.ttl,
            fetch.provider.as_str(),
            "forecast",
            &place,
            || {
                let response = env.http.send(&fetch.request)?;
                Ok((response.status(), response.body().to_owned()))
            },
        )
        .map_err(|error| rejected_key(error, fetch.provider))
}

/// Turns a provider's `401` into [`Error::InvalidKey`]; every other error keeps its taxonomy.
fn rejected_key(error: Error, provider: ProviderId) -> Error {
    match error {
        Error::Upstream {
            status: Some(401), ..
        } => Error::InvalidKey {
            provider: provider.to_string(),
            status: 401,
        },
        other => other,
    }
}

/// The number of forecast days to ask for: `0` stays `0` (current conditions only), everything
/// else is capped at what the backend serves, with one warning when the cap bites.
pub(crate) fn requested_days(requested: u8, max_days: u8, provider: &str, quiet: bool) -> u8 {
    if requested == 0 {
        return 0;
    }
    let days = requested.min(max_days);
    if days != requested && !quiet {
        eprintln!(
            "warning: {provider} serves at most {max_days} days; showing {days} of the {requested} requested"
        );
    }
    days
}

/// A `-v` note when a backend's own step size leaves the tail of the request without a complete
/// local day, so the built report carries fewer days than `requested_days` asked for even though
/// the registry's `max_days` never bit (SMHI's 6 h/12 h steps are the standing example). `built` is
/// the number of days the report actually carries and `requested` the count after the cap.
pub(crate) fn note_short_series(built: usize, requested: u8, provider: &str, env: &Env<'_>) {
    if env.verbose > 0 && !env.quiet && built < usize::from(requested) {
        eprintln!(
            "note: {provider} steps leave the tail of the range incomplete; showing {built} of the {requested} requested days"
        );
    }
}

/// The location's current calendar date, from the injected clock.
///
/// This is part of a weather cache key, so the entry a run reads is always keyed by the day it was
/// fetched for: a rollover misses by key rather than by luck with a TTL.
pub(crate) fn local_today(env: &Env<'_>, tz: Tz) -> NaiveDate {
    let now: DateTime<Utc> = env.cache.clock().now().into();
    now.with_timezone(&tz).date_naive()
}

/// One weather backend.
///
/// Implementations are synchronous and stateless (`&self`, no interior mutability), so a provider
/// value is cheap to create and cannot smuggle state between runs.
pub trait Provider {
    /// The registry id, which must match the row in [`ProviderId::metadata`].
    fn id(&self) -> ProviderId;

    /// What this backend offers; implementations return their registry row's
    /// [`ProviderMeta::capabilities`].
    fn capabilities(&self) -> Capabilities;

    /// Fetches one forecast for `loc`.
    ///
    /// This is the one path every backend's answer takes, so the finite-reading guard lives here:
    /// a report that carries `inf` or `NaN` in a weather reading is refused as [`Error::Upstream`]
    /// instead of being stored, cached and rendered as if it were a measurement.
    fn fetch(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report> {
        let mut report = self.fetch_report(loc, req, env)?;
        validate_readings(self.id().as_str(), &report)?;
        // The mode is a property of the *request* (`--date`/`--history`), not of the backend, so
        // it is stamped here — the one path every backend's answer takes — instead of in each
        // decoder.
        report.mode = req.mode(local_today(env, loc.tz));
        Ok(report)
    }

    /// Fetches one forecast for `loc`, without the finite-reading guard.
    ///
    /// Backends implement this; callers use [`Provider::fetch`], which adds the guard.
    fn fetch_report(&self, loc: &Location, req: &FetchRequest, env: &Env<'_>) -> Result<Report>;
}

/// The implementation behind a registry id.
///
/// An id whose row says `implemented: false` has no backend yet; [`select`] keeps such an id out of
/// every chain, so this error is only reachable through a hand-built chain.
pub fn provider_for(id: ProviderId) -> Result<Box<dyn Provider>> {
    match id {
        ProviderId::OpenMeteo => Ok(Box::new(open_meteo::OpenMeteo)),
        ProviderId::OpenMeteoMarine => Ok(Box::new(open_meteo_marine::OpenMeteoMarine)),
        ProviderId::MetNo => Ok(Box::new(met_no::MetNo)),
        ProviderId::OpenMeteoArchive => Ok(Box::new(open_meteo_archive::OpenMeteoArchive)),
        ProviderId::VisualCrossing => Ok(Box::new(visualcrossing::VisualCrossing)),
        ProviderId::OpenWeatherMap => Ok(Box::new(openweathermap::OpenWeatherMap)),
        ProviderId::PirateWeather => Ok(Box::new(pirateweather::PirateWeather)),
        ProviderId::QWeather => Ok(Box::new(qweather::QWeather)),
        ProviderId::Smhi => Ok(Box::new(smhi::Smhi)),
        ProviderId::WeatherApi => Ok(Box::new(weatherapi::WeatherApi)),
        ProviderId::WorldWeatherOnline => Ok(Box::new(worldweatheronline::WorldWeatherOnline)),
        ProviderId::Metar => Ok(Box::new(metar::Metar)),
    }
}

/// The credit line a provider's data licence requires, for the renderers that print it.
///
/// An unknown or unimplemented provider id has no verified licence row and therefore no line: the
/// renderer prints what the registry knows rather than inventing a credit.
#[must_use]
pub fn licence_line(provider: &str) -> Option<&'static str> {
    provider
        .parse::<ProviderId>()
        .ok()
        .and_then(|id| id.metadata().licence)
}

impl Capabilities {
    /// This capability set as the report-side mirror the renderers read.
    ///
    /// The mirror exists so that `src/render` can shape its output from what a backend declared
    /// without importing this module (the render-path audit in step 12 forbids that import).
    #[must_use]
    pub fn report(self) -> ReportCapabilities {
        ReportCapabilities {
            current: self.current,
            hourly: self.hourly,
            daily: self.daily,
            alerts: self.alerts,
            max_days: self.max_days,
            history_days: self.history_days,
            marine: self.marine,
            requires_key: self.requires_key,
            key_env: self.key_env.map(str::to_owned),
            locations: ReportLocationKinds {
                city: self.location_kinds.city,
                station: self.location_kinds.station,
                lat_lon: self.location_kinds.lat_lon,
            },
        }
    }
}

/// The [`Attribution`] block of a report produced by `id`.
///
/// Every provider builds its report through this constructor, so the renderers receive the display
/// name, the licence credit and the capability flags *with the data* instead of looking the
/// provider up again. A provider whose registry row is missing would be a bug; the report still
/// parses because the fields are optional on the model.
#[must_use]
pub fn attribution(
    id: ProviderId,
    url: String,
    fetched_at: DateTime<Utc>,
    raw: Option<String>,
) -> Attribution {
    let meta = id.metadata();
    Attribution {
        provider: id.as_str().to_owned(),
        display_name: meta.display_name.to_owned(),
        licence: meta.licence.map(str::to_owned),
        capabilities: Some(meta.capabilities().report()),
        url,
        fetched_at,
        raw,
    }
}

/// What the backend behind a report says it offers, for the renderers that shape their output
/// around it.
///
/// A renderer must never branch on the provider *id* (`metar` has no forecast, so a `days` table
/// would be an empty box): it asks this function, which answers from the same registry row
/// `provider list` prints. `None` for a provider string no registry row claims — a hand-written
/// report or a backend removed in a later version — and the renderers then draw the neutral shape.
#[must_use]
pub fn capabilities_of(provider: &str) -> Option<Capabilities> {
    provider
        .parse::<ProviderId>()
        .ok()
        .filter(|id| id.metadata().implemented)
        .map(|id| id.metadata().capabilities())
}

/// The name a message should use for a backend (`METAR`, `Open-Meteo`), from the registry row.
#[must_use]
pub fn display_name_of(provider: &str) -> Option<&'static str> {
    provider
        .parse::<ProviderId>()
        .ok()
        .map(|id| id.metadata().display_name)
}

/// The chain a `--provider` value names.
///
/// * `auto` expands to every implemented keyless backend that answers for a resolved place, in
///   registry order (the filter [`auto_chain`] applies), and never a station-only backend such as
///   `metar` (a station has to be requested explicitly).
/// * anything else is an explicit ordered chain: each entry must name a known, implemented
///   provider, and duplicates collapse to their first position.
/// * an unknown id is [`Error::Usage`] listing the known ids; a known but unimplemented one is the
///   same variant with `not implemented yet`.
pub fn select(spec: &str) -> Result<Vec<ProviderId>> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(Error::Usage(
            "no provider selected; pass `--provider <id>` or set defaults.provider".to_owned(),
        ));
    }

    let mut ids = Vec::new();
    for token in spec.split(',') {
        let token = token.trim();
        if token.is_empty() {
            return Err(Error::Usage(format!(
                "empty entry in the provider list `{spec}`"
            )));
        }
        if token.eq_ignore_ascii_case("auto") {
            for id in auto_chain()? {
                push_unique(&mut ids, id);
            }
            continue;
        }
        let id: ProviderId = token.parse()?;
        if id.metadata().marine {
            return Err(Error::Usage(format!(
                "provider `{id}` is a supplementary marine source; pass `--marine` instead of `--provider {id}`"
            )));
        }
        if !id.metadata().implemented {
            return Err(Error::Usage(format!(
                "provider `{id}` is not implemented yet"
            )));
        }
        push_unique(&mut ids, id);
    }

    if ids.is_empty() {
        return Err(Error::Config(
            "the provider chain is empty; pass `--provider <id>`".to_owned(),
        ));
    }
    Ok(ids)
}

/// The keyless backends `auto` expands to, in order.
///
/// Step 23 ships the interim fixed list `open-meteo, met-no, smhi` (the implemented keyless
/// backends that answer for a resolved place); step 24 replaces this with the coverage-ranked
/// expansion over the registry's `covers` metadata, and `metar` never enters either shape (a
/// station has to be requested explicitly). Rows whose `implemented` flag is still `false` are
/// filtered out, so the list is always a set of backends that can actually answer.
fn auto_chain() -> Result<Vec<ProviderId>> {
    let ids: Vec<ProviderId> = [ProviderId::OpenMeteo, ProviderId::MetNo, ProviderId::Smhi]
        .into_iter()
        .filter(|id| id.metadata().implemented)
        .collect();
    if ids.is_empty() {
        return Err(Error::Config(
            "no provider is available without an API key; set one and use `--provider <id>`"
                .to_owned(),
        ));
    }
    Ok(ids)
}

/// Appends `id` unless the chain already names it.
fn push_unique(ids: &mut Vec<ProviderId>, id: ProviderId) {
    if !ids.contains(&id) {
        ids.push(id);
    }
}

/// Fetches from the first backend of `ids` that answers.
///
/// The chain falls through to the next entry **only** when the failure is transport-level
/// ([`Error::Network`]) or comes from upstream ([`Error::Upstream`]): a usage, config, location,
/// missing-key or body-decoding error is the user's answer and stops the walk. A missing key is
/// reported before the backend is called at all, so a chain entry cannot be "tried" without its
/// credential. Unless `-q` is set, each fallback prints one `warning:` line naming the reason.
pub fn fetch_chain(
    ids: &[ProviderId],
    loc: &Location,
    req: &FetchRequest,
    env: &Env<'_>,
) -> Result<Report> {
    fetch_chain_with(ids, loc, req, env, provider_for)
}

/// [`fetch_chain`] with an injectable factory, so unit tests can script backend outcomes.
#[allow(clippy::trivially_copy_pass_by_ref)] // the contract fixes `req: &FetchRequest`
fn fetch_chain_with(
    ids: &[ProviderId],
    loc: &Location,
    req: &FetchRequest,
    env: &Env<'_>,
    build: impl Fn(ProviderId) -> Result<Box<dyn Provider>>,
) -> Result<Report> {
    if ids.is_empty() {
        return Err(Error::Config(
            "the provider chain is empty; pass `--provider <id>`".to_owned(),
        ));
    }

    let mut attempts = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        let provider = build(*id)?;
        let capabilities = provider.capabilities();

        if capabilities.requires_key {
            let variable = capabilities.key_env.ok_or_else(|| {
                Error::Config(format!(
                    "provider `{id}` declares that it needs a key but names no environment variable"
                ))
            })?;
            if env.keys.get(id.as_str())?.is_none() {
                return Err(Error::MissingKey {
                    provider: id.to_string(),
                    env: variable.to_owned(),
                });
            }
        }

        if env.verbose > 1 {
            eprintln!("provider: {id} attempt {}/{}", index + 1, ids.len());
        }

        match provider.fetch(loc, req, env) {
            Ok(report) => return Ok(report),
            Err(error) if matches!(error, Error::Network(_) | Error::Upstream { .. }) => {
                if env.verbose > 1 {
                    eprintln!("provider: {id} failed: {error}");
                }
                if let Some(next) = ids.get(index + 1)
                    && !env.quiet
                {
                    eprintln!(
                        "warning: {id} failed ({}); falling back to {next}",
                        error.chain_reason()
                    );
                }
                attempts.push(format!("{id} ({})", error.chain_reason()));
            }
            Err(error) => return Err(error),
        }
    }

    if attempts.is_empty() {
        return Err(Error::Config(
            "no provider could answer; pass `--provider <id>`".to_owned(),
        ));
    }
    Err(Error::Chain {
        subject: "providers",
        attempts,
    })
}

/// Rejects a report that carries a non-finite weather reading.
///
/// Every backend gets its numbers from upstream, and a value that is not finite (`inf`/`NaN`) is
/// not a measurement: it prints as `inf°C` in the text formats, serialises as `null` in JSON (which
/// the schema calls a non-nullable number) and would be cached as if it were data. The check runs in
/// [`Provider::fetch`], the one path every backend's answer takes, so no backend can forget it; the
/// message names the provider and the field, and the run treats it like any other upstream failure
/// (a chain falls through to the next backend).
fn validate_readings(provider: &str, report: &Report) -> Result<()> {
    if let Some(current) = &report.current {
        finite(provider, current.temp_c, || "current.temp_c".to_owned())?;
        optional_finite(provider, current.feels_like_c, || {
            "current.feels_like_c".to_owned()
        })?;
        finite(provider, current.precip_mm, || {
            "current.precip_mm".to_owned()
        })?;
        finite(provider, current.pressure_hpa, || {
            "current.pressure_hpa".to_owned()
        })?;
        finite(provider, current.wind_kmh, || "current.wind_kmh".to_owned())?;
        optional_finite(provider, current.wind_gust_kmh, || {
            "current.wind_gust_kmh".to_owned()
        })?;
        optional_finite(provider, current.visibility_km, || {
            "current.visibility_km".to_owned()
        })?;
        optional_finite(provider, current.uv_index, || "current.uv_index".to_owned())?;
    }
    for (day_index, day) in report.days.iter().enumerate() {
        finite(provider, day.temp_min_c, || {
            format!("days[{day_index}].temp_min_c")
        })?;
        finite(provider, day.temp_max_c, || {
            format!("days[{day_index}].temp_max_c")
        })?;
        for (part_index, part) in day.parts.iter().enumerate() {
            finite(provider, part.temp_c, || {
                format!("days[{day_index}].parts[{part_index}].temp_c")
            })?;
            optional_finite(provider, part.feels_like_c, || {
                format!("days[{day_index}].parts[{part_index}].feels_like_c")
            })?;
            finite(provider, part.precip_mm, || {
                format!("days[{day_index}].parts[{part_index}].precip_mm")
            })?;
            finite(provider, part.wind_kmh, || {
                format!("days[{day_index}].parts[{part_index}].wind_kmh")
            })?;
            optional_finite(provider, part.visibility_km, || {
                format!("days[{day_index}].parts[{part_index}].visibility_km")
            })?;
        }
    }
    Ok(())
}

/// One required reading through the finite check; `field` is named only when the check fails.
fn finite(provider: &str, value: f32, field: impl FnOnce() -> String) -> Result<()> {
    if value.is_finite() {
        return Ok(());
    }
    Err(Error::Upstream {
        provider: provider.to_owned(),
        status: None,
        message: format!("the reading `{}` is not finite ({value})", field()),
    })
}

/// One optional reading: an absent value stays absent, a present one must be finite.
fn optional_finite(
    provider: &str,
    value: Option<f32>,
    field: impl FnOnce() -> String,
) -> Result<()> {
    value.map_or(Ok(()), |value| finite(provider, value, field))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::{NaiveDate, Utc};
    use chrono_tz::Tz;

    use super::{
        Capabilities, DateWindow, Env, FetchRequest, HourlyResolution, LocationKinds, Provider,
        ProviderId, fetch_chain_with, provider_for, select,
    };
    use crate::cache::{Cache, CacheMode, SystemClock};
    use crate::config::Config;
    use crate::config::keys::KeyStore;
    use crate::error::Error;
    use crate::http::{HttpClient, StubTransport};
    use crate::model::{Attribution, Location, LocationSource, Report};
    use crate::paths::Paths;

    #[test]
    fn every_provider_parses_from_its_canonical_spelling() {
        for id in ProviderId::all() {
            assert_eq!(ProviderId::from_str(id.as_str()).ok(), Some(id));
            assert_eq!(
                ProviderId::from_str(&id.as_str().to_uppercase()).ok(),
                Some(id)
            );
        }
    }

    #[test]
    fn separators_are_ignored_when_parsing() {
        assert_eq!(
            ProviderId::from_str("open-meteo").ok(),
            Some(ProviderId::OpenMeteo)
        );
        assert_eq!(
            ProviderId::from_str("open_meteo").ok(),
            Some(ProviderId::OpenMeteo)
        );
        assert_eq!(
            ProviderId::from_str("Open-Weather-Map").ok(),
            Some(ProviderId::OpenWeatherMap)
        );
    }

    #[test]
    fn unknown_ids_are_usage_errors() {
        let error = ProviderId::from_str("nope").unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("unknown provider `nope`"));
    }

    #[test]
    fn metadata_keeps_keys_and_location_forms_consistent() {
        for id in ProviderId::all() {
            let meta = id.metadata();
            assert_eq!(meta.id, id);
            assert_eq!(
                meta.requires_key,
                meta.key_env.is_some(),
                "{id} declares a key but has no env var, or the other way round"
            );
            if let Some(env) = meta.key_env {
                assert!(env.starts_with("CIRROCAST_"), "{env} is not namespaced");
            }
        }
    }

    #[test]
    fn every_row_carries_a_verified_date() {
        for id in ProviderId::all() {
            let meta = id.metadata();
            assert!(
                chrono::NaiveDate::parse_from_str(meta.verified, "%Y-%m-%d").is_ok(),
                "{id} carries `{}` instead of an ISO `verified` date",
                meta.verified
            );
        }
    }

    #[test]
    fn metar_is_station_based_and_forecastless() {
        let meta = ProviderId::Metar.metadata();
        assert_eq!(meta.max_days, 0);
        assert_eq!(meta.location_kinds, LocationKinds::STATION_AND_LAT_LON);
        assert!(meta.current && !meta.daily && !meta.hourly);
        assert!(meta.implemented && !meta.requires_key);
        // A station-only backend never enters `auto`: it answers a station, not a place.
        assert!(!meta.location_kinds.city);
    }

    #[test]
    fn capabilities_mirror_the_registry_row() {
        for id in ProviderId::all() {
            let meta = id.metadata();
            let capabilities = meta.capabilities();
            assert_eq!(capabilities.current, meta.current);
            assert_eq!(capabilities.hourly, meta.hourly);
            assert_eq!(capabilities.daily, meta.daily);
            assert_eq!(capabilities.alerts, meta.alerts);
            assert_eq!(capabilities.max_days, meta.max_days);
            assert_eq!(capabilities.requires_key, meta.requires_key);
            assert_eq!(capabilities.key_env, meta.key_env);
            assert_eq!(capabilities.location_kinds, meta.location_kinds);
        }
    }

    #[test]
    fn the_report_mirror_carries_every_registry_field() {
        // `Attribution` carries a copy of the row so the renderers never import this module; the
        // copy is built here, so this test is the one place that proves nothing is dropped.
        let fetched = Utc::now();
        for id in ProviderId::all() {
            let meta = id.metadata();
            let attribution =
                super::attribution(id, "https://example.invalid/x".to_owned(), fetched, None);
            assert_eq!(attribution.provider, id.as_str());
            assert_eq!(attribution.display_name, meta.display_name);
            assert_eq!(attribution.licence.as_deref(), meta.licence);
            let mirror = attribution
                .capabilities
                .expect("every row exposes capabilities");
            assert_eq!(mirror.current, meta.current);
            assert_eq!(mirror.hourly, meta.hourly);
            assert_eq!(mirror.daily, meta.daily);
            assert_eq!(mirror.alerts, meta.alerts);
            assert_eq!(mirror.max_days, meta.max_days);
            assert_eq!(mirror.history_days, meta.history_days);
            assert_eq!(mirror.marine, meta.marine);
            assert_eq!(mirror.requires_key, meta.requires_key);
            assert_eq!(mirror.key_env.as_deref(), meta.key_env);
            assert_eq!(
                mirror.locations,
                crate::model::ReportLocationKinds {
                    city: meta.location_kinds.city,
                    station: meta.location_kinds.station,
                    lat_lon: meta.location_kinds.lat_lon,
                }
            );
        }
    }

    #[test]
    fn an_unimplemented_row_claims_neither_a_backend_nor_a_licence() {
        for id in ProviderId::all() {
            let meta = id.metadata();
            if !meta.implemented {
                assert!(
                    meta.licence.is_none(),
                    "{id} has no backend yet, so its licence row would be an unverified claim"
                );
            }
        }
        assert_eq!(
            ProviderId::OpenMeteo.metadata().licence,
            Some("Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/")
        );
    }

    #[test]
    fn auto_expands_to_the_implemented_keyless_chain() {
        assert_eq!(
            select("auto").expect("auto expands"),
            vec![ProviderId::OpenMeteo, ProviderId::MetNo, ProviderId::Smhi]
        );
        assert_eq!(
            select("AUTO").expect("the spelling is case insensitive"),
            vec![ProviderId::OpenMeteo, ProviderId::MetNo, ProviderId::Smhi]
        );
    }

    #[test]
    fn an_explicit_chain_keeps_order_and_drops_duplicates() {
        let ids = select("open-meteo, Open-Meteo ,open_meteo").expect("all three name one backend");
        assert_eq!(ids, vec![ProviderId::OpenMeteo]);
    }

    #[test]
    fn unknown_ids_are_usage_errors_and_planned_ones_are_refused() {
        let unknown = select("does-not-exist").expect_err("unknown id");
        assert_eq!(unknown.exit_code(), 2);
        assert!(
            unknown
                .to_string()
                .contains("unknown provider `does-not-exist`")
        );

        // A row that is not implemented is refused by `select` before any factory call, and the
        // factory keeps its own guard for a hand-built chain. Once every row is implemented the
        // loop is vacuous, which is the state the exit criteria check for.
        for id in ProviderId::all()
            .iter()
            .filter(|id| !id.metadata().implemented && !id.metadata().marine)
        {
            let refused = select(id.as_str()).expect_err("a planned row is not selectable");
            assert_eq!(refused.exit_code(), 2);
            assert!(
                refused.to_string().contains("not implemented yet"),
                "{refused}"
            );
            let error = provider_for(*id).err().expect("no backend exists");
            assert_eq!(error.exit_code(), 4);
        }
        assert_eq!(
            provider_for(ProviderId::Metar)
                .expect("metar has a backend")
                .id(),
            ProviderId::Metar
        );
    }

    #[test]
    fn a_supplementary_marine_source_is_never_a_chain_entry() {
        // The registry's `marine` row is a supplementary source: `--marine` requests it, and a
        // chain that names it is a usage error rather than a silent full-report fetch.
        for id in ProviderId::all().iter().filter(|id| id.metadata().marine) {
            let refused = select(id.as_str()).expect_err("not a chain entry");
            assert_eq!(refused.exit_code(), 2);
            assert!(refused.to_string().contains("--marine"), "{refused}");
        }
    }

    #[test]
    fn a_window_request_carries_its_span_and_its_mode() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 6).expect("a valid date");
        let forecast = FetchRequest::new(3, HourlyResolution::Hourly);
        assert_eq!(forecast.window, None);
        assert_eq!(forecast.mode(today), crate::model::ReportMode::Forecast);

        let single = DateWindow::day(NaiveDate::from_ymd_opt(2026, 9, 14).expect("a valid date"));
        let request = FetchRequest::for_window(single, HourlyResolution::Hourly);
        assert_eq!(request.days, 1);
        assert_eq!(request.mode(today), crate::model::ReportMode::Archive);

        let recent = DateWindow {
            start: NaiveDate::from_ymd_opt(2026, 9, 29).expect("a valid date"),
            end: DateWindow::day(NaiveDate::from_ymd_opt(2026, 10, 5).expect("a valid date")).end,
        };
        assert_eq!(recent.days(), 7);
        assert_eq!(
            FetchRequest::for_window(recent, HourlyResolution::Hourly).mode(today),
            crate::model::ReportMode::Archive
        );

        // A window that reaches into today (or beyond) is a forecast request, not an archive one.
        let spans_today = DateWindow {
            start: recent.start,
            end: today,
        };
        assert_eq!(
            FetchRequest::for_window(spans_today, HourlyResolution::Hourly).mode(today),
            crate::model::ReportMode::Forecast
        );
    }

    #[test]
    fn an_empty_selection_is_a_usage_error() {
        assert_eq!(select("  ").expect_err("no provider named").exit_code(), 2);
        assert_eq!(
            select("open-meteo,").expect_err("empty entry").exit_code(),
            2
        );
    }

    /// The four `Env` members plus the temp directory that keeps the key store and cache alive.
    struct Fixture {
        _directory: tempfile::TempDir,
        http: HttpClient,
        cache: Cache,
        config: Config,
        keys: KeyStore,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().expect("tempdir");
            let paths = Paths {
                config_dir: directory.path().join("config"),
                config_file: directory.path().join("config/config.toml"),
                keys_file: directory.path().join("config/keys.toml"),
                cache_dir: directory.path().join("cache"),
                data_dir: directory.path().join("data"),
            };
            Self {
                http: HttpClient::new(
                    Box::new(StubTransport::new(Vec::new())),
                    0,
                    Arc::new(SystemClock),
                    0,
                ),
                cache: Cache::with_root(
                    directory.path().join("cache"),
                    CacheMode::Normal,
                    Arc::new(SystemClock),
                    0,
                ),
                config: Config::default(),
                keys: KeyStore::new(&paths),
                _directory: directory,
            }
        }

        fn env(&self) -> Env<'_> {
            Env {
                http: &self.http,
                cache: &self.cache,
                config: &self.config,
                keys: &self.keys,
                quiet: true,
                verbose: 0,
            }
        }
    }

    /// What a stub backend answers.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Outcome {
        Report,
        Network,
        Upstream,
        Usage,
    }

    /// A backend with a scripted outcome, counting how often it was called.
    struct Stub {
        id: ProviderId,
        outcome: Outcome,
        calls: Arc<AtomicUsize>,
    }

    impl Provider for Stub {
        fn id(&self) -> ProviderId {
            self.id
        }

        fn capabilities(&self) -> Capabilities {
            self.id.metadata().capabilities()
        }

        fn fetch_report(
            &self,
            _loc: &Location,
            _req: &FetchRequest,
            _env: &Env<'_>,
        ) -> super::Result<Report> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.outcome {
                Outcome::Report => Ok(stub_report(self.id)),
                Outcome::Network => Err(Error::Network("connection reset".to_owned())),
                Outcome::Upstream => Err(Error::Upstream {
                    provider: self.id.to_string(),
                    status: Some(500),
                    message: "boom".to_owned(),
                }),
                Outcome::Usage => Err(Error::Usage("this location is not supported".to_owned())),
            }
        }
    }

    fn stub_report(id: ProviderId) -> Report {
        Report {
            location: test_location(),
            current: None,
            days: Vec::new(),
            alerts: Vec::new(),
            air: None,
            astro: None,
            marine: None,
            mode: crate::model::ReportMode::Forecast,
            attribution: Attribution::unregistered(
                id.to_string(),
                "https://example.invalid/forecast",
                Utc::now(),
                None,
            ),
        }
    }

    fn test_location() -> Location {
        Location {
            name: "Beijing".to_owned(),
            admin1: None,
            country: "China".to_owned(),
            country_code: Some("CN".to_owned()),
            lat: 39.9042,
            lon: 116.4074,
            tz: Tz::Asia__Shanghai,
            elevation_m: None,
            population: None,
            source: LocationSource::Geocoder,
            station: None,
        }
    }

    fn request() -> FetchRequest {
        FetchRequest::new(3, HourlyResolution::Hourly)
    }

    /// A factory over a scripted outcome table.
    fn factory<'a>(
        outcomes: &'a [(ProviderId, Outcome)],
        calls: &'a Arc<AtomicUsize>,
    ) -> impl Fn(ProviderId) -> super::Result<Box<dyn Provider>> + 'a {
        move |id| {
            let outcome = outcomes
                .iter()
                .find(|(candidate, _)| *candidate == id)
                .map(|(_, outcome)| *outcome)
                .ok_or_else(|| Error::Other(format!("no stub for {id}")))?;
            Ok(Box::new(Stub {
                id,
                outcome,
                calls: Arc::clone(calls),
            }))
        }
    }

    #[test]
    fn the_chain_returns_the_first_report() {
        let fixture = Fixture::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let outcomes = [(ProviderId::OpenMeteo, Outcome::Report)];
        let report = fetch_chain_with(
            &[ProviderId::OpenMeteo],
            &test_location(),
            &request(),
            &fixture.env(),
            factory(&outcomes, &calls),
        )
        .expect("the stub answers");
        assert_eq!(report.attribution.provider, "open-meteo");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn transport_and_upstream_failures_fall_through() {
        let fixture = Fixture::new();
        for outcome in [Outcome::Network, Outcome::Upstream] {
            let calls = Arc::new(AtomicUsize::new(0));
            let outcomes = [
                (ProviderId::OpenMeteo, outcome),
                (ProviderId::Smhi, Outcome::Report),
            ];
            let report = fetch_chain_with(
                &[ProviderId::OpenMeteo, ProviderId::Smhi],
                &test_location(),
                &request(),
                &fixture.env(),
                factory(&outcomes, &calls),
            )
            .expect("the second entry answers");
            assert_eq!(report.attribution.provider, "smhi");
            assert_eq!(calls.load(Ordering::SeqCst), 2);
        }
    }

    #[test]
    fn a_usage_error_stops_the_chain() {
        let fixture = Fixture::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let outcomes = [
            (ProviderId::OpenMeteo, Outcome::Usage),
            (ProviderId::Smhi, Outcome::Report),
        ];
        let error = fetch_chain_with(
            &[ProviderId::OpenMeteo, ProviderId::Smhi],
            &test_location(),
            &request(),
            &fixture.env(),
            factory(&outcomes, &calls),
        )
        .expect_err("a usage error is the user's answer");
        assert!(matches!(error, Error::Usage(_)));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the next entry must not be tried"
        );
    }

    #[test]
    fn an_exhausted_chain_reports_every_attempt() {
        let fixture = Fixture::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let outcomes = [
            (ProviderId::OpenMeteo, Outcome::Network),
            (ProviderId::Smhi, Outcome::Upstream),
        ];
        let error = fetch_chain_with(
            &[ProviderId::OpenMeteo, ProviderId::Smhi],
            &test_location(),
            &request(),
            &fixture.env(),
            factory(&outcomes, &calls),
        )
        .expect_err("both entries fail");
        assert_eq!(error.exit_code(), 3);
        let text = error.to_string();
        assert!(
            text.starts_with("all providers failed: open-meteo (network: "),
            "{text}"
        );
        assert!(text.contains("; smhi (upstream: "), "{text}");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_missing_key_is_reported_before_the_backend_runs() {
        let fixture = Fixture::new();
        if fixture.keys.get("qweather").expect("key lookup").is_some() {
            // A machine that exports CIRROCAST_QWEATHER_KEY cannot exercise this path.
            return;
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let outcomes = [
            (ProviderId::QWeather, Outcome::Report),
            (ProviderId::OpenMeteo, Outcome::Report),
        ];
        let error = fetch_chain_with(
            &[ProviderId::QWeather, ProviderId::OpenMeteo],
            &test_location(),
            &request(),
            &fixture.env(),
            factory(&outcomes, &calls),
        )
        .expect_err("no key is configured");
        assert_eq!(error.exit_code(), 6);
        let Error::MissingKey { provider, env } = error else {
            panic!("expected a missing-key error");
        };
        assert_eq!(provider, "qweather");
        assert_eq!(env, "CIRROCAST_QWEATHER_KEY");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "a backend without its key must not be called"
        );
    }
}
