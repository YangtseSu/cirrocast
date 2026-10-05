// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared helpers for the CLI integration tests: a throwaway XDG sandbox, and the offline
//! provider harness the fixture-driven tests use.

// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use assert_cmd::Command;
use chrono::{TimeZone as _, Utc};
use chrono_tz::Tz;

use cirrocast::cache::{Cache, CacheKey, CacheMode, FakeClock, SystemClock};
use cirrocast::config::Config;
use cirrocast::config::keys::KeyStore;
use cirrocast::http::{HttpClient, StubReply, StubTransport};
use cirrocast::model::{Location, LocationSource, Report};
use cirrocast::paths::Paths;
use cirrocast::provider::open_meteo::OpenMeteo;
use cirrocast::provider::{Env, FetchRequest, HourlyResolution, Provider};

/// Every `CIRROCAST_*` override variable, cleared for the child process so that the developer's
/// shell cannot influence a test.
const OVERRIDE_VARS: [&str; 10] = [
    "CIRROCAST_PROVIDER",
    "CIRROCAST_FORMAT",
    "CIRROCAST_UNITS",
    "CIRROCAST_DAYS",
    "CIRROCAST_LANG",
    "CIRROCAST_LOCATION",
    "CIRROCAST_LOCATION_PICK",
    "CIRROCAST_TIMEOUT",
    "CIRROCAST_NOMINATIM_URL",
    "CIRROCAST_IP_SERVICE",
];

/// A throwaway XDG environment: config, cache and data all point into a temporary directory, the
/// system wide config path points at an empty sibling, and `HOME` points at the tree root as well
/// so a stray write to `~/.config`, `~/.cache` or `~/.local/share` cannot escape the sandbox.
pub struct Sandbox {
    home: tempfile::TempDir,
}

impl Sandbox {
    /// Creates an empty sandbox.
    pub fn new() -> Self {
        let home = tempfile::tempdir().expect("a temporary directory");
        fs::create_dir_all(home.path().join("system")).expect("the system config directory");
        Self { home }
    }

    /// The sandbox root.
    pub fn home(&self) -> &Path {
        self.home.path()
    }

    /// `$XDG_CONFIG_HOME/cirrocast`.
    pub fn config_dir(&self) -> PathBuf {
        self.home.path().join("config/cirrocast")
    }

    /// The user configuration file.
    pub fn config_file(&self) -> PathBuf {
        self.config_dir().join("config.toml")
    }

    /// The API key file.
    pub fn keys_file(&self) -> PathBuf {
        self.config_dir().join("keys.toml")
    }

    /// `$XDG_CACHE_HOME/cirrocast`, where the cache entries live.
    pub fn cache_dir(&self) -> PathBuf {
        self.home.path().join("cache/cirrocast")
    }

    /// A `cirrocast` invocation wired to the sandbox and to a clean environment.
    ///
    /// The locale variables are cleared and `LC_ALL` is pinned to `C.UTF-8`: the ambient locale is
    /// what `--lang auto` negotiates from, so inheriting the developer's would make the output
    /// language depend on whose machine ran the test. `C.UTF-8` names no language — the run stays
    /// English — while still telling the renderer the terminal can draw UTF-8. A test that wants a
    /// locale sets `LANG`/`LC_ALL` itself, after this helper.
    pub fn cirrocast(&self) -> Command {
        let mut command = Command::cargo_bin("cirrocast").expect("the binary is built by cargo");
        command
            .env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("config"))
            .env("XDG_CONFIG_DIRS", self.home.path().join("system"))
            .env("XDG_CACHE_HOME", self.home.path().join("cache"))
            .env("XDG_DATA_HOME", self.home.path().join("data"))
            .env("LC_ALL", "C.UTF-8");
        for name in OVERRIDE_VARS {
            command.env_remove(name);
        }
        for name in ["LANG", "LC_MESSAGES", "LC_CTYPE", "CIRROCAST_LANG"] {
            command.env_remove(name);
        }
        // Every CLI test runs with the network guard on, exactly as CI runs them: a test that
        // reaches a socket fails loudly instead of silently depending on the developer's network.
        // Tests that exercise the guard's own message set the variable themselves; a run with a
        // warm cache never needs the network, and the best-effort alert sources degrade to
        // `--verbose` notes when blocked.
        command.env("CIRROCAST_FORBID_NETWORK", "1");
        command
    }

    /// Writes `text` as the user configuration file.
    pub fn write_config(&self, text: &str) {
        fs::create_dir_all(self.config_dir()).expect("the config directory");
        fs::write(self.config_file(), text).expect("the config file is written");
    }

    /// Copies `tests/fixtures/config/<name>` into place as `config.toml`.
    pub fn install_fixture(&self, name: &str) {
        self.write_config(&fixture(name));
    }
}

