// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The typed configuration document.
//!
//! `cirrocast` keeps its settings in `$XDG_CONFIG_HOME/cirrocast/config.toml`, falling back to the
//! `$XDG_CONFIG_DIRS` search path for system wide files. This module owns that document end to
//! end: the serde schema mirroring the architecture contract, loading with built-in defaults and
//! schema gating, validation, the atomic writer, the dotted-key accessors behind
//! `cirrocast config get/set`, and [`Settings`] — the resolved view the rest of the program
//! consumes.
//!
//! API keys are deliberately **not** part of this file: they live in their own `0600` document, see
//! [`keys`].

pub mod keys;

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{Error, Result};
use crate::geo::LocationSpec;
use crate::model::units::UnitSystem;
use crate::paths::Paths;
use crate::provider::ProviderId;

/// The schema version this build reads and writes.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Mode of a freshly written `config.toml` (Unix only; the shim ignores it elsewhere).
const CONFIG_FILE_MODE: u32 = 0o644;

/// Values accepted by `defaults.format`.
pub const FORMATS: &[&str] = &[
    "art-table",
    "one-line",
    "plain",
    "json",
    "dumb",
    "alerts",
    "aqi",
    "moon",
];

/// Values accepted by `defaults.units`.
pub const UNIT_SYSTEMS: &[&str] = &["metric", "us", "uk"];

/// Values accepted by `render.color`.
pub const COLOR_MODES: &[&str] = &["auto", "always", "never"];

/// Values accepted by `units.temp`.
pub const TEMP_UNITS: &[&str] = &["c", "f"];

/// Values accepted by `units.wind`.
pub const WIND_UNITS: &[&str] = &["kmh", "mph", "mps", "knots"];

/// Values accepted by `units.pressure`.
pub const PRESSURE_UNITS: &[&str] = &["hpa", "inhg", "mmhg"];

/// Values accepted by `units.distance`.
pub const DISTANCE_UNITS: &[&str] = &["km", "mi"];

/// Values accepted by `units.precip`.
pub const PRECIP_UNITS: &[&str] = &["mm", "in"];

/// Range of `defaults.days`.
const DAYS_RANGE: (u32, u32) = (0, 14);

/// Range of `network.timeout_secs`.
const TIMEOUT_RANGE: (u32, u32) = (1, 300);

/// Range of `network.retries`.
const RETRIES_RANGE: (u32, u32) = (0, 10);

/// Allowed values of `render.width` besides `0` ("detect").
const WIDTH_RANGE: (u32, u32) = (40, 500);

/// Allowed values of `[alerts] severity_threshold`; mirrors `model::alert::Severity::ALL`, and a
/// unit test keeps the two in step.
pub const SEVERITY_LEVELS: &[&str] = &["unknown", "minor", "moderate", "severe", "extreme"];

/// Allowed values of `[air] index`; mirrors `air::aqi::AqiIndex::ALL`, and a unit test keeps the
/// two in step.
pub const AQI_INDEXES: &[&str] = &["us", "european"];

/// The configuration document, matching the contract's TOML schema exactly.
///
/// Every table and field is optional on input: anything absent falls back to [`Config::default`],
/// and unknown keys are ignored so that a document written by a newer build keeps working for the
/// fields this build knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Schema version of the document.
    pub schema_version: u32,
    /// Defaults applied when the command line says nothing.
    pub defaults: Defaults,
    /// Default location handling.
    pub location: LocationDefaults,
    /// Per-quantity display overrides.
    pub units: UnitOverrides,
    /// Network behaviour.
    pub network: Network,
    /// On-disk cache behaviour.
    pub cache: CacheConfig,
    /// Output rendering.
    pub render: RenderConfig,
    /// Severe-weather alert fetching.
    pub alerts: AlertsConfig,
    /// Air-quality panel settings.
    pub air: AirConfig,
    /// Per-provider settings.
    pub providers: Providers,
}

/// `[defaults]` — what a bare `cirrocast` invocation uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Defaults {
    /// Provider id, comma separated chain, or `auto`.
    pub provider: String,
    /// Output format name.
    pub format: String,
    /// Unit system name.
    pub units: String,
    /// Forecast days.
    pub days: u8,
    /// Output language tag, or `auto`.
    pub language: String,
}

/// `[location]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocationDefaults {
    /// Location argument used when none is given on the command line.
    pub default: String,
}

/// `[units]` — per-quantity overrides on top of `defaults.units`.
///
/// An absent (or empty) entry means "follow the unit system". The values stay strings here; step 03
/// parses them into its own unit types when resolving a report for display.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UnitOverrides {
    /// Temperature unit override.
    #[serde(deserialize_with = "empty_as_none")]
    pub temp: Option<String>,
    /// Wind speed unit override.
    #[serde(deserialize_with = "empty_as_none")]
    pub wind: Option<String>,
    /// Pressure unit override.
    #[serde(deserialize_with = "empty_as_none")]
    pub pressure: Option<String>,
    /// Distance unit override.
    #[serde(deserialize_with = "empty_as_none")]
    pub distance: Option<String>,
    /// Precipitation unit override.
    #[serde(deserialize_with = "empty_as_none")]
    pub precip: Option<String>,
}

/// `[network]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Network {
    /// Per-request timeout in seconds.
    pub timeout_secs: u32,
    /// Retry attempts for retriable transport and upstream failures.
    pub retries: u32,
    /// Proxy URL; empty means "connect directly".
    pub proxy: String,
    /// Base URL of the Nominatim service; empty means the public OpenStreetMap endpoint.
    ///
    /// The OSM usage policy allows a service to be swapped without a software update, which is why
    /// this is configuration rather than a constant.
    pub nominatim_url: String,
}

/// `[cache]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    /// Whether the on-disk cache is used at all.
    pub enabled: bool,
    /// TTL of weather responses, in seconds.
    pub weather_ttl_secs: u32,
    /// TTL of public-IP lookups, in seconds.
    pub ip_ttl_secs: u32,
    /// TTL of geocoding results, in seconds.
    pub geocode_ttl_secs: u32,
}

/// `[render]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderConfig {
    /// Colour mode name.
    pub color: String,
    /// Line width: `0` = detect, otherwise the column count to lay out for.
    pub width: usize,
}

/// `[alerts]` — the severe-weather warning sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertsConfig {
    /// Whether alerts are fetched when `--alerts` is not given.
    pub enabled: bool,
    /// The lowest severity shown; alerts below it are dropped before rendering.
    pub severity_threshold: String,
    /// `["auto"]` (coverage-selected) or an explicit list of source ids.
    pub sources: Vec<String>,
    /// Base URL of the FOSS Public Alert Server; empty = the public `https://alerts.kde.org`.
    pub fpas_url: String,
    /// TTL of cached alert responses, in seconds.
    pub cache_ttl_secs: u32,
}

/// `[air]` — the air-quality panel (`--aqi`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AirConfig {
    /// Which AQI scale drives the panel's category colour and the `%q` token: `us` or `european`.
    pub index: String,
}

/// `[providers]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Providers {
    /// METAR settings.
    pub metar: MetarConfig,
    /// `QWeather` settings.
    pub qweather: QWeatherConfig,
}

/// `[providers.metar]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetarConfig {
    /// Default ICAO station identifier.
    pub station: String,
}

/// `[providers.qweather]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct QWeatherConfig {
    /// API host, assigned to your `QWeather` account; the console shows it at
    /// <https://console.qweather.com/setting>.
    pub host: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            defaults: Defaults::default(),
            location: LocationDefaults::default(),
            units: UnitOverrides::default(),
            network: Network::default(),
            cache: CacheConfig::default(),
            render: RenderConfig::default(),
            alerts: AlertsConfig::default(),
            air: AirConfig::default(),
            providers: Providers::default(),
        }
    }
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            provider: "open-meteo".to_owned(),
            format: "art-table".to_owned(),
            units: "metric".to_owned(),
            days: 3,
            language: "auto".to_owned(),
        }
    }
}

impl Default for Network {
    fn default() -> Self {
        Self {
            timeout_secs: 15,
            retries: 3,
            proxy: String::new(),
            nominatim_url: String::new(),
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            weather_ttl_secs: 600,
            ip_ttl_secs: 86_400,
            geocode_ttl_secs: 2_592_000,
        }
    }
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            color: "auto".to_owned(),
            width: 0,
        }
    }
}

impl Default for AlertsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            severity_threshold: "minor".to_owned(),
            sources: vec!["auto".to_owned()],
            fpas_url: String::new(),
            cache_ttl_secs: 300,
        }
    }
}

impl Default for AirConfig {
    fn default() -> Self {
        Self {
            index: "us".to_owned(),
        }
    }
}

/// Deserialiser for the `[units]` overrides: an empty string means "no override", so a
/// hand-written `temp = ""` behaves exactly like an absent key.
fn empty_as_none<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value.filter(|value| !value.is_empty()))
}

// ---------------------------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------------------------

impl Config {
    /// Loads the first configuration file found in XDG search order.
    ///
    /// No file anywhere yields the built-in defaults; a file that cannot be read, parsed or
    /// migrated is an [`Error::Config`] whose message starts with the offending path.
    pub fn load(paths: &Paths) -> Result<Self> {
        Self::load_with_source(paths).map(|(config, _)| config)
    }

    /// [`Config::load`], plus the file the configuration came from (`None` = built-in defaults).
    pub fn load_with_source(paths: &Paths) -> Result<(Self, Option<PathBuf>)> {
        let Some(path) = Self::source(paths)? else {
            return Ok((Self::default(), None));
        };
        let text = fs::read_to_string(&path)
            .map_err(|error| Error::Config(format!("{}: {error}", path.display())))?;
        Ok((Self::parse(&text, &path)?, Some(path)))
    }

