// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Live smoke tests: the only tests in this repository that may open a socket.
//!
//! They are `#[ignore]`d and additionally gated on `CIRROCAST_LIVE_TESTS=1`, so neither the default
//! `cargo test` nor a stray `cargo test -- --ignored` reaches the network by accident. Run them by
//! hand when the upstream schemas may have moved:
//!
//! ```sh
//! CIRROCAST_LIVE_TESTS=1 cargo test --test live -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use chrono_tz::Tz;
use cirrocast::cache::{Cache, CacheMode, SystemClock};
use cirrocast::config::keys::KeyStore;
use cirrocast::config::{Config, Network};
use cirrocast::error::Result;
use cirrocast::geo::open_meteo::OpenMeteoGeocoder;
use cirrocast::geo::{Geocoder, LocationSpec, resolve};
use cirrocast::http::{HttpClient, UreqTransport};
use cirrocast::i18n::{I18n, LanguageRequest};
use cirrocast::model::units::UnitSystem;
use cirrocast::model::{Location, LocationSource, Report};
use cirrocast::paths::Paths;
use cirrocast::provider::{Env, FetchRequest, HourlyResolution, fetch_chain, select};
use cirrocast::render::{ColorMode, Format, RenderContext, TermCaps, renderer_for};

/// The English catalog, loaded the way the CLI loads an unconfigured run.
fn english() -> I18n {
    I18n::load(&LanguageRequest::Auto, |_| None)
}

/// Whether the live tests are enabled on this machine.
fn enabled() -> bool {
    if std::env::var("CIRROCAST_LIVE_TESTS").is_ok() {
        return true;
    }
    eprintln!("skipping: set CIRROCAST_LIVE_TESTS=1 to run the live smoke tests");
    false
}

/// A real HTTP client over a throwaway cache, plus everything the providers need.
struct Live {
    _directory: tempfile::TempDir,
    http: HttpClient,
    cache: Cache,
    config: Config,
    keys: KeyStore,
}

impl Live {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let paths = Paths {
            config_dir: directory.path().join("config"),
            config_file: directory.path().join("config/config.toml"),
            keys_file: directory.path().join("config/keys.toml"),
            cache_dir: directory.path().join("cache"),
            data_dir: directory.path().join("data"),
        };
        let clock: Arc<dyn cirrocast::cache::Clock> = Arc::new(SystemClock);
        let transport = UreqTransport::new(&Network::default(), Duration::from_secs(30))
            .expect("the transport builds");
        Self {
            http: HttpClient::new(Box::new(transport), 3, Arc::clone(&clock), 0),
            cache: Cache::with_root(directory.path().join("cache"), CacheMode::NoCache, clock, 0),
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
            verbose: 1,
        }
    }

    fn fetch(&self, location: &Location) -> Result<Report> {
        let ids = select("auto")?;
        let request = FetchRequest::new(3, HourlyResolution::Hourly);
        fetch_chain(&ids, location, &request, &self.env())
    }

    /// The report as the CLI would print it.
    fn render(&self, report: &Report) -> String {
        let i18n = english();
        let times = cirrocast::model::LocalTimes::new(Utc::now(), report.location.tz);
        let ctx = RenderContext {
            units: UnitSystem::Metric
                .resolve(&self.config.units)
                .expect("the default units resolve"),
            color: ColorMode::Never,
            width: 100,
            term: TermCaps::default(),
            lang: i18n.lang(),
            i18n: &i18n,
            alert_credits: &[],
            aqi_index: cirrocast::air::aqi::AqiIndex::Us,
            times: times.clone(),
        };
        renderer_for(Format::Plain, &ctx.term, None)
            .expect("plain exists")
            .render(report, &ctx)
            .expect("the report renders")
    }
}

fn beijing() -> Location {
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
        named_by: None,
    }
}

#[test]
#[ignore = "live network: set CIRROCAST_LIVE_TESTS=1 and run `cargo test --test live -- --ignored --nocapture`"]
fn live_open_meteo_beijing() {
    if !enabled() {
        return;
    }

    let live = Live::new();
    let report = live.fetch(&beijing()).expect("Open-Meteo answers");

    assert_eq!(report.attribution.provider, "open-meteo");
    assert_eq!(report.location.tz, Tz::Asia__Shanghai);
    let current = report.current.as_ref().expect("current conditions");
    assert!((-60.0..=60.0).contains(&current.temp_c), "{current:?}");
    assert_eq!(report.days.len(), 3);
    for day in &report.days {
        assert!(day.temp_min_c <= day.temp_max_c, "{day:?}");
    }

    let text = live.render(&report);
    println!("{text}");
    assert!(
        text.contains("Data: Open-Meteo.com (CC BY 4.0) — https://open-meteo.com/"),
        "{text}"
    );
}

#[test]
#[ignore = "live network: set CIRROCAST_LIVE_TESTS=1 and run `cargo test --test live -- --ignored --nocapture`"]
fn live_geocode_plus_forecast_at_coordinates() {
    if !enabled() {
        return;
    }

    let live = Live::new();
    let spec = LocationSpec::parse_arg(Some("Beijing")).expect("a fuzzy name parses");
    let geocoder = OpenMeteoGeocoder::new(&live.http, &live.cache, Duration::from_secs(600));
    let hits = geocoder
        .search("Beijing", 10)
        .expect("the geocoder answers");
    let (location, _resolution) = resolve(hits, &spec, 10).expect("Beijing resolves");

    let report = live.fetch(&location).expect("Open-Meteo answers");
    let text = live.render(&report);
    println!("{text}");
    assert!(text.contains("Beijing"), "{text}");
}

