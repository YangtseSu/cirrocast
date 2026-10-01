// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Decoder robustness: what every upstream decoder does with a hostile body.
//!
//! The sweep is deterministic and offline: each recorded payload is truncated at every byte offset
//! and mutated one byte at a time, then fed through the real decoder over `StubTransport`. The
//! contract under test is the failure taxonomy, not the wording — a decoder may accept a mutated
//! body (some mutations stay valid JSON) but it may never panic, hang or produce an error class
//! that misrepresents the cause (`Error::Upstream` for a body that does not decode, `Error::Config`
//! for the configuration document).
//!
//! Truncation is the realistic failure class for a flaky connection, and it is what `serde` alone
//! does not protect against: a decoder that slices a string before parsing, or indexes a vector it
//! assumed non-empty, fails exactly here. The sweeps are bounded by the fixture sizes.

mod common;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use cirrocast::cache::{Cache, CacheMode};
use cirrocast::config::Config;
use cirrocast::config::keys::KeyStore;
use cirrocast::error::Error;
use cirrocast::geo::Geocoder as _;
use cirrocast::geo::ip::{IpLocator as _, IpLocatorChain, IpService};
use cirrocast::geo::open_meteo::OpenMeteoGeocoder;
use cirrocast::http::{HttpClient, StubReply, StubTransport};
use cirrocast::model::Location;
use cirrocast::paths::Paths;
use cirrocast::provider::{Env, FetchRequest, HourlyResolution, ProviderId, provider_for};

use common::{Sandbox, fixture_location, fixture_path, provider_clock};

/// One backend under the sweep: its id, where it is asked for, the replies that answer any request
/// before the body under test, and the recorded payloads to sweep.
struct Case {
    id: ProviderId,
    location: Location,
    /// Valid replies served before the body under test — a backend that answers from two requests
    /// (`OpenWeatherMap`'s current + forecast, `QWeather`'s current + hourly) needs the first one
    /// answered before the second decoder is reached.
    prefix: &'static [&'static str],
    fixtures: &'static [&'static str],
}

/// Every backend and the smallest recorded payload that exercises each of its decoders.
fn cases() -> Vec<Case> {
    let beijing = || fixture_location("beijing");
    vec![
        Case {
            id: ProviderId::OpenMeteo,
            location: beijing(),
            prefix: &[],
            fixtures: &["open_meteo/forecast_beijing_2026-07-15.json"],
        },
        Case {
            id: ProviderId::OpenWeatherMap,
            location: beijing(),
            prefix: &[],
            fixtures: &["owm/current.json"],
        },
        Case {
            id: ProviderId::OpenWeatherMap,
            location: beijing(),
            prefix: &["owm/current.json"],
            fixtures: &["owm/forecast.json"],
        },
        Case {
            id: ProviderId::WeatherApi,
            location: beijing(),
            prefix: &[],
            fixtures: &["weatherapi/forecast.json"],
        },
        Case {
            id: ProviderId::WorldWeatherOnline,
            location: beijing(),
            prefix: &[],
            fixtures: &["wwo/weather_ashx.json"],
        },
        Case {
            id: ProviderId::PirateWeather,
            location: beijing(),
            prefix: &[],
            fixtures: &["pirateweather/forecast.json"],
        },
        Case {
            id: ProviderId::QWeather,
            location: beijing(),
            prefix: &[],
            fixtures: &["qweather/current.json"],
        },
        Case {
            id: ProviderId::QWeather,
            location: beijing(),
            prefix: &["qweather/current.json"],
            fixtures: &["qweather/hourly.json"],
        },
        Case {
            id: ProviderId::Smhi,
            location: beijing(),
            prefix: &[],
            fixtures: &["smhi/point_stockholm_2026-09-30.json"],
        },
        Case {
            id: ProviderId::Metar,
            location: cirrocast::provider::metar::placeholder_location("ZBAA"),
            prefix: &[],
            fixtures: &["metar/ZBAA/current.json"],
        },
    ]
}

/// One provider wired to a scripted transport, a throwaway key store and an I/O-free cache.
struct Harness {
    _directory: tempfile::TempDir,
    transport: Arc<StubTransport>,
    http: HttpClient,
    cache: Cache,
    config: Config,
    keys: KeyStore,
    id: ProviderId,
    location: Location,
    prefix: &'static [&'static str],
}

