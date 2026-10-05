// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Alert-level regression tests for the 2026-10-05 review (§3.3, §3.13, §3.16).
//!
//! Three independent fixes: the HKO warning cache key must carry the language, a CAP document with
//! a non-`Actual` status must produce no alert, and the point-in-polygon tests must unwrap a ring
//! that crosses the antimeridian. Every case runs over `StubTransport` and the fixtures in
//! `tests/fixtures/alerts/`; no test opens a socket.

mod common;

use std::fs;
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;

use cirrocast::alerts::cap::{alerts_from_cap, parse_cap};
use cirrocast::alerts::geometry::{cap_polygon_contains, geojson_contains};
use cirrocast::alerts::{self, AlertSource, AlertsRequest};
use cirrocast::cache::{Cache, CacheMode, FakeClock};
use cirrocast::config::Config;
use cirrocast::config::keys::KeyStore;
use cirrocast::http::{HttpClient, StubReply, StubTransport};
use cirrocast::model::{Location, LocationSource, Severity};
use cirrocast::paths::Paths;
use cirrocast::provider::Env;

use common::{fixture, fixture_path};

/// The instant the adapter cases run at: the fixtures' warnings are in force then.
const NOW: &str = "2026-10-03T06:00:00Z";

/// A location with the fields the adapters read.
fn location(name: &str, lat: f64, lon: f64, country: Option<&str>, tz: Tz) -> Location {
    Location {
        name: name.to_owned(),
        admin1: None,
        country: country.unwrap_or_default().to_owned(),
        country_code: country.map(str::to_owned),
        lat,
        lon,
        tz,
        elevation_m: None,
        population: None,
        source: LocationSource::Coordinates,
        station: None,
    }
}

/// Hong Kong, the HKO fixture point.
fn hong_kong() -> Location {
    location("Hong Kong", 22.3, 114.17, Some("HK"), Tz::Asia__Hong_Kong)
}

/// One recorded fixture as a `200` reply.
fn reply(name: &str) -> StubReply {
    StubReply::json_file(fixture_path(&format!("alerts/{name}"))).expect("the fixture is readable")
}

/// A fetch run: scripted transport, throwaway cache, key store and a fixed clock.
///
/// A trimmed copy of the harness in `tests/alerts.rs`, kept local so this regression file does not
/// depend on helpers another task is editing.
struct Harness {
    directory: tempfile::TempDir,
    http: HttpClient,
    cache: Cache,
    config: Config,
    keys: KeyStore,
    transport: Arc<StubTransport>,
}

impl Harness {
    /// A run at [`NOW`].
    fn new(replies: Vec<StubReply>) -> Self {
        let start = DateTime::parse_from_rfc3339(NOW)
            .expect("a valid instant")
            .with_timezone(&Utc);
        let directory = tempfile::tempdir().expect("a temporary directory");
        let paths = Paths {
            config_dir: directory.path().join("config"),
            config_file: directory.path().join("config/config.toml"),
            keys_file: directory.path().join("config/keys.toml"),
            cache_dir: directory.path().join("cache"),
            data_dir: directory.path().join("data"),
        };
        let clock = Arc::new(FakeClock::new(SystemTime::from(start)));
        let transport = Arc::new(StubTransport::new(replies));
        let http = HttpClient::new(Box::new(Arc::clone(&transport)), 0, clock.clone(), 0);
        let cache = Cache::with_root(directory.path().join("cache"), CacheMode::Normal, clock, 0);
        Self {
            directory,
            http,
            cache,
            config: Config::default(),
            keys: KeyStore::new(&paths),
            transport,
        }
    }

    /// Every request the transport saw.
    fn calls(&self) -> Vec<cirrocast::http::HttpRequest> {
        self.transport.calls()
    }

    /// How many entry files the cache currently holds.
    fn cache_file_count(&self) -> usize {
        fn walk(directory: &std::path::Path) -> usize {
            let Ok(entries) = fs::read_dir(directory) else {
                return 0;
            };
            entries
                .map(|entry| entry.expect("a directory entry"))
                .map(|entry| {
                    let path = entry.path();
                    if path.is_dir() { walk(&path) } else { 1 }
                })
                .sum()
        }
        walk(&self.directory.path().join("cache"))
    }

    /// Fetches HKO warnings for `language`.
    fn fetch_hko(&self, language: &str) -> Vec<cirrocast::model::Alert> {
        let env = Env {
            http: &self.http,
            cache: &self.cache,
            config: &self.config,
            keys: &self.keys,
            quiet: true,
            verbose: 0,
        };
        alerts::fetch(
            &hong_kong(),
            &env,
            &AlertsRequest {
                sources: vec![AlertSource::Hko],
                explicit: true,
                threshold: Severity::Unknown,
            },
            language,
        )
        .expect("the HKO fixtures decode")
    }
}