    /// [`Config::load`], plus the raw TOML document the typed value was built from.
    ///
    /// The typed value has already forgotten the keys this build does not know — that is what makes
    /// a document written by a newer release keep working — so `config validate`, whose job is to
    /// point at what the user wrote, needs the document itself. Only that command pays the second
    /// copy.
    pub fn load_document(paths: &Paths) -> Result<(Self, Option<(PathBuf, toml::Value)>)> {
        let Some(path) = Self::source(paths)? else {
            return Ok((Self::default(), None));
        };
        let text = fs::read_to_string(&path)
            .map_err(|error| Error::Config(format!("{}: {error}", path.display())))?;
        let document = Self::document(&text, &path)?;
        let config = Self::from_value(document.clone(), &path)?;
        Ok((config, Some((path, document))))
    }

    /// The first configuration file that exists in the XDG search order.
    fn source(paths: &Paths) -> Result<Option<PathBuf>> {
        for candidate in paths.config_file_candidates() {
            match fs::symlink_metadata(&candidate) {
                Ok(_) => return Ok(Some(candidate)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(Error::Config(format!("{}: {error}", candidate.display())));
                }
            }
        }
        Ok(None)
    }

    /// Parses one configuration document.
    fn parse(text: &str, path: &Path) -> Result<Self> {
        Self::from_value(Self::document(text, path)?, path)
    }

    /// Parses and migrates one configuration document into a TOML value.
    fn document(text: &str, path: &Path) -> Result<toml::Value> {
        let mut document: toml::Value =
            toml::from_str(text).map_err(|error| Error::Config(positioned(path, text, &error)))?;

        let version = match document.get("schema_version") {
            None => CURRENT_SCHEMA_VERSION,
            Some(value) => value
                .as_integer()
                .and_then(|version| u32::try_from(version).ok())
                .ok_or_else(|| {
                    Error::Config(format!(
                        "{}: schema_version must be a non-negative integer",
                        path.display()
                    ))
                })?,
        };
        let version = migrate(version, &mut document)?;
        debug_assert_eq!(version, CURRENT_SCHEMA_VERSION);
        Ok(document)
    }

    /// Converts a migrated document into the typed configuration.
    fn from_value(document: toml::Value, path: &Path) -> Result<Self> {
        document
            .try_into()
            .map_err(|error| Error::Config(format!("{}: {error}", path.display())))
    }
}

/// The single place a schema bump transforms an older document.
///
/// Schema `1` is the first released schema, so the only work here is gating: version `0` was never
/// written by any release and a future version cannot be understood by this build.
fn migrate(schema_version: u32, document: &mut toml::Value) -> Result<u32> {
    match schema_version {
        CURRENT_SCHEMA_VERSION => {
            if !document.is_table() {
                return Err(Error::Config(
                    "config root must be a TOML table of keys".to_owned(),
                ));
            }
            Ok(CURRENT_SCHEMA_VERSION)
        }
        0 => Err(Error::Config(
            "config schema_version 0 is not supported (this build writes schema_version 1); \
             run `cirrocast config init --force` to write a fresh document"
                .to_owned(),
        )),
        newer => Err(Error::Config(format!(
            "config written by a newer cirrocast (schema_version {newer}, supported \
             {CURRENT_SCHEMA_VERSION})"
        ))),
    }
}

/// Formats a parse error as `<path>:<line>:<col>: <message>`, falling back to `<path>: <message>`
/// for the error kinds that carry no span.
fn positioned(path: &Path, text: &str, error: &toml::de::Error) -> String {
    match error.span() {
        Some(span) => match text.get(..span.start) {
            Some(prefix) => {
                let line = prefix.matches('\n').count() + 1;
                let column = prefix
                    .rfind('\n')
                    .map_or(prefix.len() + 1, |start| prefix.len() - start);
                format!("{}:{line}:{column}: {}", path.display(), error.message())
            }
            None => format!("{}: {}", path.display(), error.message()),
        },
        None => format!("{}: {}", path.display(), error.message()),
    }
}

// ---------------------------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------------------------

impl Config {
    /// Checks every value against the sets and ranges the contract documents.
    ///
    /// Every failure is an [`Error::Config`] whose message names the dotted key and the offending
    /// value, so `config validate` can point straight at the line to edit.
    pub fn validate(&self) -> Result<()> {
        self.validate_schema_version()?;
        self.validate_defaults()?;
        self.validate_location()?;
        self.validate_units()?;
        self.validate_network()?;
        self.validate_cache()?;
        self.validate_render()?;
        self.validate_alerts()?;
        self.validate_air()?;
        self.validate_providers()
    }

    /// Rejects a run that asks for something the configuration forbids.
    ///
    /// Today that is one combination: `cache.enabled = false` with `--offline` asks the cache to
    /// serve an answer it was told never to store. The message names the key and both ways out.
    pub fn check_offline(&self, offline: bool) -> Result<()> {
        if offline && !self.cache.enabled {
            return Err(Error::Config(
                "cache.enabled = false, so `--offline` could never be served: \
                 set cache.enabled = true or drop `--offline`"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    /// One note per `[units]` override that changes the quantity away from what `defaults.units`
    /// alone would select, spelling out the precedence rule.
    ///
    /// Overriding a single quantity is the documented behaviour, so these are notes, not errors:
    /// the text exists so that a reader who sees both `defaults.units = "us"` and `units.temp = "c"`
    /// is told which key the renderer actually applies.
    pub fn unit_override_notes(&self) -> Result<Vec<String>> {
        let system = UnitSystem::from_str(&self.defaults.units)?;
        let defaults = system.resolve(&UnitOverrides::default())?;
        let resolved = system.resolve(&self.units)?;
        let mut notes = Vec::new();
        for (key, value, matches_system_default, quantity) in [
            (
                "units.temp",
                self.units.temp.as_deref(),
                resolved.temp == defaults.temp,
                "temperature",
            ),
            (
                "units.wind",
                self.units.wind.as_deref(),
                resolved.wind == defaults.wind,
                "wind",
            ),
            (
                "units.pressure",
                self.units.pressure.as_deref(),
                resolved.pressure == defaults.pressure,
                "pressure",
            ),
            (
                "units.distance",
                self.units.distance.as_deref(),
                resolved.distance == defaults.distance,
                "distance",
            ),
            (
                "units.precip",
                self.units.precip.as_deref(),
                resolved.precip == defaults.precip,
                "precipitation",
            ),
        ] {
            if let Some(value) = value
                && !matches_system_default
            {
                notes.push(format!(
                    "note: {key} = \"{value}\" overrides defaults.units = \"{}\" for {quantity}; \
                     the per-quantity key wins",
                    self.defaults.units
                ));
            }
        }
        Ok(notes)
    }

    fn validate_schema_version(&self) -> Result<()> {
        if self.schema_version == CURRENT_SCHEMA_VERSION {
            Ok(())
        } else {
            Err(Error::Config(format!(
                "schema_version: {} is not supported (this build writes {CURRENT_SCHEMA_VERSION})",
                self.schema_version
            )))
        }
    }

    fn validate_defaults(&self) -> Result<()> {
        check_provider_chain("defaults.provider", &self.defaults.provider)?;
        check_enum("defaults.format", &self.defaults.format, FORMATS)?;
        check_enum("defaults.units", &self.defaults.units, UNIT_SYSTEMS)?;
        check_range("defaults.days", u32::from(self.defaults.days), DAYS_RANGE)?;
        check_language("defaults.language", &self.defaults.language)?;
        Ok(())
    }

    /// `location.default` must be a location argument the resolver would accept, checked with the
    /// same parser the command line uses so the two cannot drift.
    fn validate_location(&self) -> Result<()> {
        let text = self.location.default.trim();
        if text.is_empty() {
            return Ok(());
        }
        LocationSpec::parse_arg(Some(text))
            .map(|_| ())
            .map_err(|error| Error::Config(format!("location.default: {error}")))
    }

    fn validate_units(&self) -> Result<()> {
        check_optional_enum("units.temp", self.units.temp.as_deref(), TEMP_UNITS)?;
        check_optional_enum("units.wind", self.units.wind.as_deref(), WIND_UNITS)?;
        check_optional_enum(
            "units.pressure",
            self.units.pressure.as_deref(),
            PRESSURE_UNITS,
        )?;
        check_optional_enum(
            "units.distance",
            self.units.distance.as_deref(),
            DISTANCE_UNITS,
        )?;
        check_optional_enum("units.precip", self.units.precip.as_deref(), PRECIP_UNITS)?;
        Ok(())
    }

    fn validate_network(&self) -> Result<()> {
        check_range(
            "network.timeout_secs",
            self.network.timeout_secs,
            TIMEOUT_RANGE,
        )?;
        check_range("network.retries", self.network.retries, RETRIES_RANGE)?;
        self.validate_proxy()?;
        self.validate_nominatim_url()
    }

    /// `network.proxy` must be empty, an `http(s)` proxy or a bare `host:port`.
    fn validate_proxy(&self) -> Result<()> {
        if !is_proxy_url(&self.network.proxy) {
            return Err(Error::Config(format!(
                "network.proxy: `{}` is not an `http://` or `https://` proxy URL",
                self.network.proxy
            )));
        }
        Ok(())
    }

    /// `network.nominatim_url` must be empty or an http(s) base URL.
    fn validate_nominatim_url(&self) -> Result<()> {
        if !is_service_url(&self.network.nominatim_url) {
            return Err(Error::Config(format!(
                "network.nominatim_url: `{}` is not an http(s) base URL",
                self.network.nominatim_url
            )));
        }
        Ok(())
    }

    fn validate_cache(&self) -> Result<()> {
        check_positive("cache.weather_ttl_secs", self.cache.weather_ttl_secs)?;
        check_positive("cache.ip_ttl_secs", self.cache.ip_ttl_secs)?;
        check_positive("cache.geocode_ttl_secs", self.cache.geocode_ttl_secs)?;
        Ok(())
    }

    fn validate_render(&self) -> Result<()> {
        check_enum("render.color", &self.render.color, COLOR_MODES)?;
        let width = u32::try_from(self.render.width).unwrap_or(u32::MAX);
        if width != 0 && !(WIDTH_RANGE.0..=WIDTH_RANGE.1).contains(&width) {
            return Err(Error::Config(format!(
                "render.width: {width} is not 0 or within {}..={}",
                WIDTH_RANGE.0, WIDTH_RANGE.1
            )));
        }
        Ok(())
    }

    fn validate_providers(&self) -> Result<()> {
        self.validate_metar_station()?;
        self.validate_qweather_host()
    }

    /// Every `[alerts]` value must be usable before a run starts: an unknown source id or a bogus
    /// instance URL is a configuration mistake, and finding it at startup beats finding it after
    /// the forecast was fetched.
    fn validate_alerts(&self) -> Result<()> {
        check_enum(
            "alerts.severity_threshold",
            &self.alerts.severity_threshold,
            SEVERITY_LEVELS,
        )?;
        check_alert_sources("alerts.sources", &self.alerts.sources)?;
        self.validate_fpas_url()?;
        check_positive("alerts.cache_ttl_secs", self.alerts.cache_ttl_secs)
    }

    /// `[air] index` must be one of the two documented scales.
    fn validate_air(&self) -> Result<()> {
        check_enum("air.index", &self.air.index, AQI_INDEXES)
    }

    /// `alerts.fpas_url` must be empty or an http(s) base URL, like `network.nominatim_url`.
    fn validate_fpas_url(&self) -> Result<()> {
        if !is_service_url(&self.alerts.fpas_url) {
            return Err(Error::Config(format!(
                "alerts.fpas_url: `{}` is not an http(s) base URL",
                self.alerts.fpas_url
            )));
        }
        Ok(())
    }

    fn validate_metar_station(&self) -> Result<()> {
        let station = self.providers.metar.station.trim();
        if !station.is_empty() && !crate::provider::metar::is_icao_station(station) {
            return Err(Error::Config(format!(
                "providers.metar.station: `{station}` is not a four-character ICAO station identifier (a letter followed by three letters or digits, e.g. `ZBAA`)"
            )));
        }
        Ok(())
    }

    /// `providers.qweather.host` must be the HTTPS account host the `QWeather` console shows.
    ///
    /// The host is part of the authentication and the key travels in a header, so plain `http` is
    /// refused outright. The legacy shared domains (`api.qweather.com`, …) answer `403 Invalid
    /// Host` for every key, so accepting them here would only move the failure somewhere less
    /// explainable; the documented account shape `<account-id>.re.qweatherapi.com` is required.
    fn validate_qweather_host(&self) -> Result<()> {
        let host = self.providers.qweather.host.trim().trim_end_matches('/');
        if !host.is_empty() && !is_qweather_host(host) {
            return Err(Error::Config(format!(
                "providers.qweather.host: `{host}` must be the HTTPS account host from \
                 https://console.qweather.com/setting, e.g. \
                 `https://<account-id>.re.qweatherapi.com`"
            )));
        }
        Ok(())
    }
}

/// Whether `host` is a bare `https://<account-id>.re.qweatherapi.com` URL (no path).
///
/// Mirrors the provider's own check in `src/provider/qweather.rs`; the two must accept exactly the
/// same spelling so `config set` cannot write a value the backend then refuses.
fn is_qweather_host(host: &str) -> bool {
    let Some(rest) = host.strip_prefix("https://") else {
        return false;
    };
    if rest.contains('/') {
        return false;
    }
    rest.strip_suffix(".re.qweatherapi.com")
        .is_some_and(|account| !account.is_empty())
}

/// `key: value is not one of a, b, c`, with the accepted values quoted for the user.
fn check_enum(key: &str, value: &str, allowed: &[&str]) -> Result<()> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(Error::Config(format!(
            "{key}: `{value}` is not one of {}",
            allowed.join(", ")
        )))
    }
}

/// [`check_enum`] for an optional override: absent means "follow the unit system".
fn check_optional_enum(key: &str, value: Option<&str>, allowed: &[&str]) -> Result<()> {
    match value {
        Some(value) => check_enum(key, value, allowed),
        None => Ok(()),
    }
}

/// `key: value is out of range min..=max`.
fn check_range(key: &str, value: u32, (min, max): (u32, u32)) -> Result<()> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(Error::Config(format!(
            "{key}: {value} is out of range {min}..={max}"
        )))
    }
}

