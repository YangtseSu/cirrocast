// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Open-Meteo geocoding client, driven by recorded fixtures.
//!
//! Every case replays a body recorded from the real endpoint through `StubTransport`, so the suite
//! never opens a socket: what is asserted is the translation from the endpoint's JSON to
//! [`cirrocast::model::Location`], the exact request on the wire, and the cache behaviour
//! (`--offline` included) that the caller depends on.
//!
//! `open_meteo_geocode_beijing_ambiguous.json` is the recorded Beijing body, trimmed to the three
//! hits the read fixture keeps, with the array *reordered* so the largest-population hit comes
//! last — upstream order and population order disagree there. The client is expected to preserve
//! upstream order; the exact-name filter and the population ranking live in `cirrocast::geo::rank`.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono_tz::Tz;
use cirrocast::cache::{Cache, CacheMode, FakeClock};
use cirrocast::error::Error;
use cirrocast::geo::Geocoder as _;
use cirrocast::geo::open_meteo::{GEOCODE_URL, OpenMeteoGeocoder};
use cirrocast::http::{HttpClient, StubReply, StubTransport};
use cirrocast::model::LocationSource;

/// The cache lifetime the tests use; the exact value only matters to the cache, not the geocoder.
const TTL: Duration = Duration::from_secs(2_592_000);

/// The primary hit of the recorded Beijing body: the one `GeoNames` reports a population for.
const BEIJING: (f64, f64, u64, &str) = (39.9075, 116.39723, 18_960_744, "Beijing Municipality");

/// Loads one recorded fixture as a `200` reply.
fn fixture(name: &str) -> StubReply {
    StubReply::json_file(format!("tests/fixtures/geo/{name}"))
        .expect("the recorded geocode fixture is readable")
}

/// A scripted transport, a private cache and a client over one frozen clock.
fn harness(
    root: &Path,
    replies: Vec<StubReply>,
    mode: CacheMode,
) -> (Arc<StubTransport>, Cache, HttpClient) {
    let clock = Arc::new(FakeClock::new(SystemTime::UNIX_EPOCH));
    let transport = Arc::new(StubTransport::new(replies));
    let cache = Cache::with_root(root, mode, clock.clone(), 0);
    let client = HttpClient::new(Box::new(Arc::clone(&transport)), 3, clock, 0);
    (transport, cache, client)
}

#[test]
fn the_beijing_fixture_decodes_into_locations() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (_, cache, client) = harness(
        dir.path(),
        vec![fixture("open_meteo_geocode_beijing.json")],
        CacheMode::Normal,
    );
    let geocoder = OpenMeteoGeocoder::new(&client, &cache, TTL);

    let hits = geocoder.search("Beijing", 10).expect("the fixture decodes");

    assert_eq!(hits.len(), 3);
    let top = &hits[0];
    assert_eq!(top.name, "Beijing");
    assert_eq!((top.lat, top.lon), (BEIJING.0, BEIJING.1));
    assert_eq!(top.tz, Tz::Asia__Shanghai);
    assert_eq!(top.population, Some(BEIJING.2));
    assert_eq!(top.admin1.as_deref(), Some(BEIJING.3));
    assert_eq!(top.country, "China");
    assert_eq!(top.country_code.as_deref(), Some("CN"));
    assert_eq!(top.elevation_m, Some(49.0));
    assert_eq!(top.source, LocationSource::Geocoder);
}

#[test]
fn the_request_on_the_wire_is_the_documented_one() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(
        dir.path(),
        vec![fixture("open_meteo_geocode_beijing.json")],
        CacheMode::Normal,
    );
    let geocoder = OpenMeteoGeocoder::new(&client, &cache, TTL);

    geocoder.search("Beijing", 10).expect("the fixture decodes");

    let calls = transport.calls();
    assert_eq!(calls.len(), 1, "one lookup is one request");
    assert_eq!(calls[0].url(), GEOCODE_URL);
    assert_eq!(
        calls[0].full_url(),
        format!("{GEOCODE_URL}?name=Beijing&count=10&language=en&format=json")
    );
    assert!(
        calls[0].headers().is_empty(),
        "the client's own User-Agent is the only header"
    );
}