#[test]
fn the_hko_cache_key_carries_the_language() {
    // §3.3: the warning summary is the one request whose URL varies with the output language, and
    // the cache key used to omit it. The second run (a different language) must therefore reach the
    // transport again instead of being served the first language's summary.
    let harness = Harness::new(vec![
        reply("hko-warnsum.json"),
        reply("hko-warninginfo.json"),
        reply("hko-warnsum.json"),
        reply("hko-warninginfo.json"),
    ]);

    let chinese = harness.fetch_hko("zh-CN");
    assert_eq!(harness.calls().len(), 2, "one summary plus one detail");
    let after_chinese = harness.cache_file_count();
    assert_eq!(after_chinese, 2, "summary and detail entries are stored");

    let english = harness.fetch_hko("en-US");
    assert!(
        harness.cache_file_count() > after_chinese,
        "the English run must write its own cache entry, not reuse the Chinese one"
    );
    let summaries = harness
        .calls()
        .iter()
        .filter(|call| {
            call.query_pairs()
                .iter()
                .any(|(name, value)| name == "dataType" && value == "warnsum")
        })
        .count();
    assert_eq!(
        summaries, 2,
        "the English run must fetch the summary again instead of reusing the Chinese entry"
    );
    assert!(
        !chinese.is_empty() && !english.is_empty(),
        "both runs decode the same fixture warnings"
    );
}

/// A CAP fixture with the given `status`, parsed and turned into alerts.
fn alerts_for_status(file: &str) -> Vec<cirrocast::model::Alert> {
    let document = parse_cap(&fixture(file), AlertSource::Fpas).expect("the CAP fixture parses");
    alerts_from_cap(&document, AlertSource::Fpas, "en-US").expect("the document is usable")
}

#[test]
fn a_non_actual_cap_status_produces_no_alert() {
    // §3.13: a drill published through an aggregator used to render as a live warning. Test,
    // Exercise and Draft are the three non-operational CAP statuses.
    for file in [
        "alerts/cap-status-test.xml",
        "alerts/cap-status-exercise.xml",
        "alerts/cap-status-draft.xml",
    ] {
        assert!(
            alerts_for_status(file).is_empty(),
            "{file} must not produce an alert"
        );
    }
}

#[test]
fn an_actual_cap_status_still_produces_an_alert() {
    // The guard must filter the drills only: the recorded `status = Actual` document still yields
    // its warning.
    let alerts = alerts_for_status("alerts/fpas-gale.xml");
    assert_eq!(alerts.len(), 1, "the actual warning survives");
    assert_eq!(alerts[0].event, "gale");
}

#[test]
fn a_cap_polygon_across_the_antimeridian_keeps_the_point_inside_it() {
    // A box spanning lon 175°E → 175°W (i.e. 175° to 185°E). The point at 178.45°E is inside, and
    // the point at 0° is not; the raw even-odd cast reports exactly the opposite because the two
    // straddling edges sit at +175 and −175.
    const POLYGON: &str = "-20,175 -15,175 -15,-175 -20,-175 -20,175";
    assert_eq!(
        cap_polygon_contains(POLYGON, -17.5, 178.45),
        Some(true),
        "a point inside the antimeridian-crossing box is kept"
    );
    assert_eq!(
        cap_polygon_contains(POLYGON, -17.5, 0.0),
        Some(false),
        "a point near 0° is not a false positive"
    );
}

#[test]
fn a_geojson_ring_across_the_antimeridian_keeps_the_point_inside_it() {
    let geometry = serde_json::json!({
        "type": "Polygon",
        "coordinates": [[
            [175.0, -20.0], [175.0, -15.0], [-175.0, -15.0], [-175.0, -20.0], [175.0, -20.0]
        ]]
    });
    assert_eq!(
        geojson_contains(Some(&geometry), -17.5, 178.45),
        Some(true),
        "a point inside the antimeridian-crossing ring is kept"
    );
    assert_eq!(
        geojson_contains(Some(&geometry), -17.5, 0.0),
        Some(false),
        "a point near 0° is not a false positive"
    );
}

#[test]
fn an_ordinary_polygon_still_contains_its_interior() {
    // The unwrap must not change the result for a ring that does not cross ±180°.
    const POLYGON: &str = "39.5,116.0 40.3,116.0 40.3,116.9 39.5,116.9 39.5,116.0";
    assert_eq!(cap_polygon_contains(POLYGON, 39.9, 116.4), Some(true));
    assert_eq!(cap_polygon_contains(POLYGON, 41.0, 116.4), Some(false));
}