/// `key: must be greater than 0`, for the three cache TTLs.
fn check_positive(key: &str, value: u32) -> Result<()> {
    if value == 0 {
        Err(Error::Config(format!("{key}: must be greater than 0")))
    } else {
        Ok(())
    }
}

/// A provider id chain: `open-meteo`, `auto`, or a comma separated list of known ids.
fn check_provider_chain(key: &str, value: &str) -> Result<()> {
    let chain = value.trim();
    if chain.is_empty() {
        return Err(Error::Config(format!("{key}: the provider chain is empty")));
    }
    for id in chain.split(',').map(str::trim) {
        if id == "auto" {
            continue;
        }
        id.parse::<ProviderId>().map_err(|_| {
            Error::Config(format!(
                "{key}: unknown provider `{id}`; use `auto` or an id from `cirrocast provider list`"
            ))
        })?;
    }
    Ok(())
}

/// `auto` or a list of known alert source ids, for `alerts.sources`.
fn check_alert_sources(key: &str, sources: &[String]) -> Result<()> {
    if sources.is_empty() {
        return Err(Error::Config(format!(
            "{key}: name at least one source, or `auto`"
        )));
    }
    for source in sources {
        let token = source.trim();
        if token.eq_ignore_ascii_case("auto") {
            continue;
        }
        token.parse::<crate::model::AlertSource>().map_err(|_| {
            Error::Config(format!(
                "{key}: unknown alert source `{token}`; use `auto` or ids from: {}",
                crate::model::AlertSource::ALL
                    .map(crate::model::AlertSource::as_str)
                    .join(", ")
            ))
        })?;
    }
    Ok(())
}

/// `auto` or a BCP-47 shaped language tag (`en`, `en-US`, `zh-Hant-CN`).
fn check_language(key: &str, value: &str) -> Result<()> {
    if is_language_tag(value) {
        Ok(())
    } else {
        Err(Error::Config(format!(
            "{key}: `{value}` is not `auto` or a language tag such as `en-US`"
        )))
    }
}

/// Whether `value` is `auto` or shaped like a BCP-47 language tag.
fn is_language_tag(value: &str) -> bool {
    if value == "auto" {
        return true;
    }
    let mut subtags = value.split('-');
    let Some(primary) = subtags.next() else {
        return false;
    };
    if !(2..=3).contains(&primary.len()) || !primary.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    subtags.all(|subtag| {
        (2..=8).contains(&subtag.len()) && subtag.chars().all(|c| c.is_ascii_alphanumeric())
    })
}

/// Whether `value` is an empty proxy setting, `http(s)://host[:port]` or `host:port`.
///
/// Only the HTTP proxy schemes are accepted. `ureq` is built without its `socks-proxy` feature, and
/// a manually configured SOCKS proxy makes it panic at connect time (its `WarnOnNoSocksConnector`
/// treats a hand-written setting as a serious error) — a panic the user cannot blame on their
/// input unless this check rejects it with a typed error first. A SOCKS proxy set through the
/// environment only makes `ureq` warn, so the environment path stays as the library handles it.
fn is_proxy_url(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    if let Some((scheme, rest)) = value.split_once("://") {
        return matches!(scheme, "http" | "https") && !rest.is_empty();
    }
    match value.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && !port.is_empty() => {
            port.chars().all(|c| c.is_ascii_digit())
        }
        _ => false,
    }
}

/// Whether `value` is empty or an `http(s)://host[:port][/path]` base URL.
///
/// Used for the swappable service endpoints (`network.nominatim_url`), which are plain HTTP URLs
/// rather than proxy URLs, and which must never carry a scheme that is not http(s) — the shared
/// client only speaks HTTP.
fn is_service_url(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let Some((scheme, rest)) = value.split_once("://") else {
        return false;
    };
    matches!(scheme, "http" | "https") && !rest.trim_matches('/').is_empty()
}

// ---------------------------------------------------------------------------------------------
// Strict document check (`config validate` only)
// ---------------------------------------------------------------------------------------------

/// The keys the schema defines, per table path.
///
/// The empty path is the document root. A path that is not listed is a value, not a table, and has
/// no allowed children. Kept next to [`KEY_TABLE`], which is the same schema flattened for the
/// `config get`/`set` vocabulary; a test proves the two agree.
fn allowed_keys(table: &str) -> Option<&'static [&'static str]> {
    Some(match table {
        "" => &[
            "schema_version",
            "defaults",
            "location",
            "units",
            "network",
            "cache",
            "render",
            "alerts",
            "air",
            "providers",
        ],
        "defaults" => &["provider", "format", "units", "days", "language"],
        "location" => &["default"],
        "units" => &["temp", "wind", "pressure", "distance", "precip"],
        "network" => &["timeout_secs", "retries", "proxy", "nominatim_url"],
        "cache" => &[
            "enabled",
            "weather_ttl_secs",
            "ip_ttl_secs",
            "geocode_ttl_secs",
        ],
        "render" => &["color", "width"],
        "alerts" => &[
            "enabled",
            "severity_threshold",
            "sources",
            "fpas_url",
            "cache_ttl_secs",
        ],
        "air" => &["index"],
        "providers" => &["metar", "qweather"],
        "providers.metar" => &["station"],
        "providers.qweather" => &["host"],
        _ => return None,
    })
}

/// Rejects the first key the schema does not define, naming its dotted path and the sibling keys
/// the table does accept.
///
/// This is stricter than loading on purpose: a running build ignores unknown keys so a document
/// written by a newer release keeps working, while `config validate` exists to tell the user that
/// the key they typed will never be read — the likely causes being a typo or a key from a version
/// they are not running.
pub fn check_known_keys(document: &toml::Value) -> Result<()> {
    check_table("", document)
}

