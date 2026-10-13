// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `QWeather` `GeoAPI` geocoder, over recorded bodies (step 30).
//!
//! No test here opens a socket: the transport is scripted and the bodies are the recorded ones
//! under `tests/fixtures/qweather/geo/` (`QWeather`'s own data, `LicenseRef-Proprietary-API-Data`).
//! What is asserted is the row mapping the probe decided (the localized `country` and the **absent**
//! ISO code, the administrative chain, the zone), the two refusals the service actually sends
//! (a `400` `#no-such-location` is a no-hit, a `401` is the invalid-credential failure), and that
//! the source is refused under a silenced geo scope instead of opening a socket.
//!
//! The live counterpart is `live_qweather_geoapi` at the bottom of this file, `#[ignore]`d and
//! gated on `CIRROCAST_LIVE_TESTS=1` plus `CIRROCAST_QWEATHER_HOST` and a credential, exactly like
//! the other live smokes in `tests/live.rs`.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use cirrocast::cache::{Cache, CacheMode, Clock, FakeClock};
use cirrocast::config::keys::Credential;
use cirrocast::geo::Geocoder;
use cirrocast::geo::qweather::{PATH, QWeatherGeocoder};
use cirrocast::http::{HttpClient, StubReply, StubTransport};
use cirrocast::model::LocationSource;

/// The account host the tests pretend to have configured: the documented shape, never a real
/// account.
const HOST: &str = "https://example.re.qweatherapi.com";

/// The credential the tests pretend to hold.
const KEY: &str = "0123456789abcdef0123456789abcdef";

/// Loads one recorded body as a reply with `status`.
fn fixture(name: &str, status: u16) -> StubReply {
    let body = std::fs::read_to_string(format!("tests/fixtures/qweather/geo/{name}"))
        .expect("the recorded GeoAPI fixture is readable");
    StubReply::ok(status, body)
}

/// A scripted transport, a private cache and a client over one frozen clock.
fn harness(root: &Path, replies: Vec<StubReply>) -> (Arc<StubTransport>, Cache, HttpClient) {
    let clock: Arc<dyn Clock> = Arc::new(FakeClock::new(SystemTime::UNIX_EPOCH));
    let transport = Arc::new(StubTransport::new(replies));
    let cache = Cache::with_root(root, CacheMode::Normal, Arc::clone(&clock), 0);
    let client = HttpClient::new(Box::new(Arc::clone(&transport)), 3, clock, 0);
    (transport, cache, client)
}

/// The geocoder over the harness, with the credential and host the test wants.
fn geocoder<'a>(
    client: &'a HttpClient,
    cache: &'a Cache,
    host: &'a str,
    credential: &'a Credential,
    offline: bool,
) -> QWeatherGeocoder<'a> {
    QWeatherGeocoder::new(client, cache.clock(), host, credential, offline)
}

/// The district query the bundled table cannot answer: `GeoAPI` returns the district itself, with
/// its administrative chain and its zone, and no ISO country code.
#[test]
fn the_district_query_decodes_from_the_recorded_answer() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) =
        harness(dir.path(), vec![fixture("xinxiangxian-zh.json", 200)]);

    let hits = geocoder(
        &client,
        &cache,
        HOST,
        &Credential::ApiKey(KEY.to_owned()),
        false,
    )
    .search("新乡县", 10)
    .expect("the recorded answer decodes");

    assert_eq!(hits.len(), 10, "the recorded body carries ten rows");
    let district = &hits[0];
    assert_eq!(district.name, "新乡县");
    assert_eq!(district.admin1.as_deref(), Some("河南省"));
    assert_eq!(district.country, "中国");
    assert_eq!(
        district.country_code, None,
        "the service publishes a localized country name and no ISO code"
    );
    assert!((district.lat - 35.190_84).abs() < 1e-6, "{}", district.lat);
    assert!((district.lon - 113.805_21).abs() < 1e-6, "{}", district.lon);
    assert_eq!(district.tz, chrono_tz::Tz::Asia__Shanghai);
    assert_eq!(district.elevation_m, None);
    assert_eq!(
        district.population, None,
        "`rank` is the service's ordering hint, never a population"
    );
    assert_eq!(district.source, LocationSource::QWeather);
    assert_eq!(
        cirrocast::geo::attribution_line(district),
        Some("Location data by QWeather — https://www.qweather.com/")
    );

    // The request the source sent: the endpoint, the query's own script as `lang`, and the
    // credential in the header the vendor documents.
    let calls = transport.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].url(), format!("{HOST}{PATH}"));
    let pairs = calls[0].query_pairs();
    assert!(
        pairs.contains(&("location".to_owned(), "新乡县".to_owned())),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&("number".to_owned(), "10".to_owned())),
        "{pairs:?}"
    );
    assert!(
        pairs.contains(&("lang".to_owned(), "zh".to_owned())),
        "{pairs:?}"
    );
    assert!(
        calls[0]
            .headers()
            .iter()
            .any(|(name, value)| name == "X-QW-Api-Key" && value == KEY),
        "{:?}",
        calls[0].headers()
    );
}

/// A Latin query is answered in Latin, in the service's own order.
#[test]
fn the_english_spelling_decodes_from_the_recorded_answer() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(dir.path(), vec![fixture("beijing-en.json", 200)]);

    let hits = geocoder(
        &client,
        &cache,
        HOST,
        &Credential::ApiKey(KEY.to_owned()),
        false,
    )
    .search("Beijing", 5)
    .expect("the recorded answer decodes");

    let names: Vec<&str> = hits.iter().map(|hit| hit.name.as_str()).collect();
    assert_eq!(
        names,
        ["Beijing", "Haidian", "Chaoyang", "Shunyi", "Huairou"]
    );
    assert_eq!(hits[0].country, "China");
    assert_eq!(hits[0].admin1.as_deref(), Some("Beijing"));
    let pairs = transport.calls()[0].query_pairs().to_vec();
    assert!(
        pairs.contains(&("lang".to_owned(), "en".to_owned())),
        "{pairs:?}"
    );
}

