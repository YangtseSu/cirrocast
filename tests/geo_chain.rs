// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `qweather` geo source's place in the name-search chain (step 30).
//!
//! Three properties are the point of this file, and each one is a rule a later change could undo by
//! accident:
//!
//! * `[geo] search = "qweather"` answers through the `GeoAPI` and narrates that it stored nothing;
//! * `auto` never selects it, even when a credential is configured — a chain test proves no request
//!   goes to the `GeoAPI` host;
//! * an explicitly selected source without a credential is the exit-6 credential error, not a
//!   silent skip, and its storage rule holds: a search leaves the cache root untouched and a repeat
//!   query goes out again.
//!
//! Everything runs over a scripted transport with the recorded bodies under
//! `tests/fixtures/qweather/geo/`; no test here opens a socket.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use cirrocast::cache::{Cache, CacheMode, Clock, FakeClock};
use cirrocast::config::keys::Credential;
use cirrocast::geo::chain::{GeoSource, QWeatherInputs, SearchChain, SearchInputs};
use cirrocast::geo::qweather::PATH as QWEATHER_PATH;
use cirrocast::http::{HttpClient, StubReply, StubTransport};
use cirrocast::model::LocationSource;

/// The cache lifetime the tests use; the exact value only matters to the cache.
const TTL: Duration = Duration::from_hours(720);

/// The account host the tests pretend to have configured: the documented shape, never a real
/// account.
const HOST: &str = "https://example.re.qweatherapi.com";

/// Loads a recorded `GeoAPI` body with `status`.
fn qweather_fixture(name: &str, status: u16) -> StubReply {
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

/// The inputs a chain run needs, with a `qweather` credential the caller decides on.
fn inputs<'a>(
    http: &'a HttpClient,
    cache: &'a Cache,
    qweather: Option<QWeatherInputs>,
) -> SearchInputs<'a> {
    SearchInputs {
        http,
        cache,
        ttl: TTL,
        limit: 10,
        geonames_user: None,
        qweather,
        nominatim_url: "https://nominatim.openstreetmap.org".to_owned(),
        offline: false,
    }
}

/// The credential a configured account would hold.
fn credential() -> Credential {
    Credential::ApiKey("0123456789abcdef0123456789abcdef".to_owned())
}

/// Every file under `root`, so a test can prove nothing was stored.
fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}

/// An explicit `qweather` selection answers through the `GeoAPI`, and the `-v` narration says the
/// answer was not stored.
#[test]
fn an_explicit_qweather_selection_answers_through_the_geoapi() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(
        dir.path(),
        vec![qweather_fixture("xinxiangxian-zh.json", 200)],
    );
    let chain = SearchChain::new(
        "qweather",
        inputs(
            &client,
            &cache,
            Some(QWeatherInputs {
                host: HOST.to_owned(),
                credential: credential(),
            }),
        ),
    )
    .expect("`qweather` is a known setting");

    assert_eq!(chain.sources(), [GeoSource::QWeather]);
    let report = chain.search("新乡县").expect("the GeoAPI answers");
    assert!(!report.is_empty());
    let hits = &report.answered[0].1;
    assert_eq!(hits[0].name, "新乡县");
    assert_eq!(hits[0].source, LocationSource::QWeather);
    assert_eq!(
        cirrocast::geo::attribution_line(&hits[0]),
        Some("Location data by QWeather — https://www.qweather.com/")
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note
                == "qweather: 10 candidates (not cached: GeoAPI data may not be stored)"),
        "{:?}",
        report.notes
    );

    let calls = transport.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].url(), format!("{HOST}{QWEATHER_PATH}"));
}

/// `auto` never selects `qweather`, even with a credential configured: the request stays with the
/// keyless sources.
#[test]
fn auto_never_selects_qweather() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(
        dir.path(),
        vec![
            StubReply::json_file("tests/fixtures/geo/open_meteo_geocode_beijing.json")
                .expect("the recorded Open-Meteo fixture is readable"),
        ],
    );
    let chain = SearchChain::new(
        "auto",
        inputs(
            &client,
            &cache,
            Some(QWeatherInputs {
                host: HOST.to_owned(),
                credential: credential(),
            }),
        ),
    )
    .expect("`auto` is a known setting");

    assert!(
        !chain.sources().contains(&GeoSource::QWeather),
        "{:?}",
        chain.sources()
    );
    let report = chain.search("Beijing").expect("Open-Meteo answers");
    assert!(!report.is_empty());

    let calls = transport.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].url(), cirrocast::geo::open_meteo::GEOCODE_URL);
    assert!(
        calls.iter().all(|call| !call.url().contains(QWEATHER_PATH)),
        "`auto` must never reach the GeoAPI host: {calls:?}"
    );
    assert!(
        report
            .notes
            .iter()
            .all(|note| !note.starts_with("qweather:")),
        "{:?}",
        report.notes
    );
}

/// An explicitly selected `qweather` without a credential is the exit-6 credential error, never a
/// silent skip.
#[test]
fn an_explicit_qweather_without_a_credential_is_the_credential_error() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(dir.path(), Vec::new());
    let chain = SearchChain::new("qweather", inputs(&client, &cache, None))
        .expect("`qweather` is a known setting");

    let error = chain
        .search("Beijing")
        .expect_err("a credential-bound source needs a credential");
    assert_eq!(error.exit_code(), 6, "{error}");
    let text = error.to_string();
    assert!(text.contains("key set qweather"), "{text}");
    assert!(text.contains("CIRROCAST_QWEATHER_KEY"), "{text}");
    assert_eq!(transport.calls().len(), 0, "nothing was sent");
}

/// The storage rule: a search writes no cache entry under any name, so a repeat query asks again.
#[test]
fn a_qweather_search_stores_nothing_and_asks_again() {
    let dir = tempfile::tempdir().expect("a temporary cache root");
    let (transport, cache, client) = harness(
        dir.path(),
        vec![
            qweather_fixture("xinxiangxian-zh.json", 200),
            qweather_fixture("xinxiangxian-zh.json", 200),
        ],
    );
    let qweather = || QWeatherInputs {
        host: HOST.to_owned(),
        credential: credential(),
    };
    let chain = SearchChain::new("qweather", inputs(&client, &cache, Some(qweather())))
        .expect("`qweather` is a known setting");

    chain.search("新乡县").expect("the first query answers");
    assert_eq!(
        files_under(dir.path()),
        Vec::<PathBuf>::new(),
        "GeoAPI data must not be stored in any form"
    );

    chain
        .search("新乡县")
        .expect("the second query answers, from the service");
    assert_eq!(
        transport.calls().len(),
        2,
        "a repeat query is a fresh request: {:?}",
        transport.calls()
    );
}