/// [`check_known_keys`] for one table, recursing into the tables it allows.
fn check_table(path: &str, value: &toml::Value) -> Result<()> {
    let Some(table) = value.as_table() else {
        return Ok(());
    };
    let Some(allowed) = allowed_keys(path) else {
        return Ok(());
    };
    for (key, child) in table {
        let child_path = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        if !allowed.contains(&key.as_str()) {
            let known = allowed.join(", ");
            let scope = if path.is_empty() {
                "known keys".to_owned()
            } else {
                format!("known keys in `{path}`")
            };
            return Err(Error::Config(format!(
                "unknown config key `{child_path}`; {scope}: {known}"
            )));
        }
        check_table(&child_path, child)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------------------------

impl Config {
    /// Writes the initial user configuration to `$XDG_CONFIG_HOME/cirrocast/config.toml`.
    ///
    /// Refuses to overwrite an existing file unless `force` is set. When a configuration is
    /// already in effect from another file (a system document under `$XDG_CONFIG_DIRS`, say), the
    /// new user file is seeded with that effective configuration instead of the commented
    /// defaults: writing the defaults there would silently shadow the system document and change
    /// every later run. With no other source the commented [`DEFAULT_DOCUMENT`] is written as
    /// before. The write is atomic: readers see either the old file or the new one.
    pub fn write_default(paths: &Paths, force: bool) -> Result<PathBuf> {
        let path = paths.config_file.clone();
        if path.exists() {
            if !force {
                return Err(Error::Config(format!(
                    "{} exists; pass --force to overwrite it",
                    path.display()
                )));
            }
            atomic_write(&path, DEFAULT_DOCUMENT.as_bytes(), CONFIG_FILE_MODE)?;
            return Ok(path);
        }
        let document = match Self::source(paths)? {
            Some(source) => {
                let text = fs::read_to_string(&source)
                    .map_err(|error| Error::Config(format!("{}: {error}", source.display())))?;
                let config = Self::parse(&text, &source)?;
                toml::to_string_pretty(&config).map_err(|error| {
                    Error::Config(format!("cannot serialise {}: {error}", source.display()))
                })?
            }
            None => DEFAULT_DOCUMENT.to_owned(),
        };
        atomic_write(&path, document.as_bytes(), CONFIG_FILE_MODE)?;
        Ok(path)
    }

    /// Writes this configuration as canonical TOML (comments are not preserved, unknown keys are
    /// dropped — see the design notes in `docs/plans/02-config-and-state.md`).
    pub fn save(&self, paths: &Paths) -> Result<PathBuf> {
        let path = paths.config_file.clone();
        let document = toml::to_string_pretty(self).map_err(|error| {
            Error::Config(format!("cannot serialise the configuration: {error}"))
        })?;
        atomic_write(&path, document.as_bytes(), CONFIG_FILE_MODE)?;
        Ok(path)
    }
}

/// Writes `data` to `path` by creating a sibling temporary file, syncing it and renaming it over
/// the target, so that a reader never observes a half-written document.
///
/// The temporary file is `.<name>.tmp.<pid>` and is removed again when anything fails. Cache blobs
/// (step 05) and `keys.toml` reuse this helper with their own mode.
pub(crate) fn atomic_write(path: &Path, data: &[u8], mode: u32) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Config(format!("{} has no parent directory", path.display())))?;
    let name = path
        .file_name()
        .ok_or_else(|| Error::Config(format!("{} has no file name", path.display())))?;
    fs::create_dir_all(parent)
        .map_err(|error| Error::Config(format!("{}: {error}", parent.display())))?;

    let temporary = parent.join(format!(
        ".{}.tmp.{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    let mut file = create_exclusive(&temporary, mode)
        .map_err(|error| Error::Config(format!("{}: {error}", temporary.display())))?;

    let written = file.write_all(data).and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| fs::rename(&temporary, path));
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(Error::Config(format!("{}: {error}", temporary.display())));
    }
    Ok(())
}

/// Creates `path` exclusively with `mode`.
///
/// A stale `.<name>.tmp.<pid>` from a crashed run with a recycled pid is cleared once, so the write
/// can always proceed.
#[cfg(unix)]
fn create_exclusive(path: &Path, mode: u32) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    let open = || {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(path)
    };
    match open() {
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(path)?;
            open()
        }
        other => other,
    }
}

/// Non-Unix shim: the call sites are shared, file modes are not a concept there.
#[cfg(not(unix))]
fn create_exclusive(path: &Path, mode: u32) -> std::io::Result<fs::File> {
    let _ = mode;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// The document `cirrocast config init` writes: every key with its default and a comment.
///
/// A unit test keeps it in sync with [`Config::default`].
pub const DEFAULT_DOCUMENT: &str = r#"# cirrocast configuration.
#
# Every key is optional: a missing key falls back to the built-in default shown
# here. `cirrocast config show` prints the effective values, `config get <KEY>`
# reads one and `config set <KEY> <VALUE>` edits this file in place (which
# rewrites it in canonical form, dropping comments). Values given on the command
# line, or through the matching `CIRROCAST_*` variable, win over this file.

schema_version = 1

[defaults]
provider = "open-meteo"  # id, comma separated chain, or "auto" (the keyless chain)
format = "art-table"     # art-table | one-line | plain | json | dumb | alerts | aqi | moon
units = "metric"         # metric | us | uk
days = 3                 # 0..=14; each provider clamps to its own maximum
language = "auto"        # "auto" or a BCP-47 tag such as "en-US", "zh-CN"

[location]
default = ""             # "Beijing", ":Beijing", "@39.9,116.4", "~Tsinghua"; empty = ask for the IP location

[units]
# Per-quantity overrides on top of `defaults.units`. Remove the `#` to pin one
# quantity; an absent key follows the unit system.
# temp = "c"        # c | f
# wind = "kmh"      # kmh | mph | mps | knots
# pressure = "hpa"  # hpa | inhg | mmhg
# distance = "km"   # km | mi
# precip = "mm"     # mm | in

[network]
timeout_secs = 15        # 1..=300
retries = 3              # 0..=10
proxy = ""               # e.g. "http://127.0.0.1:8080"; empty = connect directly
nominatim_url = ""       # Nominatim base URL for `~name` searches; empty = the public OpenStreetMap service

[cache]
enabled = true
weather_ttl_secs = 600       # 10 minutes
ip_ttl_secs = 86400          # 24 hours; a larger value is capped there (ipapi.co's terms)
geocode_ttl_secs = 2592000   # 30 days

[render]
color = "auto"           # auto | always | never
width = 0                # 0 = detect from the terminal, or 40..=500 columns

[alerts]
enabled = true                # fetch warnings automatically when a source covers the location
severity_threshold = "minor"  # unknown | minor | moderate | severe | extreme
sources = ["auto"]            # ["auto"] (coverage-selected) or ids: nws, meteoalarm, qweather,
                              # hko, wmoswic, fpas, visualcrossing
fpas_url = ""                 # FOSS Public Alert Server base URL; empty = https://alerts.kde.org
cache_ttl_secs = 300          # 5 minutes

[air]
index = "us"             # us | european: the AQI scale that drives the panel colour and %q

[providers.metar]
station = ""             # default ICAO identifier, e.g. "ZBAA"

[providers.qweather]
host = ""                # API host from https://console.qweather.com/setting,
                         # e.g. "https://<account-id>.re.qweatherapi.com"
"#;

// ---------------------------------------------------------------------------------------------
// Dotted-key access
// ---------------------------------------------------------------------------------------------

/// The type of a configuration value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// Free-form string.
    Str,
    /// One of a fixed set of lowercase tokens. An empty value passes this check so that an optional
    /// override can clear itself; keys that require a value reject it in their setter.
    Enum(&'static [&'static str]),
    /// Non-negative integer.
    U32,
    /// TOML boolean.
    Bool,
}

impl KeyKind {
    /// Validates a raw `config set` value and returns the trimmed value to hand to the setter.
    pub fn parse(self, key: &str, raw: &str) -> Result<String> {
        let value = raw.trim();
        match self {
            Self::Str => Ok(value.to_owned()),
            Self::Enum(allowed) => {
                if value.is_empty() {
                    Ok(String::new())
                } else {
                    check_enum(key, value, allowed).map(|()| value.to_owned())
                }
            }
            Self::U32 => value.parse::<u32>().map(|_| value.to_owned()).map_err(|_| {
                Error::Config(format!("{key}: `{value}` is not a non-negative integer"))
            }),
            Self::Bool => match value {
                "true" | "false" => Ok(value.to_owned()),
                _ => Err(Error::Config(format!(
                    "{key}: `{value}` is not `true` or `false`"
                ))),
            },
        }
    }
}

/// One row of [`KEY_TABLE`]: a dotted key with its type, its environment override and its
/// description.
#[derive(Debug, Clone, Copy)]
pub struct KeySpec {
    /// The dotted key, spelled exactly as `config get`/`config set` accept it.
    pub name: &'static str,
    /// The value type, used to validate `config set` input before it reaches the setter.
    pub kind: KeyKind,
    /// One-line description, used by `--help`-style listings and the README table.
    pub doc: &'static str,
    /// Environment variable of the `CIRROCAST_*` family that overrides this key, if any.
    pub env: Option<&'static str>,
}

