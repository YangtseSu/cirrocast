// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `GeoNames` `searchJSON` geocoder: request shape, mapping, refusals and caching.
//!
//! Every case replays a body through `StubTransport`, so the suite never opens a socket.
//! `tests/fixtures/geo/geonames_search_beijing.json` is **hand-authored** from the documented
//! `style=FULL` shape (the free `demo` account was out of quota on 2026-10-06, so no live body
//! could be recorded): the two usable rows carry the `GeoNames` values Open-Meteo also serves for
//! the same places, and the other three rows exercise the rows the decoder must drop (the `0, 0`
//! sentinel, a missing country code, `-99`). `geonames_quota.json` is a verbatim recording of the
//! service's own quota refusal. The place data is `GeoNames`' (CC BY 4.0).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono_tz::Tz;
use cirrocast::cache::{Cache, CacheMode, FakeClock};
use cirrocast::geo::Geocoder as _;
use cirrocast::geo::geonames::{CREDENTIAL, ENV_VAR, GeoNamesGeocoder, SEARCH_URL};
use cirrocast::http::{HttpClient, StubReply, StubTransport};
use cirrocast::model::LocationSource;

/// The cache lifetime the tests use; the exact value only matters to the cache.
const TTL: Duration = Duration::from_hours(720);

/// The account name every test sends; it is a credential, never a fixed value in the product.
const USER: &str = "cirrocast-test";

/// Loads one fixture as a reply with the given status.
fn fixture(name: &str, status: u16) -> StubReply {
    let body = std::fs::read_to_string(format!("tests/fixtures/geo/{name}"))
        .expect("the fixture is readable");
    StubReply::ok(status, body)
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
fn the_fixture_decodes_into_locations() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (_, cache, client) = harness(
        dir.path(),
        vec![fixture("geonames_search_beijing.json", 200)],
        CacheMode::Normal,
    );
    let geocoder = GeoNamesGeocoder::new(&client, &cache, TTL, USER);

    let hits = geocoder.search("Beijing", 10).expect("the fixture decodes");

    // The `0, 0` row, the one without a country code and the `-99` one are dropped.
    assert_eq!(hits.len(), 3, "{hits:?}");
    let top = &hits[0];
    assert_eq!(top.name, "Beijing");
    assert_eq!(top.admin1.as_deref(), Some("Beijing"));
    assert_eq!(top.country, "China");
    assert_eq!(top.country_code.as_deref(), Some("CN"));
    assert_eq!(top.lat, 39.9075);
    assert_eq!(top.lon, 116.39723);
    assert_eq!(top.tz, Tz::Asia__Shanghai);
    assert_eq!(top.population, Some(18_960_744));
    assert_eq!(top.source, LocationSource::Geonames);
    assert_eq!(top.elevation_m, None);
    assert_eq!(top.station, None);

    // The Shanxi village of the same name is a separate candidate, in the service's order.
    assert_eq!(hits[1].admin1.as_deref(), Some("Shanxi"));
    assert_eq!(hits[1].population, None, "a zero population is unknown");
    assert_eq!(hits[2].name, "Beixiang");
    assert_eq!(hits[2].admin1.as_deref(), Some("Guangdong"));
}

#[test]
fn the_request_on_the_wire_is_the_documented_one() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(
        dir.path(),
        vec![fixture("geonames_search_beijing.json", 200)],
        CacheMode::Normal,
    );
    let geocoder = GeoNamesGeocoder::new(&client, &cache, TTL, USER);

    geocoder.search("Beijing", 10).expect("the fixture decodes");

    let calls = transport.calls();
    assert_eq!(calls.len(), 1, "one lookup is one request");
    assert_eq!(calls[0].url(), SEARCH_URL);
    assert_eq!(
        calls[0].full_url(),
        format!("{SEARCH_URL}?q=Beijing&fuzzy=0.8&maxRows=10&style=FULL&username={USER}")
    );
    assert_eq!(
        calls[0].headers(),
        [] as [(String, String); 0],
        "the client's own User-Agent is the only header"
    );
    // The account name is a credential: the redacted spellings are what may be logged.
    let redacted = calls[0].redacted_url();
    assert!(!redacted.contains(USER), "{redacted}");
    assert!(redacted.contains("username=***"), "{redacted}");
}

#[test]
fn a_quota_body_is_an_upstream_error_not_an_empty_result() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (_, cache, client) = harness(
        dir.path(),
        vec![fixture("geonames_quota.json", 200)],
        CacheMode::Normal,
    );
    let geocoder = GeoNamesGeocoder::new(&client, &cache, TTL, USER);

    let error = geocoder
        .search("Beijing", 10)
        .expect_err("a quota body is a failure, not a hit list");

    assert_eq!(error.exit_code(), 3);
    let text = error.to_string();
    assert!(text.contains("geonames"), "{text}");
    assert!(text.contains("daily limit"), "{text}");
    assert!(text.contains("status 18"), "{text}");
}

/// The `401` the service answers for an account it will not accept is the missing-key failure
/// naming the two commands that fix it — not the upstream's own wording.
#[test]
fn a_401_is_the_missing_key_failure() {
    for body in [
        r#"{"status":{"message":"Please add a username to each call in order for geonames to be able to identify the calling application and count the credits usage.","value":10}}"#,
        r#"{"status":{"message":"invalid user","value":10}}"#,
    ] {
        let dir = tempfile::tempdir().expect("a temporary cache root");
        let (_, cache, client) = harness(
            dir.path(),
            vec![StubReply::ok(401, body)],
            CacheMode::Normal,
        );
        let geocoder = GeoNamesGeocoder::new(&client, &cache, TTL, USER);

        let error = geocoder
            .search("Beijing", 10)
            .expect_err("a 401 is a credential failure");

        assert_eq!(error.exit_code(), 6, "{error}");
        let text = error.to_string();
        assert!(text.contains(CREDENTIAL), "{text}");
        assert!(text.contains(ENV_VAR), "{text}");
        assert!(text.contains("cirrocast key set geonames"), "{text}");
    }
}

#[test]
fn an_answer_is_served_from_the_cache() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(
        dir.path(),
        vec![fixture("geonames_search_beijing.json", 200)],
        CacheMode::Normal,
    );
    let geocoder = GeoNamesGeocoder::new(&client, &cache, TTL, USER);

    let first = geocoder
        .search("Beijing", 10)
        .expect("the first lookup fetches");
    // Case and surrounding blanks are the same lookup, so this one is a cache hit.
    let second = geocoder
        .search("  beijing ", 10)
        .expect("the second lookup hits");
    assert_eq!(first, second);
    assert_eq!(transport.calls().len(), 1);
    assert!(
        dir.path().join("geocode").is_dir(),
        "the answer is cached under the geocode namespace"
    );
}

#[test]
fn offline_without_a_cached_answer_is_a_network_error() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(
        dir.path(),
        vec![fixture("geonames_search_beijing.json", 200)],
        CacheMode::Offline,
    );
    let geocoder = GeoNamesGeocoder::new(&client, &cache, TTL, USER);

    let error = geocoder
        .search("Beijing", 10)
        .expect_err("nothing is cached");

    assert_eq!(error.exit_code(), 3);
    let text = error.to_string();
    assert!(text.contains("offline"), "{text}");
    assert!(text.contains("geonames"), "{text}");
    assert_eq!(transport.calls().len(), 0, "offline never opens a socket");
}
