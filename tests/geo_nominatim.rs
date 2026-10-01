// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Nominatim geocoder: request shape, jsonv2 mapping, cache and self-throttling.
//!
//! Every case replays a recorded body through `StubTransport` and asserts the waits the geocoder
//! asks a `FakeClock` for, so no test opens a socket and none of them sleeps.
//!
//! `tests/fixtures/geo/nominatim_search_tsinghua.json` is a recording of
//! `GET https://nominatim.openstreetmap.org/search?format=jsonv2&q=Tsinghua&limit=10&addressdetails=1&extratags=1`
//! made with the same `User-Agent` this client sends and `Accept-Language: en`, trimmed to its
//! first three hits (the recording has seven). The language header is why the fixture says
//! `China` where a locale-dependent recording says `中国` — the client pins the language for the
//! same reason the Open-Meteo geocoder pins `language=en`.
//! `nominatim_search_tsinghua_ambiguous.json` is a **minimised** fixture: it keeps only the keys
//! the mapper reads (`lat`, `lon`, `display_name`, `address`, `extratags`) plus `place_id`, drops
//! the `licence`/`boundingbox`/`importance` fields a real response carries, and its populations
//! are hand-written, because real OSM objects named `Tsinghua…` carry no `population` tag at all.
//! OpenStreetMap data is ODbL-1.0.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono_tz::Tz;
use serde::Deserialize;

use cirrocast::cache::{Cache, CacheMode, Clock, FakeClock};
use cirrocast::geo::nominatim::{DEFAULT_URL, Nominatim};
use cirrocast::geo::{Geocoder, location_line};
use cirrocast::http::{HttpClient, StubReply, StubTransport, UA};
use cirrocast::model::LocationSource;

/// The instant every test starts from: a fixed, plausible wall clock.
const START: u64 = 1_700_000_000;

/// The throttle state file, relative to the cache root.
const THROTTLE_STATE: &str = "ratelimit/nominatim.json";

/// The payload of that file.
#[derive(Debug, Deserialize)]
struct ThrottleState {
    last_request_unix_ms: u128,
}

/// The clock every test starts from.
fn start() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(START)
}

/// `time` as milliseconds since the Unix epoch.
fn millis(time: SystemTime) -> u128 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .expect("the clock starts after the epoch")
        .as_millis()
}

/// The recorded Tsinghua response.
fn tsinghua() -> StubReply {
    StubReply::json_file("tests/fixtures/geo/nominatim_search_tsinghua.json")
        .expect("the recorded Tsinghua fixture is readable")
}

/// The minimised pair whose upstream order contradicts its population order.
fn ambiguous() -> StubReply {
    StubReply::json_file("tests/fixtures/geo/nominatim_search_tsinghua_ambiguous.json")
        .expect("the ambiguous fixture is readable")
}

/// One geocoder under test, plus the handles a test asserts on.
struct Harness {
    dir: tempfile::TempDir,
    transport: Arc<StubTransport>,
    clock: Arc<FakeClock>,
    cache: Cache,
    http: HttpClient,
}

impl Harness {
    /// A fresh cache root, a scripted transport and a clock that only moves when asked.
    fn new(replies: Vec<StubReply>) -> Self {
        let dir = tempfile::tempdir().expect("a temporary cache root");
        let transport = Arc::new(StubTransport::new(replies));
        let clock = Arc::new(FakeClock::new(start()));
        let cache = Cache::with_root(
            dir.path(),
            CacheMode::Normal,
            Arc::clone(&clock) as Arc<dyn Clock>,
            0,
        );
        let http = HttpClient::new(Box::new(Arc::clone(&transport)), 3, clock.clone(), 0);
        Self {
            dir,
            transport,
            clock,
            cache,
            http,
        }
    }

    /// The public service, built the way the CLI builds it.
    fn geocoder(&self) -> Nominatim<'_> {
        Nominatim::new(&self.http, &self.cache, DEFAULT_URL)
    }

    /// The cache root, for asserting on the state file directly.
    fn root(&self) -> &Path {
        self.dir.path()
    }
}