#[test]
fn a_result_without_a_usable_iana_zone_is_an_upstream_error() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (_, cache, client) = harness(
        dir.path(),
        vec![fixture("open_meteo_geocode_bad_timezone.json")],
        CacheMode::Normal,
    );
    let geocoder = OpenMeteoGeocoder::new(&client, &cache, TTL);

    let error = geocoder
        .search("Olympus Mons", 10)
        .expect_err("`Mars/Olympus` is not an IANA zone");

    assert_eq!(error.exit_code(), 3);
    match error {
        Error::Upstream {
            provider,
            status,
            message,
        } => {
            assert_eq!(provider, "open-meteo-geocoding");
            assert_eq!(status, None);
            assert!(message.contains("has no usable IANA timezone"), "{message}");
            assert!(message.contains("Mars/Olympus"), "{message}");
        }
        other => panic!("expected an upstream error, got {other:?}"),
    }
}

#[test]
fn a_missing_results_key_is_an_empty_result_not_an_error() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(
        dir.path(),
        vec![fixture("open_meteo_geocode_no_hits.json")],
        CacheMode::Normal,
    );
    let geocoder = OpenMeteoGeocoder::new(&client, &cache, TTL);

    let hits = geocoder
        .search("Zzyzxq", 10)
        .expect("nothing matched is a legitimate answer");

    assert!(hits.is_empty(), "expected no hits, got {hits:?}");
    assert_eq!(transport.calls().len(), 1);
}

#[test]
fn a_second_lookup_is_served_from_the_cache() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    // Exactly one reply: a second request to the transport fails, so the assertions below can only
    // pass when the lookup came out of the cache.
    let (transport, cache, client) = harness(
        dir.path(),
        vec![fixture("open_meteo_geocode_beijing.json")],
        CacheMode::Normal,
    );
    let geocoder = OpenMeteoGeocoder::new(&client, &cache, TTL);

    let first = geocoder
        .search("Beijing", 10)
        .expect("the first lookup fetches");
    let second = geocoder
        .search("Beijing", 10)
        .expect("the second lookup is cached");
    // The key trims and lower-cases, so another spelling of the same name is the same lookup.
    let respelled = geocoder
        .search("  beijing ", 10)
        .expect("another spelling hits the same entry");

    assert_eq!(first, second);
    assert_eq!(first, respelled);
    assert_eq!(transport.calls().len(), 1);
}

#[test]
fn offline_without_an_entry_fails_before_the_transport_is_asked() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(dir.path(), Vec::new(), CacheMode::Offline);
    let geocoder = OpenMeteoGeocoder::new(&client, &cache, TTL);

    let error = geocoder
        .search("Beijing", 10)
        .expect_err("nothing is cached yet");

    assert!(matches!(&error, Error::Network(_)), "{error:?}");
    assert_eq!(error.exit_code(), 3);
    let text = error.to_string();
    assert!(
        text.contains("offline mode: no cached entry for geocode/"),
        "{text}"
    );
    assert!(
        transport.calls().is_empty(),
        "offline mode never reaches the transport"
    );
}

#[test]
fn hits_keep_the_upstream_order() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (_, cache, client) = harness(
        dir.path(),
        vec![fixture("open_meteo_geocode_beijing_ambiguous.json")],
        CacheMode::Normal,
    );
    let geocoder = OpenMeteoGeocoder::new(&client, &cache, TTL);

    let hits = geocoder.search("Beijing", 10).expect("the fixture decodes");

    assert_eq!(hits.len(), 3);
    // The small hit the fixture puts first stays first: filtering and ranking are the caller's job.
    assert_eq!((hits[0].population, hits[0].lat), (None, 35.20917));
    assert_eq!(
        (hits[2].population, hits[2].lat),
        (Some(BEIJING.2), BEIJING.0)
    );
    assert_eq!(hits[2].admin1.as_deref(), Some(BEIJING.3));
}