/// Every key `config get`/`config set` understand, in schema order.
///
/// `keys.*` is deliberately absent: API keys live in their own document ([`keys`]) so that
/// `config show`, the `0644` mode of `config.toml` and hand-editing never have to deal with
/// secrets.
pub const KEY_TABLE: &[KeySpec] = &[
    KeySpec {
        name: "schema_version",
        kind: KeyKind::U32,
        doc: "configuration schema version (do not edit)",
        env: None,
    },
    KeySpec {
        name: "defaults.provider",
        kind: KeyKind::Str,
        doc: "provider id, chain, or auto",
        env: Some("CIRROCAST_PROVIDER"),
    },
    KeySpec {
        name: "defaults.format",
        kind: KeyKind::Enum(FORMATS),
        doc: "output format",
        env: Some("CIRROCAST_FORMAT"),
    },
    KeySpec {
        name: "defaults.units",
        kind: KeyKind::Enum(UNIT_SYSTEMS),
        doc: "unit system",
        env: Some("CIRROCAST_UNITS"),
    },
    KeySpec {
        name: "defaults.days",
        kind: KeyKind::U32,
        doc: "forecast days, 0..=14",
        env: Some("CIRROCAST_DAYS"),
    },
    KeySpec {
        name: "defaults.language",
        kind: KeyKind::Str,
        doc: "output language tag or auto",
        env: Some("CIRROCAST_LANG"),
    },
    KeySpec {
        name: "location.default",
        kind: KeyKind::Str,
        doc: "location used when none is given",
        env: Some("CIRROCAST_LOCATION"),
    },
    KeySpec {
        name: "units.temp",
        kind: KeyKind::Enum(TEMP_UNITS),
        doc: "temperature override",
        env: None,
    },
    KeySpec {
        name: "units.wind",
        kind: KeyKind::Enum(WIND_UNITS),
        doc: "wind speed override",
        env: None,
    },
    KeySpec {
        name: "units.pressure",
        kind: KeyKind::Enum(PRESSURE_UNITS),
        doc: "pressure override",
        env: None,
    },
    KeySpec {
        name: "units.distance",
        kind: KeyKind::Enum(DISTANCE_UNITS),
        doc: "distance override",
        env: None,
    },
    KeySpec {
        name: "units.precip",
        kind: KeyKind::Enum(PRECIP_UNITS),
        doc: "precipitation override",
        env: None,
    },
    KeySpec {
        name: "network.timeout_secs",
        kind: KeyKind::U32,
        doc: "request timeout, 1..=300 seconds",
        env: Some("CIRROCAST_TIMEOUT"),
    },
    KeySpec {
        name: "network.retries",
        kind: KeyKind::U32,
        doc: "retry attempts, 0..=10",
        env: None,
    },
    KeySpec {
        name: "network.proxy",
        kind: KeyKind::Str,
        doc: "proxy URL; empty = direct",
        env: None,
    },
    KeySpec {
        name: "network.nominatim_url",
        kind: KeyKind::Str,
        doc: "Nominatim base URL; empty = the public service",
        env: Some("CIRROCAST_NOMINATIM_URL"),
    },
    KeySpec {
        name: "cache.enabled",
        kind: KeyKind::Bool,
        doc: "use the on-disk cache",
        env: None,
    },
    KeySpec {
        name: "cache.weather_ttl_secs",
        kind: KeyKind::U32,
        doc: "weather cache TTL in seconds",
        env: None,
    },
    KeySpec {
        name: "cache.ip_ttl_secs",
        kind: KeyKind::U32,
        doc: "IP location cache TTL in seconds (capped at 24 h)",
        env: None,
    },
    KeySpec {
        name: "cache.geocode_ttl_secs",
        kind: KeyKind::U32,
        doc: "geocoding cache TTL in seconds",
        env: None,
    },
    KeySpec {
        name: "render.color",
        kind: KeyKind::Enum(COLOR_MODES),
        doc: "colour mode",
        env: None,
    },
    KeySpec {
        name: "render.width",
        kind: KeyKind::U32,
        doc: "0 = detect, or 40..=500 columns",
        env: None,
    },
    KeySpec {
        name: "alerts.enabled",
        kind: KeyKind::Bool,
        doc: "fetch alerts automatically",
        env: None,
    },
    KeySpec {
        name: "alerts.severity_threshold",
        kind: KeyKind::Enum(SEVERITY_LEVELS),
        doc: "lowest severity shown",
        env: None,
    },
    KeySpec {
        name: "alerts.sources",
        kind: KeyKind::Str,
        doc: "comma separated source ids or auto",
        env: None,
    },
    KeySpec {
        name: "alerts.fpas_url",
        kind: KeyKind::Str,
        doc: "FOSS Public Alert Server URL; empty = the public instance",
        env: None,
    },
    KeySpec {
        name: "alerts.cache_ttl_secs",
        kind: KeyKind::U32,
        doc: "alert cache TTL in seconds",
        env: None,
    },
    KeySpec {
        name: "air.index",
        kind: KeyKind::Enum(AQI_INDEXES),
        doc: "AQI scale for the panel colour and %q",
        env: None,
    },
    KeySpec {
        name: "providers.metar.station",
        kind: KeyKind::Str,
        doc: "default ICAO station",
        env: None,
    },
    KeySpec {
        name: "providers.qweather.host",
        kind: KeyKind::Str,
        doc: "QWeather API host",
        env: None,
    },
];

/// Looks a dotted key up in [`KEY_TABLE`].
///
/// An unknown key is a usage problem (exit code 2) and the message lists what is available.
pub fn key_spec(key: &str) -> Result<&'static KeySpec> {
    KEY_TABLE
        .iter()
        .find(|spec| spec.name == key)
        .ok_or_else(|| Error::Usage(unknown_key_message(key)))
}

/// `unknown config key `x`; known keys: a, b, c`.
fn unknown_key_message(key: &str) -> String {
    let known = KEY_TABLE
        .iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>()
        .join(", ");
    format!("unknown config key `{key}`; known keys: {known}")
}

impl Config {
    /// Reads one key as text, applying the `CIRROCAST_*` environment override when the key has one.
    ///
    /// Optional unit overrides read as an empty string when they are unset.
    pub fn get_key(&self, key: &str) -> Result<String> {
        let spec = key_spec(key)?;
        if let Some(env) = spec.env
            && let Some(value) = env_value(env)
        {
            return Ok(value);
        }
        Ok(match spec.name {
            "schema_version" => self.schema_version.to_string(),
            "defaults.provider" => self.defaults.provider.clone(),
            "defaults.format" => self.defaults.format.clone(),
            "defaults.units" => self.defaults.units.clone(),
            "defaults.days" => self.defaults.days.to_string(),
            "defaults.language" => self.defaults.language.clone(),
            "location.default" => self.location.default.clone(),
            "units.temp" => self.units.temp.clone().unwrap_or_default(),
            "units.wind" => self.units.wind.clone().unwrap_or_default(),
            "units.pressure" => self.units.pressure.clone().unwrap_or_default(),
            "units.distance" => self.units.distance.clone().unwrap_or_default(),
            "units.precip" => self.units.precip.clone().unwrap_or_default(),
            "network.timeout_secs" => self.network.timeout_secs.to_string(),
            "network.retries" => self.network.retries.to_string(),
            "network.proxy" => self.network.proxy.clone(),
            "network.nominatim_url" => self.network.nominatim_url.clone(),
            "cache.enabled" => self.cache.enabled.to_string(),
            "cache.weather_ttl_secs" => self.cache.weather_ttl_secs.to_string(),
            "cache.ip_ttl_secs" => self.cache.ip_ttl_secs.to_string(),
            "cache.geocode_ttl_secs" => self.cache.geocode_ttl_secs.to_string(),
            "render.color" => self.render.color.clone(),
            "render.width" => self.render.width.to_string(),
            "alerts.enabled" => self.alerts.enabled.to_string(),
            "alerts.severity_threshold" => self.alerts.severity_threshold.clone(),
            "alerts.sources" => self.alerts.sources.join(","),
            "alerts.fpas_url" => self.alerts.fpas_url.clone(),
            "alerts.cache_ttl_secs" => self.alerts.cache_ttl_secs.to_string(),
            "air.index" => self.air.index.clone(),
            "providers.metar.station" => self.providers.metar.station.clone(),
            "providers.qweather.host" => self.providers.qweather.host.clone(),
            _ => return Err(Error::Usage(unknown_key_message(spec.name))),
        })
    }