/// The service answers a query that matches no place with a `400` `#no-such-location`, which is a
/// no-hit rather than a failure — exactly what an empty `location[]` would have been.
#[test]
fn a_refusal_for_an_unknown_place_is_a_no_hit() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (_, cache, client) = harness(dir.path(), vec![fixture("no-such-location.json", 400)]);

    let hits = geocoder(
        &client,
        &cache,
        HOST,
        &Credential::ApiKey(KEY.to_owned()),
        false,
    )
    .search("Zzzqqqnotaplace", 5)
    .expect("a place that does not exist is not a failure");
    assert_eq!(hits, Vec::new());
}

/// A credential the service refuses is the shared invalid-credential failure: exit 6, and the
/// message names the commands that replace it.
#[test]
fn a_rejected_credential_is_the_invalid_credential_error() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let body = std::fs::read_to_string("tests/fixtures/qweather/error_401.json")
        .expect("the recorded refusal is readable");
    let (_, cache, client) = harness(dir.path(), vec![StubReply::ok(401, body)]);

    let error = geocoder(
        &client,
        &cache,
        HOST,
        &Credential::ApiKey(KEY.to_owned()),
        false,
    )
    .search("Beijing", 5)
    .expect_err("a refused credential fails the query");
    assert_eq!(error.exit_code(), 6, "{error}");
    assert!(
        error.to_string().contains("rejected the credential"),
        "{error}"
    );
}

/// A missing host is the configuration error naming the console, before any request.
#[test]
fn a_missing_host_is_a_configuration_error() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(dir.path(), Vec::new());

    let error = geocoder(
        &client,
        &cache,
        "  ",
        &Credential::ApiKey(KEY.to_owned()),
        false,
    )
    .search("Beijing", 5)
    .expect_err("no host is a configuration problem");
    assert_eq!(error.exit_code(), 4, "{error}");
    assert!(
        error.to_string().contains("providers.qweather.host"),
        "{error}"
    );
    assert_eq!(transport.calls().len(), 0, "nothing was sent");
}

/// A silenced geo scope refuses the source: it has no cache to answer from, so no socket is opened.
#[test]
fn the_source_is_refused_offline() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(dir.path(), Vec::new());

    let error = geocoder(
        &client,
        &cache,
        HOST,
        &Credential::ApiKey(KEY.to_owned()),
        true,
    )
    .search("Beijing", 5)
    .expect_err("offline has no cached answer for this source");
    assert_eq!(error.exit_code(), 3, "{error}");
    assert!(error.to_string().contains("offline"), "{error}");
    assert_eq!(transport.calls().len(), 0, "offline never opens a socket");
}

/// The live probe: the account's `GeoAPI` answers the district query the bundled table cannot.
///
/// `#[ignore]`d and gated like the other live smokes; it is the one test in this file that may open
/// a socket.
#[test]
#[ignore = "live network: set CIRROCAST_LIVE_TESTS=1 and run `cargo test --test geo_qweather -- --ignored --nocapture`"]
fn live_qweather_geoapi() {
    if std::env::var("CIRROCAST_LIVE_TESTS").is_err() {
        eprintln!("skipping: set CIRROCAST_LIVE_TESTS=1 to run the live probe");
        return;
    }
    let host = std::env::var("CIRROCAST_QWEATHER_HOST").unwrap_or_default();
    let Some(credential) = live_credential() else {
        eprintln!(
            "skipping: set CIRROCAST_QWEATHER_HOST and CIRROCAST_QWEATHER_KEY (or the \
             CIRROCAST_QWEATHER_JWT_* quartet) to run this probe"
        );
        return;
    };

    let dir = tempfile::tempdir().expect("a temporary directory");
    let clock: Arc<dyn Clock> = Arc::new(FakeClock::new(SystemTime::now()));
    let transport = cirrocast::http::UreqTransport::new(
        &cirrocast::config::Network::default(),
        Duration::from_secs(30),
    )
    .expect("the transport builds");
    let client = HttpClient::new(Box::new(transport), 3, Arc::clone(&clock), 0);
    let cache = Cache::with_root(dir.path(), CacheMode::NoCache, clock, 0);
    let hits = QWeatherGeocoder::new(&client, cache.clock(), host.trim(), &credential, false)
        .search("新乡县", 10)
        .expect("the account answers the district query");

    println!("{hits:#?}");
    assert_eq!(hits[0].name, "新乡县", "{hits:#?}");
    assert_eq!(hits[0].country_code, None, "{hits:#?}");
    assert_eq!(hits[0].source, LocationSource::QWeather);
    assert!(
        hits[0].tz != chrono_tz::Tz::UTC,
        "the row carries a real zone: {hits:#?}"
    );
}

/// The credential a live run configured: the key store's resolution over the environment, exactly
/// the way `tests/live.rs` resolves the same credential (the temporary paths have no `keys.toml`,
/// so only the environment can answer).
fn live_credential() -> Option<Credential> {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let paths = cirrocast::paths::Paths {
        config_dir: dir.path().join("config"),
        config_file: dir.path().join("config/config.toml"),
        keys_file: dir.path().join("config/keys.toml"),
        cache_dir: dir.path().join("cache"),
        data_dir: dir.path().join("data"),
    };
    cirrocast::config::keys::KeyStore::new(&paths)
        .credential(cirrocast::provider::ProviderId::QWeather)
        .expect("the store reads the environment")
}