#[test]
fn the_recorded_tsinghua_response_maps_to_osm_locations() {
    let harness = Harness::new(vec![tsinghua()]);
    let hits = harness
        .geocoder()
        .search("Tsinghua", 10)
        .expect("the recorded body decodes");

    assert_eq!(hits.len(), 3, "the fixture holds three trimmed hits");
    assert!(hits.iter().all(|hit| hit.source == LocationSource::Osm));
    // The service's own order survives the mapping; ranking belongs to `cirrocast::geo::resolve`.
    assert_eq!(hits[0].name, "Tsinghua University");
    assert_eq!(hits[0].admin1, None);
    assert_eq!(
        hits[1].name,
        "Tsinghua Southeast Asia Center (Tsinghua SEA)"
    );
    assert_eq!(hits[1].country, "Indonesia");
    assert_eq!(hits[1].admin1.as_deref(), Some("Bali"));

    let chinese = hits
        .iter()
        .find(|hit| hit.country == "China" && hit.admin1.is_some())
        .expect("the fixture holds a Chinese hit with a state");
    assert_eq!(
        chinese.name,
        "Tsinghua University (Shenzhen International Graduate School)"
    );
    assert_eq!(chinese.admin1.as_deref(), Some("Guangdong"));
    assert_eq!(chinese.country_code.as_deref(), Some("cn"));
    assert!((chinese.lat - 22.594_959_8).abs() < 1e-6);
    assert!((chinese.lon - 113.964_401_5).abs() < 1e-6);
    assert_eq!(chinese.elevation_m, None);
    assert_eq!(chinese.population, None);

    // No hit in this recording carries an `extratags.timezone` tag, so its zone stays the
    // provisional UTC that an `Osm` location renders as "resolved at fetch time".
    assert_eq!(chinese.tz, Tz::UTC);
    assert_eq!(
        location_line(chinese),
        "Tsinghua University (Shenzhen International Graduate School), Guangdong, China \
         (22.59, 113.96) <timezone resolved at fetch time>"
    );
}

#[test]
fn the_search_request_follows_the_usage_policy() {
    let harness = Harness::new(vec![tsinghua()]);
    harness
        .geocoder()
        .search("Tsinghua", 10)
        .expect("the fixture decodes");

    let calls = harness.transport.calls();
    assert_eq!(calls.len(), 1);
    let request = &calls[0];
    assert_eq!(request.url(), "https://nominatim.openstreetmap.org/search");
    assert_eq!(
        request.full_url(),
        "https://nominatim.openstreetmap.org/search\
         ?format=jsonv2&q=Tsinghua&limit=10&addressdetails=1&extratags=1"
    );
    assert!(
        request
            .headers()
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("user-agent") && value == UA),
        "the usage policy requires the shared User-Agent: {:?}",
        request.headers()
    );
}

#[test]
fn a_second_request_waits_out_the_one_request_per_second_policy() {
    let harness = Harness::new(vec![tsinghua(), tsinghua()]);
    let geocoder = harness.geocoder();
    geocoder
        .search("Tsinghua", 10)
        .expect("the first search succeeds");
    geocoder
        .search("Peking University", 10)
        .expect("the second search succeeds");

    assert_eq!(harness.transport.calls().len(), 2);
    // The first call had no recorded request to honour and did not wait at all; the fake clock
    // only moves when something asks it to, so the second one waited out the whole window.
    assert_eq!(harness.clock.sleeps(), vec![Duration::from_secs(1)]);

    let path = harness.root().join(THROTTLE_STATE);
    assert!(
        path.exists(),
        "the send time is recorded before the request: {}",
        path.display()
    );
    let text = harness
        .cache
        .read_state(THROTTLE_STATE)
        .expect("the state file is readable")
        .expect("the state file is written");
    let state: ThrottleState = serde_json::from_str(&text).expect("the state file is JSON");
    assert_eq!(state.last_request_unix_ms, millis(harness.clock.now()));
    assert_eq!(state.last_request_unix_ms, u128::from(START + 1) * 1000);
}