#[test]
#[ignore = "live network: set CIRROCAST_LIVE_TESTS=1 and run `cargo test --test live -- --ignored --nocapture`"]
fn live_metar_station() {
    if !enabled() {
        return;
    }

    let live = Live::new();
    let location = cirrocast::provider::metar::placeholder_location("KJFK");
    let report = cirrocast::provider::provider_for(cirrocast::provider::ProviderId::Metar)
        .expect("metar has a backend")
        .fetch(
            &location,
            &FetchRequest::new(0, HourlyResolution::Hourly),
            &live.env(),
        )
        .expect("aviationweather.gov answers");

    assert_eq!(report.attribution.provider, "metar");
    assert_eq!(
        report.days,
        Vec::<cirrocast::model::DayForecast>::new(),
        "an observation has no forecast"
    );
    assert_eq!(report.location.station.as_deref(), Some("KJFK"));
    assert_eq!(report.location.tz, Tz::America__New_York);
    let current = report.current.as_ref().expect("current conditions");
    assert!((-60.0..=60.0).contains(&current.temp_c), "{current:?}");
    assert!(current.visibility_km.is_some(), "{current:?}");

    let text = live.render(&report);
    println!("{text}");
    assert!(
        text.contains("aviationweather.gov (NOAA/NWS, public domain)"),
        "{text}"
    );
}

#[test]
#[ignore = "live network: set CIRROCAST_LIVE_TESTS=1 and run `cargo test --test live -- --ignored --nocapture`"]
fn live_qweather_jwt() {
    if !enabled() {
        return;
    }

    // The credential comes from the environment quartet (`CIRROCAST_QWEATHER_JWT_*`), which is the
    // way a live run configures one without touching the developer's real `keys.toml`. The account
    // host has no environment override of its own, so this harness names its own variable for it.
    let host = std::env::var("CIRROCAST_QWEATHER_HOST").unwrap_or_default();
    if host.trim().is_empty() {
        eprintln!(
            "skipping: set CIRROCAST_QWEATHER_HOST and the CIRROCAST_QWEATHER_JWT_* quartet to run this test"
        );
        return;
    }

    let mut live = Live::new();
    live.config.providers.qweather.host = host.trim().to_owned();
    let credential = live
        .keys
        .credential(cirrocast::provider::ProviderId::QWeather)
        .expect("the store reads the quartet")
        .expect("the quartet is configured");
    let cirrocast::config::keys::Credential::QWeatherJwt(jwt) = &credential else {
        eprintln!("skipping: this test needs the JWT quartet, not an API key");
        return;
    };
    // Minted here too, so the test can prove the token reaches neither the report nor the output.
    let token = cirrocast::auth::jwt::qweather_token(jwt, std::time::SystemTime::now())
        .expect("the configured key signs");

    let report = cirrocast::provider::provider_for(cirrocast::provider::ProviderId::QWeather)
        .expect("qweather has a backend")
        .fetch(
            &beijing(),
            &FetchRequest::new(3, HourlyResolution::Hourly),
            &live.env(),
        )
        .expect("QWeather answers a JWT-authenticated request");

    assert_eq!(report.attribution.provider, "qweather");
    let current = report.current.as_ref().expect("current conditions");
    assert!((-60.0..=60.0).contains(&current.temp_c), "{current:?}");
    assert!(!report.days.is_empty(), "the v1 hourly series covers days");

    let text = live.render(&report);
    println!("{text}");
    assert!(
        !text.contains(&token),
        "the token must never be echoed: {text}"
    );
}

#[test]
#[ignore = "live network: set CIRROCAST_LIVE_TESTS=1 and run `cargo test --test live -- --ignored --nocapture`"]
fn live_climate_normals_for_beijing() {
    if !enabled() {
        return;
    }

    let live = Live::new();
    // October is a month every long-record station covers, so the assertions can be exact about
    // what a normal is rather than about which month the run happens in.
    let normal = cirrocast::normals::fetch(&beijing(), 10, &live.env())
        .expect("NCEI answers")
        .expect("Beijing has a GSOM station inside the default 60 km");
    assert!(!normal.station.is_empty(), "{normal:?}");
    assert!(normal.distance_km <= 60.0, "{normal:?}");
    assert!(normal.years >= 20, "{normal:?}");
    assert!((-60.0..=60.0).contains(&normal.temp_max_c), "{normal:?}");
    assert!(
        normal.temp_min_c <= normal.temp_max_c,
        "the mean low sits below the mean high: {normal:?}"
    );
    assert!(normal.precip_mm >= 0.0, "{normal:?}");

    let mut report = live.fetch(&beijing()).expect("Open-Meteo answers");
    report.normals = Some(normal);
    let text = live.render(&report);
    println!("{text}");
    assert!(text.contains("climate_normals: "), "{text}");
    assert!(
        text.contains("Climate normals computed from NOAA NCEI Global Summary of the Month"),
        "{text}"
    );
}