/// The contents of `tests/fixtures/<name>`.
pub fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The canonical report recorded in `tests/fixtures/report/<file>`.
///
/// These are hand-written `Report` documents, not upstream payloads: no test that renders one can
/// reach the network, and the same fixture renders to the same bytes on any machine.
pub fn fixture_report(file: &str) -> Report {
    let path = fixture_path(&format!("report/{file}"));
    let text =
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The instant a fixture is rendered at: its own observation time, else noon of its first day.
///
/// Taken from the report rather than from the clock, so a snapshot cannot drift — and so that the
/// `Today, Sep 30` heading of the first day is decided by the fixture, not by the test runner's
/// calendar.
pub fn fixture_now(report: &Report) -> chrono::DateTime<chrono::FixedOffset> {
    if let Some(current) = &report.current {
        return current.observed_at;
    }
    let date = report.days.first().map_or_else(
        || chrono::NaiveDate::from_ymd_opt(2026, 9, 30).expect("a valid date"),
        |day| day.date,
    );
    let noon = date.and_hms_opt(12, 0, 0).expect("noon is a valid time");
    cirrocast::model::resolve_local(report.location.tz, noon)
        .expect("the fixture's time zone resolves noon")
        .fixed_offset()
}

/// The render clock for `report`: [`fixture_now`] at the location's zone, as the CLI derives it.
///
/// A context borrows the value (`times: &times`), so a test builds it once beside the context.
pub fn fixture_times(report: &Report) -> cirrocast::model::LocalTimes {
    cirrocast::model::LocalTimes::new(fixture_now(report), report.location.tz)
}

/// Asserts that `directory` holds no leftover `.<file>.tmp.<pid>` from the atomic writer.
pub fn assert_no_temporary_files(directory: &Path) {
    let leftovers: Vec<String> = fs::read_dir(directory)
        .expect("the directory is readable")
        .map(|entry| {
            entry
                .expect("a directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.contains(".tmp."))
        .collect();
    assert_eq!(leftovers, Vec::<String>::new(), "temporary files left");
}

// ---------------------------------------------------------------------------------------------
// Weather-cache seeding for the CLI tests
// ---------------------------------------------------------------------------------------------

/// The calendar dates a weather cache key could carry around `now` in `zone`.
///
/// The binary asks its own clock for the location-local date; a test that seeds the cache before
/// the process starts cannot read that clock, and a run that crosses local midnight between the
/// write and the child would otherwise miss the entry it just wrote — failing with a message that
/// reads like a product bug. Seeding yesterday, today and tomorrow in the same zone the binary
/// uses makes the seed/execute pair immune in both directions.
pub fn weather_key_dates(zone: Tz) -> [chrono::NaiveDate; 3] {
    let today = Utc::now().with_timezone(&zone).date_naive();
    [
        today - chrono::Days::new(1),
        today,
        today + chrono::Days::new(1),
    ]
}

/// Seeds one cache entry per date in [`weather_key_dates`], built by `key_for`.
fn seed_weather_keys(
    sandbox: &Sandbox,
    zone: Tz,
    mut key_for: impl FnMut(chrono::NaiveDate) -> CacheKey,
    body: &str,
) {
    let cache = Cache::with_root(
        sandbox.cache_dir(),
        CacheMode::Normal,
        Arc::new(SystemClock),
        0,
    );
    for date in weather_key_dates(zone) {
        let key = key_for(date);
        cache
            .write(&key, 200, body, Duration::from_secs(600))
            .expect("the cache entry is written");
    }
}

/// Seeds `body` as the weather cache entry for `provider`/`lat`/`lon`/`days`, dated in `zone`.
///
/// One entry is written per date in [`weather_key_dates`], so whichever local date the child
/// computes finds a fresh entry. `zone` is the location's zone *as the binary sees it when it
/// builds the key*: [`Tz::UTC`] for a coordinate query, whose zone stays provisional until the
/// answer arrives, and the resolved zone for a geocoded name.
pub fn seed_weather(
    sandbox: &Sandbox,
    provider: &str,
    lat: f64,
    lon: f64,
    days: u8,
    zone: Tz,
    body: &str,
) {
    seed_weather_keys(
        sandbox,
        zone,
        |date| CacheKey::weather(provider, lat, lon, days, date),
        body,
    );
}

/// [`seed_weather`] for a provider that splits one fetch across resource keys
/// (`openweathermap`'s `current` and `forecast`, `qweather`'s `current` and `hourly`).
// A seeding helper mirrors the key constructor's arguments; a builder struct would obscure it.
#[allow(clippy::too_many_arguments)]
pub fn seed_weather_part(
    sandbox: &Sandbox,
    provider: &str,
    part: &str,
    lat: f64,
    lon: f64,
    days: u8,
    zone: Tz,
    body: &str,
) {
    seed_weather_keys(
        sandbox,
        zone,
        |date| CacheKey::weather_part(provider, part, lat, lon, days, date),
        body,
    );
}

// ---------------------------------------------------------------------------------------------
// The offline provider harness
// ---------------------------------------------------------------------------------------------

/// The recorded response body of `tests/fixtures/open_meteo/<file>` as a `200` reply.
pub fn provider_fixture(file: &str) -> StubReply {
    StubReply::json_file(fixture_path(&format!("open_meteo/{file}")))
        .expect("the fixture is readable")
}

/// The recorded response body of `tests/fixtures/<directory>/<file>` as a `200` reply.
pub fn fixture_reply(directory: &str, file: &str) -> StubReply {
    StubReply::json_file(fixture_path(&format!("{directory}/{file}")))
        .expect("the fixture is readable")
}

/// `tests/fixtures/<name>`, absolute.
pub fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// A clock fixed at 06:00 UTC on the given date.
///
/// The time of day is what makes the `local_today` of the cache key land on the fixture's first
/// day for the zones the fixtures were recorded in.
pub fn provider_clock(year: i32, month: u32, day: u32) -> Arc<FakeClock> {
    let start = Utc
        .with_ymd_and_hms(year, month, day, 6, 0, 0)
        .single()
        .expect("a valid instant");
    Arc::new(FakeClock::new(SystemTime::from(start)))
}

/// The location one fixture was recorded for.
pub fn fixture_location(name: &str) -> Location {
    let (display, lat, lon, tz, admin1) = match name {
        "beijing" => (
            "Beijing",
            39.9042,
            116.4074,
            Tz::Asia__Shanghai,
            Some("Beijing".to_owned()),
        ),
        "longyearbyen" => (
            "Longyearbyen",
            78.2232,
            15.6469,
            Tz::Arctic__Longyearbyen,
            None,
        ),
        "berlin" => ("Berlin", 52.52, 13.405, Tz::Europe__Berlin, None),
        "lisbon" => ("Lisbon", 38.7223, -9.1393, Tz::Europe__Lisbon, None),
        "stockholm" => ("Stockholm", 59.33, 18.06, Tz::Europe__Stockholm, None),
        other => panic!("no fixture for {other}"),
    };
    Location {
        name: display.to_owned(),
        admin1,
        country: String::new(),
        country_code: None,
        lat,
        lon,
        tz,
        elevation_m: None,
        population: None,
        source: LocationSource::Geocoder,
        station: None,
    }
}

/// One fetch run: a scripted transport, a throwaway cache and the key store, with the transport
/// handle the assertions read.
pub struct ProviderRun {
    _directory: tempfile::TempDir,
    http: HttpClient,
    cache: Cache,
    config: Config,
    keys: KeyStore,
    transport: Arc<StubTransport>,
    clock: Arc<FakeClock>,
}

impl ProviderRun {
    /// A run over `replies` in `mode`, at `clock`.
    pub fn new(replies: Vec<StubReply>, clock: Arc<FakeClock>, mode: CacheMode) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let paths = Paths {
            config_dir: directory.path().join("config"),
            config_file: directory.path().join("config/config.toml"),
            keys_file: directory.path().join("config/keys.toml"),
            cache_dir: directory.path().join("cache"),
            data_dir: directory.path().join("data"),
        };
        let transport = Arc::new(StubTransport::new(replies));
        let http = HttpClient::new(Box::new(Arc::clone(&transport)), 0, clock.clone(), 0);
        let cache = Cache::with_root(directory.path().join("cache"), mode, clock.clone(), 0);
        Self {
            _directory: directory,
            http,
            cache,
            config: Config::default(),
            keys: KeyStore::new(&paths),
            transport,
            clock,
        }
    }

    /// A run with one scripted fixture reply, at the day the fixture starts on.
    pub fn fixture(file: &str, day: (i32, u32, u32), mode: CacheMode) -> Self {
        Self::new(
            vec![provider_fixture(file)],
            provider_clock(day.0, day.1, day.2),
            mode,
        )
    }

    /// A run with a caller-provided configuration, for the backends that read one (`QWeather`'s
    /// account host).
    pub fn with_config(
        replies: Vec<StubReply>,
        clock: Arc<FakeClock>,
        mode: CacheMode,
        config: Config,
    ) -> Self {
        let mut run = Self::new(replies, clock, mode);
        run.config = config;
        run
    }

    /// The environment a provider receives.
    pub fn env(&self) -> Env<'_> {
        Env {
            http: &self.http,
            cache: &self.cache,
            config: &self.config,
            keys: &self.keys,
            quiet: true,
            verbose: 0,
        }
    }

    /// Every request the transport has seen.
    pub fn calls(&self) -> Vec<cirrocast::http::HttpRequest> {
        self.transport.calls()
    }

    /// The cache this run writes into.
    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    /// Stores an API key for the run, exactly as `cirrocast key set` does (`0600`, `[keys]` table).
    pub fn with_key(&self, provider: &str, value: &str) {
        self.keys.set(provider, value).expect("the key is stored");
    }

    /// The clock this run uses, for `advance`.
    pub fn clock(&self) -> &Arc<FakeClock> {
        &self.clock
    }

    /// Fetches from Open-Meteo through the scripted transport.
    pub fn fetch(&self, loc: &Location, days: u8) -> cirrocast::error::Result<Report> {
        OpenMeteo.fetch(
            loc,
            &FetchRequest::new(days, HourlyResolution::Hourly),
            &self.env(),
        )
    }

    /// Fetches from any provider through the scripted transport.
    pub fn fetch_with(
        &self,
        provider: &dyn Provider,
        loc: &Location,
        days: u8,
    ) -> cirrocast::error::Result<Report> {
        provider.fetch(
            loc,
            &FetchRequest::new(days, HourlyResolution::Hourly),
            &self.env(),
        )
    }
}