#[test]
fn a_cached_response_is_served_without_throttling() {
    let harness = Harness::new(vec![tsinghua()]);
    let geocoder = harness.geocoder();
    let first = geocoder
        .search("Tsinghua", 10)
        .expect("the first search succeeds");
    let second = geocoder
        .search("Tsinghua", 10)
        .expect("the second search is a cache hit");

    assert_eq!(first, second);
    assert_eq!(
        harness.transport.calls().len(),
        1,
        "a cache hit sends nothing"
    );
    assert_eq!(
        harness.clock.sleeps(),
        Vec::<Duration>::new(),
        "a cache hit does not wait"
    );
    assert_eq!(
        harness.clock.now(),
        start(),
        "a cache hit does not move the clock"
    );
}

#[test]
fn the_cache_key_ignores_case_and_surrounding_whitespace() {
    let harness = Harness::new(vec![tsinghua()]);
    let geocoder = harness.geocoder();
    geocoder
        .search("Tsinghua", 10)
        .expect("the first search succeeds");
    let hits = geocoder
        .search("  tsinghua  ", 10)
        .expect("the second search is a cache hit");

    assert_eq!(hits.len(), 3);
    assert_eq!(harness.transport.calls().len(), 1);
}

#[test]
fn a_damaged_throttle_state_file_is_not_an_error() {
    let harness = Harness::new(vec![tsinghua()]);
    harness
        .cache
        .write_state(THROTTLE_STATE, "{not json")
        .expect("the damaged state file is written");

    let hits = harness
        .geocoder()
        .search("Tsinghua", 10)
        .expect("a damaged state file means no recorded request");

    assert_eq!(hits.len(), 3);
    assert_eq!(harness.clock.sleeps(), Vec::<Duration>::new());
    let text = harness
        .cache
        .read_state(THROTTLE_STATE)
        .expect("the state file is readable")
        .expect("the state file is rewritten");
    serde_json::from_str::<ThrottleState>(&text).expect("the search rewrote a valid state file");
}

#[test]
fn a_state_file_from_the_future_still_waits_a_full_second() {
    let harness = Harness::new(vec![tsinghua()]);
    let ahead = millis(start()) + 5_000;
    harness
        .cache
        .write_state(
            THROTTLE_STATE,
            &format!(r#"{{"last_request_unix_ms": {ahead}}}"#),
        )
        .expect("the state file is written");

    harness
        .geocoder()
        .search("Tsinghua", 10)
        .expect("the search succeeds");

    // A clock that moved backwards must over-throttle rather than send early.
    assert_eq!(harness.clock.sleeps(), vec![Duration::from_secs(1)]);
}

#[test]
fn hits_without_a_timezone_tag_keep_the_provisional_utc_zone() {
    let harness = Harness::new(vec![ambiguous()]);
    let hits = harness
        .geocoder()
        .search("Tsinghua", 10)
        .expect("the fixture decodes");

    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|hit| hit.tz == Tz::UTC));
    assert!(hits.iter().all(|hit| hit.source == LocationSource::Osm));
    assert!(
        hits.iter()
            .all(|hit| location_line(hit).ends_with("<timezone resolved at fetch time>"))
    );

    // The upstream order is deliberately the opposite of the population order: a consumer that
    // trusted the service's first hit would pick the school over the university. `search` must
    // not reorder anything — `cirrocast::geo::rank` is what does that, on the caller's side.
    assert_eq!(hits[0].name, "Tsinghua University High School");
    assert_eq!(hits[0].population, Some(1200));
    assert_eq!(
        hits[1].name, "Tsinghua University",
        "the hit has no `name` key"
    );
    assert_eq!(hits[1].population, Some(47_000));
    assert_eq!(hits[1].admin1.as_deref(), Some("Beijing"));
    assert_eq!(hits[1].country, "China");
}