impl Harness {
    fn new(case: &Case) -> Self {
        let id = case.id;
        let directory = tempfile::tempdir().expect("a temporary directory");
        let paths = Paths {
            config_dir: directory.path().join("config"),
            config_file: directory.path().join("config/config.toml"),
            keys_file: directory.path().join("config/keys.toml"),
            cache_dir: directory.path().join("cache"),
            data_dir: directory.path().join("data"),
        };
        let clock: Arc<dyn cirrocast::cache::Clock> = provider_clock(2026, 9, 30);
        let transport = Arc::new(StubTransport::new(Vec::new()));
        let http = HttpClient::new(Box::new(Arc::clone(&transport)), 0, Arc::clone(&clock), 0);
        // `NoCache` keeps the sweep off the disk: nothing is read, nothing is written.
        let cache = Cache::with_root(directory.path().join("cache"), CacheMode::NoCache, clock, 0);
        let keys = KeyStore::new(&paths);
        if id.metadata().requires_key {
            keys.set(id.as_str(), "throwaway-decoder-sweep-key")
                .expect("the throwaway key is stored");
        }
        let mut config = Config::default();
        // QWeather refuses to build a request without an account host.
        "https://test-account.re.qweatherapi.example"
            .clone_into(&mut config.providers.qweather.host);
        Self {
            _directory: directory,
            transport,
            http,
            cache,
            config,
            keys,
            id,
            location: case.location.clone(),
            prefix: case.prefix,
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

    /// Feeds one body through the provider's fetch path.
    fn decode(&self, body: &str) -> Result<(), Error> {
        for name in self.prefix {
            self.transport.push(StubReply::ok(
                200,
                fs::read_to_string(fixture_path(name)).expect("prefix"),
            ));
        }
        self.transport.push(StubReply::ok(200, body));
        let provider = provider_for(self.id).expect("the id has a backend");
        let request = FetchRequest::new(3, HourlyResolution::Hourly);
        provider
            .fetch(&self.location, &request, &self.env())
            .map(|_| ())
    }
}

/// Asserts one payload is either decoded or refused with a truthful class.
///
/// A mutated body may still be valid JSON (`"wind": 12` → `"wind": 1X` is not, but a mutation
/// inside a free-form string is), so `Ok` is not a failure. What is a failure: a panic (the test
/// would abort), a wrong error class, or an error that names no provider.
fn check(harness: &Harness, body: &str, what: &str) {
    match harness.decode(body) {
        Ok(()) => {}
        Err(error) => {
            assert!(
                matches!(error, Error::Upstream { .. } | Error::Network(_)),
                "{what}: {error:?}"
            );
            if let Error::Upstream { provider, .. } = &error {
                assert_eq!(
                    provider,
                    harness.id.as_str(),
                    "{what}: the error names the wrong provider"
                );
            }
        }
    }
}

/// The most offsets one fixture contributes to a sweep.
///
/// A recorded payload up to `EXHAUSTIVE_LIMIT + MAX_OFFSETS` bytes is therefore swept at every byte
/// offset; beyond that the stride grows so one 84 KB fixture costs the same as a 16 KB one. The
/// bound exists because a debug-build decode costs about a millisecond: the four largest recorded
/// payloads would otherwise put this file into the minutes on their own.
const MAX_OFFSETS: usize = 2048;

/// Payloads up to this size are swept at every single byte offset.
const EXHAUSTIVE_LIMIT: usize = 8 * 1024;

/// The step between swept offsets for a payload of `len` bytes.
fn stride(len: usize) -> usize {
    if len <= EXHAUSTIVE_LIMIT {
        1
    } else {
        len.div_ceil(MAX_OFFSETS).max(1)
    }
}

/// Truncates `fixture` at each swept offset and checks every prefix.
fn truncation_sweep(harness: &Harness, fixture: &str) {
    let body = fs::read_to_string(fixture_path(fixture)).expect("the fixture is readable");
    let step = stride(body.len());
    let mut end = 0;
    while end <= body.len() {
        if body.is_char_boundary(end) {
            check(
                harness,
                &body[..end],
                &format!("{fixture} truncated to {end} bytes"),
            );
        }
        end += step;
    }
}

/// Replaces one byte at a time with `X` (or `Y` when it already is `X`), over the swept offsets.
fn mutation_sweep(harness: &Harness, fixture: &str) {
    let body = fs::read_to_string(fixture_path(fixture)).expect("the fixture is readable");
    let mut bytes = body.into_bytes();
    let step = stride(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let original = bytes[index];
        bytes[index] = if original == b'X' { b'Y' } else { b'X' };
        let mutated = String::from_utf8_lossy(&bytes).into_owned();
        check(
            harness,
            &mutated,
            &format!("{fixture} byte {index} flipped"),
        );
        bytes[index] = original;
        index += step;
    }
}

#[test]
fn every_provider_decoder_survives_truncation_and_mutation() {
    for case in cases() {
        let harness = Harness::new(&case);
        for fixture in case.fixtures {
            truncation_sweep(&harness, fixture);
            mutation_sweep(&harness, fixture);
        }
    }
}

#[test]
fn empty_and_wrong_typed_bodies_are_upstream_errors() {
    let malformed = [
        "malformed/empty.json",
        "malformed/empty-object.json",
        "malformed/empty-array.json",
        "malformed/null.json",
        "malformed/wrong-types.json",
        "malformed/wrong-type-nested.json",
    ];
    for case in cases() {
        let harness = Harness::new(&case);
        for name in malformed {
            let body = fs::read_to_string(fixture_path(name)).expect("the fixture is readable");
            let error = harness
                .decode(&body)
                .expect_err(&format!("{} accepted {name}", case.id));
            assert!(
                matches!(error, Error::Upstream { .. }),
                "{} with {name}: {error:?}",
                case.id
            );
        }
    }
}

#[test]
fn the_geocoder_decoder_survives_the_sweep() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let clock: Arc<dyn cirrocast::cache::Clock> = provider_clock(2026, 9, 30);
    let transport = Arc::new(StubTransport::new(Vec::new()));
    let http = HttpClient::new(Box::new(Arc::clone(&transport)), 0, Arc::clone(&clock), 0);
    let cache = Cache::with_root(directory.path().join("cache"), CacheMode::NoCache, clock, 0);
    let geocoder = OpenMeteoGeocoder::new(&http, &cache, Duration::from_secs(600));
    let decode = |body: &str| {
        transport.push(StubReply::ok(200, body));
        geocoder.search("Beijing", 10)
    };

    let body = fs::read_to_string(fixture_path("geo/open_meteo_geocode_beijing.json"))
        .expect("the fixture is readable");
    let step = stride(body.len());
    let mut end = 0;
    while end <= body.len() {
        if body.is_char_boundary(end)
            && let Err(error) = decode(&body[..end])
        {
            assert!(
                matches!(error, Error::Upstream { .. }),
                "geocode truncated to {end}: {error:?}"
            );
        }
        end += step;
    }
    for name in [
        "malformed/empty.json",
        "malformed/null.json",
        "malformed/geocode-wrong-types.json",
    ] {
        let body = fs::read_to_string(fixture_path(name)).expect("the fixture is readable");
        let error = decode(&body).expect_err(&format!("the geocoder accepted {name}"));
        assert!(matches!(error, Error::Upstream { .. }), "{name}: {error:?}");
    }

    // `{}` is a legitimate "nothing matched" answer (the `results` key is absent), while a JSON
    // array is not a geocoding envelope at all and must be an upstream error.
    let body = fs::read_to_string(fixture_path("malformed/empty-object.json"))
        .expect("the fixture is readable");
    let hits = decode(&body).expect("an absent `results` key is no hits");
    assert!(hits.is_empty(), "`{{}}` produced hits");

    let body = fs::read_to_string(fixture_path("malformed/empty-array.json"))
        .expect("the fixture is readable");
    let error = decode(&body).expect_err("an array is not a geocoding envelope");
    assert!(matches!(error, Error::Upstream { .. }), "{error:?}");
}

#[test]
fn the_ip_locator_decoder_survives_the_sweep() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let clock: Arc<dyn cirrocast::cache::Clock> = provider_clock(2026, 9, 30);
    let transport = Arc::new(StubTransport::new(Vec::new()));
    let http = HttpClient::new(Box::new(Arc::clone(&transport)), 0, Arc::clone(&clock), 0);
    let cache = Cache::with_root(directory.path().join("cache"), CacheMode::NoCache, clock, 0);
    let services = IpService::chain("auto").expect("`auto` is a known setting");
    let decode = |body: &str| {
        // The chain tries each service in turn; a malformed primary must fall through to the
        // secondary, so both replies are the same body.
        transport.push(StubReply::ok(200, body));
        transport.push(StubReply::ok(200, body));
        IpLocatorChain::new(&http, &cache, services.clone(), Duration::from_secs(600)).locate()
    };