    /// Writes one key from text and validates the result.
    ///
    /// The caller persists the change with [`Config::save`]; nothing is written here.
    pub fn set_key(&mut self, key: &str, value: &str) -> Result<()> {
        let spec = key_spec(key)?;
        let value = spec.kind.parse(spec.name, value)?;
        match spec.name {
            "schema_version" => self.schema_version = u32_value(spec.name, &value)?,
            "defaults.provider" => {
                check_provider_chain(spec.name, &value)?;
                self.defaults.provider = value;
            }
            "defaults.format" => {
                check_enum(spec.name, &value, FORMATS)?;
                self.defaults.format = value;
            }
            "defaults.units" => {
                check_enum(spec.name, &value, UNIT_SYSTEMS)?;
                self.defaults.units = value;
            }
            "defaults.days" => self.defaults.days = days_value(spec.name, &value)?,
            "defaults.language" => {
                check_language(spec.name, &value)?;
                self.defaults.language = value;
            }
            "location.default" => self.location.default = value,
            "units.temp" => self.units.temp = optional_enum(spec.name, &value, TEMP_UNITS)?,
            "units.wind" => self.units.wind = optional_enum(spec.name, &value, WIND_UNITS)?,
            "units.pressure" => {
                self.units.pressure = optional_enum(spec.name, &value, PRESSURE_UNITS)?;
            }
            "units.distance" => {
                self.units.distance = optional_enum(spec.name, &value, DISTANCE_UNITS)?;
            }
            "units.precip" => self.units.precip = optional_enum(spec.name, &value, PRECIP_UNITS)?,
            "network.timeout_secs" => self.network.timeout_secs = u32_value(spec.name, &value)?,
            "network.retries" => self.network.retries = u32_value(spec.name, &value)?,
            "network.proxy" => self.network.proxy = value,
            "network.nominatim_url" => self.network.nominatim_url = value,
            "cache.enabled" => self.cache.enabled = bool_value(spec.name, &value)?,
            "cache.weather_ttl_secs" => {
                self.cache.weather_ttl_secs = u32_value(spec.name, &value)?;
            }
            "cache.ip_ttl_secs" => self.cache.ip_ttl_secs = u32_value(spec.name, &value)?,
            "cache.geocode_ttl_secs" => {
                self.cache.geocode_ttl_secs = u32_value(spec.name, &value)?;
            }
            "render.color" => {
                check_enum(spec.name, &value, COLOR_MODES)?;
                self.render.color = value;
            }
            "render.width" => self.render.width = width_value(spec.name, &value)?,
            "alerts.enabled" => self.alerts.enabled = bool_value(spec.name, &value)?,
            "alerts.severity_threshold" => {
                check_enum(spec.name, &value, SEVERITY_LEVELS)?;
                self.alerts.severity_threshold = value;
            }
            "alerts.sources" => {
                self.alerts.sources = value
                    .split(',')
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            "alerts.fpas_url" => self.alerts.fpas_url = value,
            "alerts.cache_ttl_secs" => {
                self.alerts.cache_ttl_secs = u32_value(spec.name, &value)?;
            }
            "air.index" => {
                check_enum(spec.name, &value, AQI_INDEXES)?;
                self.air.index = value;
            }
            "providers.metar.station" => self.providers.metar.station = value,
            "providers.qweather.host" => self.providers.qweather.host = value,
            _ => return Err(Error::Usage(unknown_key_message(spec.name))),
        }
        // Only the key that was just written is checked. A whole-document check would refuse the
        // very command that repairs a document that already holds an invalid value elsewhere.
        self.validate_key(spec.name)
    }

    /// Validates only the field `key` names, the counterpart of [`Config::validate`] for
    /// [`Config::set_key`].
    fn validate_key(&self, key: &str) -> Result<()> {
        match key {
            "schema_version" => self.validate_schema_version(),
            "defaults.provider" => check_provider_chain(key, &self.defaults.provider),
            "defaults.format" => check_enum(key, &self.defaults.format, FORMATS),
            "defaults.units" => check_enum(key, &self.defaults.units, UNIT_SYSTEMS),
            "defaults.days" => check_range(key, u32::from(self.defaults.days), DAYS_RANGE),
            "defaults.language" => check_language(key, &self.defaults.language),
            "location.default" => self.validate_location(),
            "units.temp" => check_optional_enum(key, self.units.temp.as_deref(), TEMP_UNITS),
            "units.wind" => check_optional_enum(key, self.units.wind.as_deref(), WIND_UNITS),
            "units.pressure" => {
                check_optional_enum(key, self.units.pressure.as_deref(), PRESSURE_UNITS)
            }
            "units.distance" => {
                check_optional_enum(key, self.units.distance.as_deref(), DISTANCE_UNITS)
            }
            "units.precip" => check_optional_enum(key, self.units.precip.as_deref(), PRECIP_UNITS),
            "network.timeout_secs" => check_range(key, self.network.timeout_secs, TIMEOUT_RANGE),
            "network.retries" => check_range(key, self.network.retries, RETRIES_RANGE),
            "network.proxy" => self.validate_proxy(),
            "network.nominatim_url" => self.validate_nominatim_url(),
            "cache.weather_ttl_secs" => check_positive(key, self.cache.weather_ttl_secs),
            "cache.ip_ttl_secs" => check_positive(key, self.cache.ip_ttl_secs),
            "cache.geocode_ttl_secs" => check_positive(key, self.cache.geocode_ttl_secs),
            "render.color" => check_enum(key, &self.render.color, COLOR_MODES),
            "render.width" => self.validate_render(),
            "alerts.severity_threshold" => {
                check_enum(key, &self.alerts.severity_threshold, SEVERITY_LEVELS)
            }
            "alerts.sources" => check_alert_sources(key, &self.alerts.sources),
            "alerts.fpas_url" => self.validate_fpas_url(),
            "alerts.cache_ttl_secs" => check_positive(key, self.alerts.cache_ttl_secs),
            "air.index" => self.validate_air(),
            "providers.metar.station" => self.validate_metar_station(),
            "providers.qweather.host" => self.validate_qweather_host(),
            _ => Ok(()),
        }
    }

    /// This configuration with every `CIRROCAST_*` environment override applied, the same values
    /// [`Config::get_key`] reports for those keys.
    ///
    /// `config show` promises the *effective* configuration, so it renders this rather than the
    /// file alone; `config get` has always resolved the environment first, and the two must not
    /// disagree about the same key.
    pub fn with_env_overrides(&self) -> Result<Self> {
        let mut config = self.clone();
        for spec in KEY_TABLE {
            if let Some(env) = spec.env
                && let Some(value) = env_value(env)
            {
                config.set_key(spec.name, &value)?;
            }
        }
        Ok(config)
    }
}

fn u32_value(key: &str, value: &str) -> Result<u32> {
    value
        .parse::<u32>()
        .map_err(|_| Error::Config(format!("{key}: `{value}` is not a non-negative integer")))
}

fn bool_value(key: &str, value: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(Error::Config(format!(
            "{key}: `{value}` is not `true` or `false`"
        ))),
    }
}

fn days_value(key: &str, value: &str) -> Result<u8> {
    let days = u32_value(key, value)?;
    check_range(key, days, DAYS_RANGE)?;
    u8::try_from(days).map_err(|_| Error::Config(format!("{key}: {days} is out of range 0..=14")))
}

fn width_value(key: &str, value: &str) -> Result<usize> {
    let width = u32_value(key, value)?;
    if width != 0 && !(WIDTH_RANGE.0..=WIDTH_RANGE.1).contains(&width) {
        return Err(Error::Config(format!(
            "{key}: {width} is not 0 or within {}..={}",
            WIDTH_RANGE.0, WIDTH_RANGE.1
        )));
    }
    Ok(usize::try_from(width).unwrap_or(usize::MAX))
}

fn optional_enum(key: &str, value: &str, allowed: &[&str]) -> Result<Option<String>> {
    if value.is_empty() {
        Ok(None)
    } else {
        check_enum(key, value, allowed)?;
        Ok(Some(value.to_owned()))
    }
}

/// The value of an environment variable, trimmed; an unset or empty variable is `None`.
pub(crate) fn env_value(name: &str) -> Option<String> {
    let value = std::env::var_os(name)?;
    let value = value.to_string_lossy();
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

// ---------------------------------------------------------------------------------------------
// Resolved settings
// ---------------------------------------------------------------------------------------------

/// The inputs a weather run works with, after the precedence rules have been applied:
/// command line flag > `CIRROCAST_*` environment variable > `config.toml` > built-in default.
///
/// The vocabulary fields (`provider`, `format`, `units`, `lang`) stay strings on purpose: the steps
/// that own those types (03 for units, 05 for the cache mode, 06 for formats, 09 for languages)
/// parse them with their own `FromStr`, and this module never has to know them. The stored values
/// are checked by [`Config::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Provider chain or `auto`.
    pub provider: String,
    /// Output format name.
    pub format: String,
    /// Unit system name.
    pub units: String,
    /// Forecast days.
    pub days: u8,
    /// Output language tag or `auto`.
    pub lang: String,
    /// AQI scale for the air panel (`us` or `european`).
    pub aqi_index: String,
    /// Location argument (`location.default`), when one is configured.
    pub location: Option<String>,
    /// Per-request timeout in seconds.
    pub timeout_secs: u32,
    /// Proxy URL, when configured.
    pub proxy: Option<String>,
    /// `--no-cache`.
    pub no_cache: bool,
    /// `--refresh`.
    pub refresh: bool,
    /// `--offline`.
    pub offline: bool,
}

/// The command line overrides applied on top of the configuration file.
///
/// Step 08 fills this from the clap flags, whose `env = "CIRROCAST_*"` attributes already resolve
/// flag-over-environment; until then only tests construct it. Every field is `None`/`false` when
/// the user said nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliOverrides {
    /// `-p/--provider`.
    pub provider: Option<String>,
    /// `-f/--format`.
    pub format: Option<String>,
    /// `-u/--units`.
    pub units: Option<String>,
    /// `-d/--days`.
    pub days: Option<u8>,
    /// `--lang`.
    pub lang: Option<String>,
    /// `--aqi-index`.
    pub aqi_index: Option<String>,
    /// The positional location argument.
    pub location: Option<String>,
    /// `--timeout`.
    pub timeout_secs: Option<u32>,
    /// `--no-cache`.
    pub no_cache: bool,
    /// `--refresh`.
    pub refresh: bool,
    /// `--offline`.
    pub offline: bool,
}

impl Settings {
    /// Resolves the settings for one run.
    ///
    /// The configuration is validated first, so every consumer can rely on the documented value
    /// sets; the overrides themselves are already parsed by clap (step 08) and are taken as given.
    pub fn resolve(config: &Config, cli: &CliOverrides) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            provider: cli
                .provider
                .clone()
                .unwrap_or_else(|| config.defaults.provider.clone()),
            format: cli
                .format
                .clone()
                .unwrap_or_else(|| config.defaults.format.clone()),
            units: cli
                .units
                .clone()
                .unwrap_or_else(|| config.defaults.units.clone()),
            days: cli.days.unwrap_or(config.defaults.days),
            lang: cli
                .lang
                .clone()
                .unwrap_or_else(|| config.defaults.language.clone()),
            aqi_index: cli
                .aqi_index
                .clone()
                .unwrap_or_else(|| config.air.index.clone()),
            location: cli
                .location
                .clone()
                .or_else(|| non_empty(&config.location.default)),
            timeout_secs: cli.timeout_secs.unwrap_or(config.network.timeout_secs),
            proxy: non_empty(&config.network.proxy),
            no_cache: cli.no_cache,
            refresh: cli.refresh,
            offline: cli.offline,
        })
    }
}

