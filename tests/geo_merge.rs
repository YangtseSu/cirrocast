// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The name-search chain and the candidate merge, end to end over recorded bodies.
//!
//! The chain is driven with a scripted transport, so no test opens a socket: what is asserted is
//! which sources a setting asks, in what order, what the merged candidate list looks like, and
//! that the last-resort rule keeps Nominatim out of a query that already has an answer.
//!
//! Fixtures: `open_meteo_geocode_beijing.json` and `open_meteo_geocode_no_hits.json` are recorded
//! Open-Meteo bodies; `geonames_search_beijing.json` is hand-authored from the documented
//! `searchJSON` shape (the free account was out of quota when this step was written);
//! `nominatim_search_tsinghua.json` is a recorded OSM answer. Place data: `GeoNames` (CC BY 4.0) and
//! OpenStreetMap (`ODbL`).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use cirrocast::cache::{Cache, CacheMode, FakeClock};
use cirrocast::error::Error;
use cirrocast::geo::chain::{GeoSource, SearchChain, SearchInputs};
use cirrocast::geo::merge::merge;
use cirrocast::http::{HttpClient, StubReply, StubTransport};
use cirrocast::model::LocationSource;

/// The cache lifetime the tests use; the exact value only matters to the cache.
const TTL: Duration = Duration::from_hours(720);

/// The account name a test run pretends to have configured.
const USER: &str = "cirrocast-test";

/// Loads one fixture as a `200` reply.
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

/// The inputs a chain run needs, with the account name the test wants.
fn inputs<'a>(
    http: &'a HttpClient,
    cache: &'a Cache,
    geonames_user: Option<&str>,
) -> SearchInputs<'a> {
    SearchInputs {
        http,
        cache,
        ttl: TTL,
        limit: 10,
        geonames_user: geonames_user.map(str::to_owned),
        nominatim_url: "https://nominatim.openstreetmap.org".to_owned(),
        offline: false,
    }
}

/// `auto` with an account: both keyless sources answer, the merge collapses the places they agree
/// on, and Nominatim is never asked.
#[test]
fn auto_merges_open_meteo_and_geonames_and_spares_nominatim() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(
        dir.path(),
        vec![
            fixture("open_meteo_geocode_beijing.json"),
            fixture("geonames_search_beijing.json"),
        ],
        CacheMode::Normal,
    );
    let chain = SearchChain::new("auto", inputs(&client, &cache, Some(USER)))
        .expect("`auto` is a known setting");

    let report = chain.search("Beijing").expect("both sources answer");
    assert_eq!(
        chain.sources(),
        [
            GeoSource::OpenMeteo,
            GeoSource::GeoNames,
            GeoSource::Nominatim
        ]
    );
    assert!(!report.is_empty());

    let merged = merge(&report.answered);
    // Open-Meteo's three Beijings, plus GeoNames' Beixiang; the two rows both sources carry for
    // the capital and for the Shanxi village collapse into the earlier (Open-Meteo) records.
    assert_eq!(merged.len(), 4, "{merged:?}");
    let names: Vec<(&str, Option<&str>)> = merged
        .iter()
        .map(|location| (location.name.as_str(), location.admin1.as_deref()))
        .collect();
    assert_eq!(names[0], ("Beijing", Some("Beijing Municipality")));
    assert!(
        merged
            .iter()
            .any(|location| location.source == LocationSource::Geonames
                && location.name == "Beixiang"),
        "the GeoNames-only candidate survives with its own provenance: {merged:?}"
    );

    let calls = transport.calls();
    assert_eq!(calls.len(), 2, "open-meteo and geonames only: {calls:?}");
    assert_eq!(calls[0].url(), cirrocast::geo::open_meteo::GEOCODE_URL);
    assert_eq!(calls[1].url(), cirrocast::geo::geonames::SEARCH_URL);
    assert!(
        report
            .notes
            .iter()
            .any(|note| note == "nominatim: not asked (an earlier source answered)"),
        "{:?}",
        report.notes
    );
}

