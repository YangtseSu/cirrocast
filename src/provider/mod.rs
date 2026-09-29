// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Registry metadata for the weather backends `cirrocast` can talk to.
//!
//! This module is the single source of truth for *what* each backend offers: its command line id,
//! the environment variable holding its API key, how many forecast days it serves, which location
//! forms it understands and which data shapes it returns.
//!
//! The numbers below are **declared from provider documentation, not measured**. Step 10
//! (additional providers) re-verifies every row against the live API docs before the matching
//! backend is implemented, and corrects this table where reality disagrees. Treat the values as
//! claims, not as facts.
//!
//! The [`Provider`] trait and the per-provider implementations arrive in steps 06 and 10; this
//! module deliberately holds data only.

use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};

/// A backend `cirrocast` knows how to talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProviderId {
    /// Open-Meteo, keyless global model data (the default backend).
    OpenMeteo,
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
    pub const fn all() -> [Self; 8] {
        [
            Self::OpenMeteo,
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
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "keyless, global coverage",
            },
            Self::OpenWeatherMap => ProviderMeta {
                id: *self,
                display_name: "OpenWeatherMap",
                requires_key: true,
                key_env: Some("CIRROCAST_OPENWEATHERMAP_KEY"),
                docs_url: "https://openweathermap.org/forecast5",
                max_days: 5,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "free tier forecast is served in 3-hour steps",
            },
            Self::WeatherApi => ProviderMeta {
                id: *self,
                display_name: "WeatherAPI",
                requires_key: true,
                key_env: Some("CIRROCAST_WEATHERAPI_KEY"),
                docs_url: "https://www.weatherapi.com/docs/",
                max_days: 3,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "free tier",
            },
            Self::WorldWeatherOnline => ProviderMeta {
                id: *self,
                display_name: "World Weather Online",
                requires_key: true,
                key_env: Some("CIRROCAST_WORLDWEATHERONLINE_KEY"),
                docs_url: "https://www.worldweatheronline.com/weather-api/api/docs/",
                max_days: 3,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "free tier",
            },
            Self::PirateWeather => ProviderMeta {
                id: *self,
                display_name: "Pirate Weather",
                requires_key: true,
                key_env: Some("CIRROCAST_PIRATEWEATHER_KEY"),
                docs_url: "https://docs.pirateweather.net/",
                max_days: 7,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "Dark Sky shaped responses",
            },
            Self::QWeather => ProviderMeta {
                id: *self,
                display_name: "QWeather",
                requires_key: true,
                key_env: Some("CIRROCAST_QWEATHER_KEY"),
                docs_url: "https://dev.qweather.com/docs/api/",
                max_days: 7,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "China focused; needs a configured API host",
            },
            Self::Smhi => ProviderMeta {
                id: *self,
                display_name: "SMHI",
                requires_key: false,
                key_env: None,
                docs_url: "https://opendata.smhi.se/apidocs/metfcst/index.html",
                max_days: 10,
                current: true,
                hourly: true,
                daily: true,
                location_kinds: LocationKinds::CITY_AND_LAT_LON,
                notes: "keyless, Nordics only",
            },
            Self::Metar => ProviderMeta {
                id: *self,
                display_name: "METAR",
                requires_key: false,
                key_env: None,
                docs_url: "https://aviationweather.gov/data/api/",
                max_days: 0,
                current: true,
                hourly: false,
                daily: false,
                location_kinds: LocationKinds::STATION,
                notes: "keyless, station observations only",
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{LocationKinds, ProviderId};

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
    fn metar_is_station_only_and_forecastless() {
        let meta = ProviderId::Metar.metadata();
        assert_eq!(meta.max_days, 0);
        assert_eq!(meta.location_kinds, LocationKinds::STATION);
        assert!(!meta.daily && !meta.hourly);
    }
}