/// `Some(trimmed)` unless the value is empty.
fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{CliOverrides, Config, DEFAULT_DOCUMENT, KEY_TABLE, migrate};

    /// Parses a document the way `Config::load` does.
    fn parse(text: &str) -> super::Result<Config> {
        Config::parse(text, Path::new("config.toml"))
    }

    #[test]
    fn built_in_defaults_match_the_contract() {
        let config = Config::default();
        assert_eq!(config.schema_version, 1);
        assert_eq!(config.defaults.provider, "open-meteo");
        assert_eq!(config.defaults.format, "art-table");
        assert_eq!(config.defaults.units, "metric");
        assert_eq!(config.defaults.days, 3);
        assert_eq!(config.defaults.language, "auto");
        assert_eq!(config.location.default, "");
        assert_eq!(config.units, super::UnitOverrides::default());
        assert_eq!(config.network.timeout_secs, 15);
        assert_eq!(config.network.retries, 3);
        assert_eq!(config.network.proxy, "");
        assert!(config.cache.enabled);
        assert_eq!(config.cache.weather_ttl_secs, 600);
        assert_eq!(config.cache.ip_ttl_secs, 86_400);
        assert_eq!(config.cache.geocode_ttl_secs, 2_592_000);
        assert_eq!(config.render.color, "auto");
        assert_eq!(config.render.width, 0);
        assert!(config.alerts.enabled);
        assert_eq!(config.alerts.severity_threshold, "minor");
        assert_eq!(config.alerts.sources, ["auto"]);
        assert_eq!(config.alerts.fpas_url, "");
        assert_eq!(config.alerts.cache_ttl_secs, 300);
        assert_eq!(config.providers.metar.station, "");
        assert_eq!(config.providers.qweather.host, "");
    }

    #[test]
    fn the_severity_level_list_mirrors_the_model() {
        let model: Vec<&str> = crate::model::Severity::ALL
            .into_iter()
            .map(crate::model::Severity::as_str)
            .collect();
        assert_eq!(super::SEVERITY_LEVELS, model.as_slice());
    }

    #[test]
    fn a_missing_key_falls_back_to_its_default_and_unknown_keys_are_ignored() {
        let config = parse("schema_version = 1\n[defaults]\ndays = 7\n[future]\nx = 1\n")
            .expect("a partial document parses");
        assert_eq!(config.defaults.days, 7);
        assert_eq!(config.defaults.provider, "open-meteo");
        assert_eq!(config.network.retries, 3);
    }

    #[test]
    fn the_shipped_default_document_is_the_built_in_default_configuration() {
        let config = parse(DEFAULT_DOCUMENT).expect("the default document parses");
        assert_eq!(config, Config::default());
        config.validate().expect("the default document validates");
    }

    #[test]
    fn migrate_gates_unknown_schema_versions() {
        let mut document: toml::Value =
            toml::from_str("schema_version = 1").expect("a document parses");
        assert_eq!(migrate(1, &mut document).expect("schema 1 is current"), 1);

        let zero = migrate(0, &mut document).expect_err("schema 0 is not supported");
        assert!(zero.to_string().contains("schema_version 0"), "{zero}");

        let newer = migrate(99, &mut document).expect_err("schema 99 is too new");
        assert!(
            newer
                .to_string()
                .contains("config written by a newer cirrocast (schema_version 99, supported 1)"),
            "{newer}"
        );

        let mut not_a_table = toml::Value::Integer(3);
        assert!(migrate(1, &mut not_a_table).is_err());
    }

    #[test]
    fn syntax_errors_carry_the_path_line_and_column() {
        let error = parse("schema_version = 1\n[defaults\nprovider = \"open-meteo\"\n")
            .expect_err("the unclosed table header is a syntax error");
        let message = error.to_string();
        assert!(message.contains("config.toml:"), "{message}");
        assert!(message.contains(":2:"), "{message}");
    }

    #[test]
    fn empty_unit_overrides_behave_like_absent_keys() {
        let mut config =
            parse("[units]\ntemp = \"\"\nwind = \"mph\"\n").expect("the document parses");
        assert_eq!(config.units.temp, None);
        assert_eq!(config.units.wind.as_deref(), Some("mph"));

        config.set_key("units.temp", "f").expect("f is a unit");
        assert_eq!(config.units.temp.as_deref(), Some("f"));
        config.set_key("units.temp", "").expect("empty clears");
        assert_eq!(config.units.temp, None);
        assert_eq!(config.get_key("units.temp").expect("readable"), "");
    }

    #[test]
    fn validation_names_the_offending_key() {
        type Case = (&'static str, fn(&mut Config), &'static str);
        let cases: [Case; 17] = [
            (
                "defaults.days",
                |config| config.defaults.days = 99,
                "defaults.days: 99 is out of range 0..=14",
            ),
            (
                "defaults.format",
                |config| config.defaults.format = "yaml".to_owned(),
                "defaults.format: `yaml` is not one of art-table, one-line, plain, json, dumb, \
                 alerts, aqi, moon",
            ),
            (
                "defaults.units",
                |config| config.defaults.units = "imperial".to_owned(),
                "defaults.units: `imperial` is not one of metric, us, uk",
            ),
            (
                "defaults.provider",
                |config| config.defaults.provider = "nope".to_owned(),
                "defaults.provider: unknown provider `nope`",
            ),
            (
                "defaults.provider",
                |config| config.defaults.provider = String::new(),
                "defaults.provider: the provider chain is empty",
            ),
            (
                "defaults.language",
                |config| config.defaults.language = "english".to_owned(),
                "defaults.language: `english` is not `auto`",
            ),
            (
                "units.temp",
                |config| config.units.temp = Some("k".to_owned()),
                "units.temp: `k` is not one of c, f",
            ),
            (
                "network.timeout_secs",
                |config| config.network.timeout_secs = 0,
                "network.timeout_secs: 0 is out of range 1..=300",
            ),
            (
                "network.retries",
                |config| config.network.retries = 11,
                "network.retries: 11 is out of range 0..=10",
            ),
            (
                "network.proxy",
                |config| config.network.proxy = "http://".to_owned(),
                "network.proxy: `http://` is not an `http://` or `https://` proxy URL",
            ),
            (
                "network.nominatim_url",
                |config| config.network.nominatim_url = "ftp://osm.example".to_owned(),
                "network.nominatim_url: `ftp://osm.example` is not an http(s) base URL",
            ),
            (
                "cache.weather_ttl_secs",
                |config| config.cache.weather_ttl_secs = 0,
                "cache.weather_ttl_secs: must be greater than 0",
            ),
            (
                "cache.ip_ttl_secs",
                |config| config.cache.ip_ttl_secs = 0,
                "cache.ip_ttl_secs: must be greater than 0",
            ),
            (
                "cache.geocode_ttl_secs",
                |config| config.cache.geocode_ttl_secs = 0,
                "cache.geocode_ttl_secs: must be greater than 0",
            ),
            (
                "render.width",
                |config| config.render.width = 12,
                "render.width: 12 is not 0 or within 40..=500",
            ),
            (
                "render.color",
                |config| config.render.color = "sometimes".to_owned(),
                "render.color: `sometimes` is not one of auto, always, never",
            ),
            (
                "providers.metar.station",
                |config| config.providers.metar.station = "ZZ Z".to_owned(),
                "providers.metar.station: `ZZ Z` is not a four-character ICAO station identifier",
            ),
        ];

        for (key, mutate, expected) in cases {
            let mut config = Config::default();
            mutate(&mut config);
            let error = config
                .validate()
                .expect_err(&format!("{key} should not validate"));
            assert!(error.to_string().contains(expected), "{key}: {error}");
        }
    }

    #[test]
    fn every_render_format_is_configurable() {
        // `FORMATS`, the template comment and `render::Format` are three lists of the same thing;
        // this is where they are compared, so a new format cannot land in only one of them — and
        // `defaults.format` accepts everything `-f` and `CIRROCAST_FORMAT` accept.
        use crate::render::Format;

        let spellings: Vec<&str> = Format::ALL.iter().map(|format| format.as_str()).collect();
        assert_eq!(
            super::FORMATS,
            spellings,
            "config::FORMATS drifted from render::Format::ALL"
        );
        for format in Format::ALL {
            let mut config = Config::default();
            config.defaults.format = format.as_str().to_owned();
            config.validate().expect("every render format validates");
            assert!(DEFAULT_DOCUMENT.contains(format.as_str()));
        }
    }

    #[test]
    fn set_key_rejects_a_socks_proxy_and_a_foreign_qweather_host() {
        // Both values are refused by `config set` through the per-key validator, so a user is told
        // before the value is written (a SOCKS URL would otherwise reach ureq's panic path).
        for (key, value, expected) in [
            (
                "network.proxy",
                "socks5://127.0.0.1:1080",
                "network.proxy: `socks5://127.0.0.1:1080` is not an `http://` or `https://` proxy URL",
            ),
            (
                "providers.qweather.host",
                "http://api.qweather.com",
                "providers.qweather.host: `http://api.qweather.com` must be the HTTPS account host",
            ),
        ] {
            let mut config = Config::default();
            let error = config
                .set_key(key, value)
                .expect_err(&format!("{key} should not validate"));
            assert!(error.to_string().contains(expected), "{key}: {error}");
        }
    }

    #[test]
    fn a_fully_populated_configuration_validates() {
        let mut config = Config::default();
        config.defaults.provider = "open-meteo,smhi".to_owned();
        config.defaults.format = "one-line".to_owned();
        config.defaults.units = "uk".to_owned();
        config.defaults.language = "zh-CN".to_owned();
        config.units.temp = Some("f".to_owned());
        config.network.proxy = "http://127.0.0.1:8080".to_owned();
        config.render.width = 80;
        config.render.color = "never".to_owned();
        config.providers.metar.station = "ZBAA".to_owned();
        config.providers.qweather.host = "https://abc123.re.qweatherapi.com".to_owned();
        config.alerts.enabled = false;
        config.alerts.severity_threshold = "severe".to_owned();
        config.alerts.sources = vec!["nws".to_owned(), "fpas".to_owned()];
        config.alerts.fpas_url = "https://alerts.example.org".to_owned();
        config.alerts.cache_ttl_secs = 60;
        config.validate().expect("every value is in range");
    }

    #[test]
    fn alert_settings_reject_unknown_sources_and_bad_urls() {
        let mut config = Config::default();
        config.alerts.sources = vec!["acme".to_owned()];
        let error = config.validate().unwrap_err();
        assert!(
            error.to_string().contains("unknown alert source `acme`"),
            "{error}"
        );

        let mut config = Config::default();
        config.alerts.sources = Vec::new();
        let error = config.validate().unwrap_err();
        assert!(error.to_string().contains("at least one source"), "{error}");

        let mut config = Config::default();
        config.alerts.fpas_url = "ftp://example.org".to_owned();
        let error = config.validate().unwrap_err();
        assert!(error.to_string().contains("alerts.fpas_url"), "{error}");

        let mut config = Config::default();
        config.alerts.severity_threshold = "catastrophic".to_owned();
        let error = config.validate().unwrap_err();
        assert!(error.to_string().contains("severity_threshold"), "{error}");
    }

    #[test]
    fn the_key_table_is_unique_and_unknown_keys_are_usage_errors() {
        let mut names: Vec<&str> = KEY_TABLE.iter().map(|spec| spec.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "two key specs share a name");

        for spec in KEY_TABLE {
            assert_ne!(spec.doc, "", "{} has no description", spec.name);
        }

        let error = Config::default()
            .get_key("keys.openweathermap")
            .expect_err("secrets are not configuration keys");
        assert_eq!(error.exit_code(), 2);
        assert!(
            error
                .to_string()
                .contains("unknown config key `keys.openweathermap`"),
            "{error}"
        );
        assert!(error.to_string().contains("defaults.days"), "{error}");
    }

    #[test]
    fn set_key_repairs_a_document_that_is_invalid_elsewhere() {
        let mut config = Config::default();
        config.defaults.days = 99;
        config.render.width = 12;

        config
            .set_key("render.width", "80")
            .expect("the key being set is valid");
        config
            .set_key("defaults.days", "5")
            .expect("the key being set is valid");

        assert_eq!(config.render.width, 80);
        assert_eq!(config.defaults.days, 5);
    }

    #[test]
    fn atomic_write_replaces_the_target_and_leaves_no_temporary_file() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("nested/config.toml");

        super::atomic_write(&path, b"first", 0o644).expect("the first write succeeds");
        super::atomic_write(&path, b"second", 0o644).expect("the second write succeeds");

        assert_eq!(
            std::fs::read_to_string(&path).expect("the file is readable"),
            "second"
        );
        let names: Vec<String> = std::fs::read_dir(path.parent().expect("parent"))
            .expect("the directory is readable")
            .map(|entry| {
                entry
                    .expect("a directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, ["config.toml"], "a temporary file was left behind");
    }

    #[test]
    fn settings_resolve_applies_the_precedence_order() {
        let mut config = Config::default();
        config.defaults.days = 5;
        config.location.default = "Beijing".to_owned();

        let from_file = super::Settings::resolve(&config, &CliOverrides::default())
            .expect("the configuration is valid");
        assert_eq!(from_file.days, 5);
        assert_eq!(from_file.provider, "open-meteo");
        assert_eq!(from_file.location.as_deref(), Some("Beijing"));
        assert_eq!(from_file.proxy, None);
        assert!(!from_file.no_cache && !from_file.refresh && !from_file.offline);

        let overridden = CliOverrides {
            provider: Some("smhi".to_owned()),
            days: Some(7),
            location: Some("@39.9,116.4".to_owned()),
            timeout_secs: Some(30),
            refresh: true,
            ..CliOverrides::default()
        };
        let resolved =
            super::Settings::resolve(&config, &overridden).expect("the configuration is valid");
        assert_eq!(resolved.provider, "smhi");
        assert_eq!(resolved.days, 7);
        assert_eq!(resolved.location.as_deref(), Some("@39.9,116.4"));
        assert_eq!(resolved.timeout_secs, 30);
        assert!(resolved.refresh && !resolved.no_cache);

        let mut invalid = Config::default();
        invalid.defaults.days = 99;
        assert!(super::Settings::resolve(&invalid, &CliOverrides::default()).is_err());
    }

    #[test]
    fn unknown_keys_are_rejected_with_their_dotted_path() {
        let document = |text: &str| {
            toml::from_str::<toml::Value>(text).expect("the test document parses as TOML")
        };

        // The shipped document is exactly the schema.
        super::check_known_keys(&document(DEFAULT_DOCUMENT))
            .expect("the default document defines no unknown key");

        // A whole unknown table at the root.
        let error = super::check_known_keys(&document("[future]\nx = 1\n"))
            .expect_err("an unknown table is rejected");
        assert!(
            error.to_string().contains("unknown config key `future`"),
            "{error}"
        );
        assert!(error.to_string().contains("schema_version"), "{error}");

        // A typo inside a known table names the table's real keys.
        let error = super::check_known_keys(&document("[defaults]\ndayz = 3\n"))
            .expect_err("a typo inside a known table is rejected");
        assert!(
            error
                .to_string()
                .contains("unknown config key `defaults.dayz`"),
            "{error}"
        );
        assert!(
            error
                .to_string()
                .contains("known keys in `defaults`: provider, format, units, days, language"),
            "{error}"
        );

        // A value where a table is expected is a type error at load time, not an unknown key.
        super::check_known_keys(&document("defaults = 3\n"))
            .expect("a scalar where a table belongs is left to serde");
    }

    #[test]
    fn the_strict_schema_and_the_dotted_key_table_agree() {
        // Every documented dotted key is reachable through `allowed_keys`, and every allowed key
        // has a row in `KEY_TABLE`: the two halves of the same schema cannot drift apart.
        let mut table_paths: Vec<String> =
            KEY_TABLE.iter().map(|spec| spec.name.to_owned()).collect();
        table_paths.sort();

        let mut schema_paths = Vec::new();
        collect_schema_paths("", &mut schema_paths);
        schema_paths.sort();

        assert_eq!(schema_paths, table_paths);
    }

    /// Every leaf key the strict schema defines, as dotted paths.
    fn collect_schema_paths(table: &str, paths: &mut Vec<String>) {
        for key in super::allowed_keys(table).unwrap_or_default() {
            let path = if table.is_empty() {
                (*key).to_owned()
            } else {
                format!("{table}.{key}")
            };
            if super::allowed_keys(&path).is_some() {
                collect_schema_paths(&path, paths);
            } else {
                paths.push(path);
            }
        }
    }

    #[test]
    fn location_default_must_parse_as_a_location_argument() {
        let mut config = Config::default();
        for accepted in ["", "Beijing", ":Beijing", "~Tsinghua", "@39.9042,116.4074"] {
            config.location.default = accepted.to_owned();
            config
                .validate()
                .unwrap_or_else(|error| panic!("`{accepted}` should validate: {error}"));
        }

        for rejected in [":", "~", "@not,numbers", "a"] {
            config.location.default = rejected.to_owned();
            let error = config
                .validate()
                .expect_err(&format!("`{rejected}` should be rejected"));
            assert_eq!(error.exit_code(), 4);
            assert!(
                error.to_string().contains("location.default:"),
                "{rejected}: {error}"
            );
            assert!(
                error.to_string().contains("accepted forms:"),
                "{rejected}: {error}"
            );
        }
    }

    #[test]
    fn the_metar_station_must_be_an_icao_identifier() {
        let mut config = Config::default();
        for accepted in ["", "ZBAA", "kjfk"] {
            config.providers.metar.station = accepted.to_owned();
            config
                .validate()
                .unwrap_or_else(|error| panic!("`{accepted}` should validate: {error}"));
        }
        for rejected in ["ZB", "ZBA", "ZBAAA", "1BAA", "ZB A"] {
            config.providers.metar.station = rejected.to_owned();
            let error = config
                .validate()
                .expect_err(&format!("`{rejected}` should be rejected"));
            assert_eq!(error.exit_code(), 4);
            assert!(
                error.to_string().contains("providers.metar.station: "),
                "{rejected}: {error}"
            );
        }
    }

    #[test]
    fn offline_needs_an_enabled_cache() {
        let mut config = Config::default();
        config
            .check_offline(true)
            .expect("an enabled cache serves offline");

        config.cache.enabled = false;
        config.check_offline(false).expect("no flags, no problem");
        let error = config
            .check_offline(true)
            .expect_err("--offline and cache.enabled = false cannot be served");
        assert_eq!(error.exit_code(), 4);
        assert!(
            error.to_string().contains("cache.enabled = false"),
            "{error}"
        );
        assert!(error.to_string().contains("drop `--offline`"), "{error}");
    }

    #[test]
    fn unit_overrides_that_conflict_with_the_system_say_which_key_wins() {
        let mut config = Config::default();
        config.defaults.units = "us".to_owned();
        assert_eq!(
            config
                .unit_override_notes()
                .expect("no overrides, no notes"),
            Vec::<String>::new()
        );

        // Agreeing with the system is not a conflict.
        config.units.temp = Some("f".to_owned());
        assert_eq!(
            config.unit_override_notes().expect("agreement is silent"),
            Vec::<String>::new()
        );

        // Disagreeing is: the note names the key, the value and the rule.
        config.units.temp = Some("c".to_owned());
        let notes = config.unit_override_notes().expect("notes are built");
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("units.temp = \"c\""), "{}", notes[0]);
        assert!(
            notes[0].contains("overrides defaults.units = \"us\""),
            "{}",
            notes[0]
        );
        assert!(
            notes[0].contains("the per-quantity key wins"),
            "{}",
            notes[0]
        );
    }
}