/// `auto` without an account: `GeoNames` is skipped with a note naming the fix, and the query still
/// resolves through Open-Meteo.
#[test]
fn auto_without_a_geonames_account_skips_it_and_names_the_fix() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(
        dir.path(),
        vec![fixture("open_meteo_geocode_beijing.json")],
        CacheMode::Normal,
    );
    let chain = SearchChain::new("auto", inputs(&client, &cache, None)).expect("a known setting");

    let report = chain.search("Beijing").expect("Open-Meteo answers");
    assert!(!report.is_empty());
    let skipped = report
        .notes
        .iter()
        .find(|note| note.starts_with("geonames: skipped"))
        .unwrap_or_else(|| panic!("the skip is noted: {:?}", report.notes));
    assert!(skipped.contains("cirrocast key set geonames"), "{skipped}");
    assert!(skipped.contains("CIRROCAST_GEONAMES_USER"), "{skipped}");
    assert_eq!(transport.calls().len(), 1, "GeoNames was not asked");
}

/// `auto` with nothing found: Nominatim is the last resort and is asked then.
#[test]
fn auto_falls_back_to_nominatim_when_nothing_was_found() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(
        dir.path(),
        vec![
            fixture("open_meteo_geocode_no_hits.json"),
            fixture("nominatim_search_tsinghua.json"),
        ],
        CacheMode::Normal,
    );
    let chain = SearchChain::new("auto", inputs(&client, &cache, None)).expect("a known setting");

    let report = chain.search("Tsinghua").expect("Nominatim answers");
    assert!(!report.is_empty());
    let calls = transport.calls();
    assert_eq!(calls.len(), 2, "open-meteo then nominatim: {calls:?}");
    assert_eq!(calls[1].url(), "https://nominatim.openstreetmap.org/search");
    let merged = merge(&report.answered);
    assert!(
        merged
            .iter()
            .any(|location| location.source == LocationSource::Osm),
        "{merged:?}"
    );
}

/// An explicitly selected `geonames` without an account is the missing-key failure, not a skip.
#[test]
fn an_explicit_geonames_selection_without_an_account_is_a_missing_key() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(dir.path(), Vec::new(), CacheMode::Normal);
    let chain =
        SearchChain::new("geonames", inputs(&client, &cache, None)).expect("a known setting");

    let error = chain
        .search("Beijing")
        .expect_err("the account name is missing");

    assert_eq!(error.exit_code(), 6);
    let text = error.to_string();
    assert!(text.contains("cirrocast key set geonames"), "{text}");
    assert!(text.contains("CIRROCAST_GEONAMES_USER"), "{text}");
    assert_eq!(transport.calls().len(), 0);
}

/// Every source failing is one chain error naming all three attempts.
#[test]
fn every_source_failing_names_every_attempt() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    // The client retries a retryable status twice more, so Open-Meteo and `GeoNames` consume three
    // replies each; Nominatim sends exactly once (its throttle forbids a retry loop), so seven in
    // all.
    let (transport, cache, client) = harness(
        dir.path(),
        vec![StubReply::ok(503, "unavailable"); 7],
        CacheMode::NoCache,
    );
    let chain =
        SearchChain::new("auto", inputs(&client, &cache, Some(USER))).expect("a known setting");

    let error = chain.search("Beijing").expect_err("all three fail");

    assert!(matches!(error, Error::Chain { .. }), "{error}");
    assert_eq!(error.exit_code(), 3);
    let text = error.to_string();
    assert!(text.starts_with("all geocoding sources failed:"), "{text}");
    assert!(text.contains("open-meteo"), "{text}");
    assert!(text.contains("geonames"), "{text}");
    assert!(text.contains("nominatim"), "{text}");
    assert_eq!(transport.calls().len(), 7, "{:?}", transport.calls());
}

/// Offline, a source that has nothing cached is a no-hit, not a failure — and no socket is opened.
#[test]
fn offline_turns_every_source_into_a_no_hit() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(dir.path(), Vec::new(), CacheMode::Offline);
    let chain = SearchChain::new(
        "auto",
        SearchInputs {
            offline: true,
            ..inputs(&client, &cache, Some(USER))
        },
    )
    .expect("a known setting");

    let report = chain
        .search("Beijing")
        .expect("a cache miss is not an error");
    assert!(report.is_empty());
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("offline, no cached answer")),
        "{:?}",
        report.notes
    );
    assert_eq!(transport.calls().len(), 0, "offline never opens a socket");
}