    for fixture in ["ip/ipwho_is_beijing.json", "ip/ipapi_co_beijing.json"] {
        let body = fs::read_to_string(fixture_path(fixture)).expect("the fixture is readable");
        for end in 0..body.len() {
            if !body.is_char_boundary(end) {
                continue;
            }
            if let Err(error) = decode(&body[..end]) {
                // Both services refused: the chain reports the last failure, which is an upstream
                // error. A location error would mean the body decoded into a place, which a
                // truncated body cannot honestly do.
                assert!(
                    matches!(error, Error::Upstream { .. } | Error::Network(_)),
                    "{fixture} truncated to {end}: {error:?}"
                );
            }
        }
    }
}

#[test]
fn the_config_decoder_survives_the_sweep() {
    // The configuration parser is only reachable through the CLI, so the sweep pays one process
    // per offset — which is why the document is a small representative one.
    let document = "schema_version = 1\n[defaults]\nprovider = \"open-meteo\"\nformat = \"plain\"\n\
                    units = \"metric\"\ndays = 3\nlanguage = \"auto\"\n[render]\ncolor = \"never\"\n";
    let sandbox = Sandbox::new();
    for end in 0..=document.len() {
        if !document.is_char_boundary(end) {
            continue;
        }
        sandbox.write_config(&document[..end]);
        let assert = sandbox.cirrocast().args(["config", "validate"]).assert();
        let output = assert.get_output();
        let code = output.status.code().unwrap_or(-1);
        assert!(
            code == 0 || code == 4,
            "a document truncated to {end} bytes exited {code}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
