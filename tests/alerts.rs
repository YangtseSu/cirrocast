// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Severe-weather alerts end to end: the source adapters against scripted transports, the fetch
//! policy, and the CLI's formats over a seeded alert cache.
//!
//! Nothing here touches the network: the adapters run over [`StubTransport`] with the recorded
//! fixtures in `tests/fixtures/alerts/`, and the CLI runs use `--offline` over cache entries the
//! test writes itself (the sandbox also sets `CIRROCAST_FORBID_NETWORK=1` for every child).

mod common;

use std::fs;
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;

use cirrocast::alerts::{self, AlertSource, AlertsRequest};
use cirrocast::cache::{CACHE_SCHEMA_VERSION, Cache, CacheKey, CacheMode, FakeClock};
use cirrocast::config::Config;
use cirrocast::config::keys::KeyStore;
use cirrocast::http::{HttpClient, StubReply, StubTransport};
use cirrocast::model::{Alert, Location, LocationSource, Severity};
use cirrocast::paths::Paths;
use cirrocast::provider::Env;

use common::{Sandbox, fixture_path};

/// The instant every adapter test runs at: alerts from the fixtures are in force then.
const NOW: &str = "2026-10-03T06:00:00Z";

/// The point the Beijing geocode fixture resolves to; every CLI alert cache key uses it.
const LAT: f64 = 39.9075;
/// The longitude half of the pair.
const LON: f64 = 116.39723;

/// The instant the harness clock starts at.
fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(NOW)
        .expect("a valid instant")
        .with_timezone(&Utc)
}

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

/// Beijing, the fixture point.
fn beijing() -> Location {
    location("Beijing", LAT, LON, Some("CN"), Tz::Asia__Shanghai)
}

/// Oklahoma, the NWS fixture point.
fn norman() -> Location {
    location("Norman", 35.22, -97.44, Some("US"), Tz::America__Chicago)
}

/// One recorded fixture as a `200` reply.
fn fixture(name: &str) -> StubReply {
    StubReply::json_file(fixture_path(&format!("alerts/{name}"))).expect("the fixture is readable")
}

/// A fetch run: scripted transport, throwaway cache, key store and a fixed clock.
struct Harness {
    _directory: tempfile::TempDir,
    http: HttpClient,
    cache: Cache,
    config: Config,
    keys: KeyStore,
    transport: Arc<StubTransport>,
}

impl Harness {
    /// A run at [`NOW`].
    fn new(replies: Vec<StubReply>, mode: CacheMode) -> Self {
        Self::at(replies, mode, now())
    }

    /// A run at `start`.
    fn at(replies: Vec<StubReply>, mode: CacheMode, start: DateTime<Utc>) -> Self {
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
        let cache = Cache::with_root(directory.path().join("cache"), mode, clock, 0);
        Self {
            _directory: directory,
            http,
            cache,
            config: Config::default(),
            keys: KeyStore::new(&paths),
            transport,
        }
    }

    /// Runs the alert fetch for `sources`.
    fn fetch(
        &self,
        loc: &Location,
        sources: &[AlertSource],
        explicit: bool,
        threshold: Severity,
    ) -> cirrocast::error::Result<Vec<Alert>> {
        let env = Env {
            http: &self.http,
            cache: &self.cache,
            config: &self.config,
            keys: &self.keys,
            quiet: true,
            verbose: 0,
        };
        alerts::fetch(
            loc,
            &env,
            &AlertsRequest {
                sources: sources.to_vec(),
                explicit,
                threshold,
            },
            "en-US",
        )
    }

    /// Every request the transport saw.
    fn calls(&self) -> Vec<cirrocast::http::HttpRequest> {
        self.transport.calls()
    }
}

#[test]
fn nws_decodes_the_active_endpoint_and_drops_cancellations() {
    let harness = Harness::new(vec![fixture("nws-tornado.json")], CacheMode::Normal);
    let alerts = harness
        .fetch(&norman(), &[AlertSource::Nws], true, Severity::Unknown)
        .expect("the feed decodes");
    assert_eq!(alerts.len(), 1, "the cancellation is not an alert");
    let alert = &alerts[0];
    assert_eq!(alert.event, "Tornado Warning");
    assert_eq!(alert.severity, Severity::Extreme);
    assert_eq!(alert.areas, ["Cleveland, OK", "McClain, OK"]);
    assert_eq!(alert.sender.as_deref(), Some("NWS Norman OK"));
    assert_eq!(
        alert.instruction.as_deref(),
        Some("Take shelter now. Mobile homes will be damaged or destroyed.")
    );

    let calls = harness.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url(), "https://api.weather.gov/alerts/active");
    assert!(
        calls[0]
            .query_pairs()
            .iter()
            .any(|(key, value)| key == "point" && value == "35.2200,-97.4400"),
        "{:?}",
        calls[0].query_pairs()
    );
    assert!(
        calls[0]
            .headers()
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("accept")
                && value == "application/geo+json"),
        "{:?}",
        calls[0].headers()
    );
}

#[test]
fn wmoswic_fetches_the_index_and_the_cap_document_and_caches_both() {
    let harness = Harness::new(
        vec![fixture("wmoswic-index.json"), fixture("wmoswic-cap.xml")],
        CacheMode::Normal,
    );
    let alerts = harness
        .fetch(&beijing(), &[AlertSource::WmoSwic], true, Severity::Unknown)
        .expect("the aggregator decodes");
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].event, "gale");
    assert_eq!(
        alerts[0].areas,
        ["Wuqing District, Tianjin", "天津市武清区"]
    );
    assert_eq!(harness.calls().len(), 2, "index plus one CAP document");

    // The second run is served from the two cache entries: no further request is scripted, so a
    // network attempt would fail.
    let again = harness
        .fetch(&beijing(), &[AlertSource::WmoSwic], true, Severity::Unknown)
        .expect("the cached answer decodes");
    assert_eq!(again, alerts);
    assert_eq!(harness.calls().len(), 2, "the cache served both resources");
}

#[test]
fn fpas_applies_the_category_msgtype_and_geometry_filters() {
    let harness = Harness::new(
        vec![
            fixture("fpas-area.json"),
            fixture("fpas-gale.xml"),
            fixture("fpas-quake.xml"),
            fixture("fpas-cancel.xml"),
        ],
        CacheMode::Normal,
    );
    let alerts = harness
        .fetch(&beijing(), &[AlertSource::Fpas], true, Severity::Unknown)
        .expect("the area list decodes");
    assert_eq!(alerts.len(), 1, "the Geo and Cancel documents are dropped");
    assert_eq!(alerts[0].event, "gale");
    assert_eq!(harness.calls().len(), 4, "one list plus three documents");
}

#[test]
fn hko_maps_the_warning_summary_and_reads_the_statement_text() {
    let harness = Harness::new(
        vec![fixture("hko-warnsum.json"), fixture("hko-warninginfo.json")],
        CacheMode::Normal,
    );
    let hong_kong = location("Hong Kong", 22.3, 114.17, Some("HK"), Tz::Asia__Hong_Kong);
    let alerts = harness
        .fetch(&hong_kong, &[AlertSource::Hko], true, Severity::Unknown)
        .expect("the summary decodes");
    assert_eq!(alerts.len(), 2, "the CANCEL entry is dropped");
    let cyclone = alerts
        .iter()
        .find(|alert| alert.event.contains("Tropical Cyclone"))
        .expect("the cyclone signal is there");
    assert_eq!(cyclone.severity, Severity::Severe);
    assert!(
        cyclone
            .description
            .as_deref()
            .unwrap_or_default()
            .contains("No. 8 signal"),
        "{cyclone:?}"
    );
    let rain = alerts
        .iter()
        .find(|alert| alert.event.contains("Rainstorm"))
        .expect("the rainstorm warning is there");
    assert_eq!(rain.severity, Severity::Moderate);
}

#[test]
fn hko_stops_after_the_empty_summary() {
    let harness = Harness::new(vec![fixture("hko-warnsum-empty.json")], CacheMode::Normal);
    let hong_kong = location("Hong Kong", 22.3, 114.17, Some("HK"), Tz::Asia__Hong_Kong);
    let alerts = harness
        .fetch(&hong_kong, &[AlertSource::Hko], true, Severity::Unknown)
        .expect("an empty summary is not an error");
    assert_eq!(alerts.len(), 0);
    assert_eq!(
        harness.calls().len(),
        1,
        "no detail request without a warning"
    );
}

#[test]
fn qweather_reuses_the_provider_host_and_credential() {
    let mut harness = Harness::new(vec![fixture("qweather-rainstorm.json")], CacheMode::Normal);
    harness.config.providers.qweather.host = "https://abc123.re.qweatherapi.com".to_owned();
    harness
        .keys
        .set("qweather", "test-key-qweather")
        .expect("the key store accepts the key");
    let alerts = harness
        .fetch(
            &beijing(),
            &[AlertSource::QWeather],
            true,
            Severity::Unknown,
        )
        .expect("the warning list decodes");
    assert_eq!(alerts.len(), 1, "the Cancel record is dropped");
    assert_eq!(alerts[0].event, "暴雨");
    assert_eq!(alerts[0].severity, Severity::Severe);
    assert!(
        !alerts[0]
            .description
            .as_deref()
            .unwrap_or_default()
            .contains('<'),
        "markup is stripped"
    );

    let calls = harness.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].url(),
        "https://abc123.re.qweatherapi.com/weatheralert/v7/alert/now"
    );
    assert!(
        calls[0]
            .query_pairs()
            .iter()
            .any(|(key, value)| key == "location" && value == "116.3972,39.9075"),
        "{:?}",
        calls[0].query_pairs()
    );
    assert!(
        calls[0]
            .headers()
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("X-QW-Api-Key")),
        "the credential travels in the documented header"
    );
}

#[test]
fn an_expired_alert_never_leaves_the_fetch() {
    let later = DateTime::parse_from_rfc3339("2026-10-03T14:00:00Z")
        .expect("a valid instant")
        .with_timezone(&Utc);
    let harness = Harness::at(vec![fixture("nws-tornado.json")], CacheMode::Normal, later);
    let alerts = harness
        .fetch(&norman(), &[AlertSource::Nws], true, Severity::Unknown)
        .expect("the feed decodes");
    assert_eq!(alerts.len(), 0, "ends 08:15-05:00 is past 14:00Z");
}

#[test]
fn a_duplicate_from_two_sources_is_one_alert_and_the_stronger_survives() {
    // The NWS warning and the FPAS copy share the event, the onset and both area names, so the
    // `(event, onset, areas)` rule collapses them; NWS reports it as Extreme, FPAS as Severe.
    let harness = Harness::new(
        vec![
            fixture("nws-tornado.json"),
            fixture("fpas-duplicate-area.json"),
            fixture("fpas-duplicate.xml"),
        ],
        CacheMode::Normal,
    );
    let alerts = harness
        .fetch(
            &norman(),
            &[AlertSource::Nws, AlertSource::Fpas],
            true,
            Severity::Unknown,
        )
        .expect("both sources decode");
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].severity, Severity::Extreme);
    assert_eq!(alerts[0].source, AlertSource::Nws);
}

#[test]
fn a_malformed_cap_document_fails_an_explicit_source_and_is_a_note_in_auto() {
    let replies = || {
        vec![
            fixture("fpas-malformed-area.json"),
            fixture("cap-truncated.xml"),
            fixture("cap-no-event.xml"),
        ]
    };
    // Every document under the area is unusable: an explicit source propagates the first failure.
    let harness = Harness::new(replies(), CacheMode::Normal);
    let error = harness
        .fetch(&beijing(), &[AlertSource::Fpas], true, Severity::Unknown)
        .expect_err("a truncated CAP document fails an explicit source");
    assert!(error.to_string().contains("fpas"), "{error}");

    // The same fetch in auto mode is a note: the source yields nothing and the run continues.
    let harness = Harness::new(replies(), CacheMode::Normal);
    let alerts = harness
        .fetch(&beijing(), &[AlertSource::Fpas], false, Severity::Unknown)
        .expect("auto mode degrades");
    assert_eq!(alerts.len(), 0);
}

#[test]
fn a_malformed_cap_document_does_not_hide_its_neighbours() {
    let harness = Harness::new(
        vec![
            fixture("fpas-area.json"),
            fixture("cap-truncated.xml"),
            fixture("cap-no-event.xml"),
            fixture("fpas-gale.xml"),
        ],
        CacheMode::Normal,
    );
    let alerts = harness
        .fetch(&beijing(), &[AlertSource::Fpas], false, Severity::Unknown)
        .expect("the surviving document answers");
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].event, "gale");
}

// ---------------------------------------------------------------------------------------------
// The CLI over a seeded alert cache
// ---------------------------------------------------------------------------------------------

/// Writes one cache entry with an explicit age, so the offline-replay path is testable.
fn seed_entry(
    sandbox: &Sandbox,
    key: &CacheKey,
    body: &str,
    fetched_at: DateTime<Utc>,
    ttl_secs: u64,
) {
    let path = sandbox.cache_dir().join(key.path());
    fs::create_dir_all(path.parent().expect("the entry has a parent"))
        .expect("the cache directory");
    let envelope = serde_json::json!({
        "cache_schema_version": CACHE_SCHEMA_VERSION,
        "key": key.normalised(),
        "fetched_at": fetched_at.to_rfc3339(),
        "ttl_secs": ttl_secs,
        "status": 200,
        "body": body,
    });
    fs::write(
        &path,
        serde_json::to_string_pretty(&envelope).expect("the envelope encodes"),
    )
    .expect("the entry is written");
}

/// The fixture body of `alerts/<name>`.
fn alert_fixture(name: &str) -> String {
    fs::read_to_string(fixture_path(&format!("alerts/{name}"))).expect("the fixture is readable")
}

/// The gale CAP document with its validity window moved around the real clock.
///
/// The CLI runs in these tests read the system clock and drop expired alerts, so the fixture's
/// fixed 2026-10-03/04 window would silently expire and the cached-set assertions would flip to
/// "no active weather alerts" with no code change. The adapter tests keep the raw fixture: they
/// inject their own clock, which is what makes them deterministic.
fn live_gale_fixture() -> String {
    let now = Utc::now().with_timezone(&Tz::Asia__Shanghai);
    let stamp = |offset_hours: i64| {
        (now + chrono::Duration::hours(offset_hours))
            .format("%Y-%m-%dT%H:%M:00+08:00")
            .to_string()
    };
    alert_fixture("fpas-gale.xml")
        .replace("2026-10-03T08:32:00+08:00", &stamp(-1))
        .replace("2026-10-04T08:32:00+08:00", &stamp(6))
}

/// Seeds the geocode answer for `Beijing`, the weather answer for the resolved point and, with
/// `alerts`, a fresh FPAS alert set for it.
fn seed(sandbox: &Sandbox, alerts: bool, stale: bool) {
    let geocode = fs::read_to_string(fixture_path("geo/open_meteo_geocode_beijing.json"))
        .expect("the geocode fixture is readable");
    seed_entry(
        sandbox,
        &CacheKey::hash("geocode", "open-meteo|beijing|10|en"),
        &geocode,
        Utc::now(),
        2_592_000,
    );
    let today = Utc::now().with_timezone(&Tz::Asia__Shanghai).date_naive();
    let weather = fs::read_to_string(fixture_path("open_meteo/forecast_beijing_2026-07-15.json"))
        .expect("the weather fixture is readable");
    seed_entry(
        sandbox,
        &CacheKey::weather("open-meteo", LAT, LON, 3, today),
        &weather,
        Utc::now(),
        600,
    );
    if !alerts {
        return;
    }
    let fetched_at = if stale {
        Utc::now() - chrono::Duration::hours(2)
    } else {
        Utc::now()
    };
    let hour = Utc::now();
    seed_entry(
        sandbox,
        &CacheKey::alert("fpas", LAT, LON, hour),
        &alert_fixture("fpas-area.json"),
        fetched_at,
        300,
    );
    seed_entry(
        sandbox,
        &CacheKey::hash("alerts", "fpas|cap|fpas-gale-1"),
        &live_gale_fixture(),
        fetched_at,
        300,
    );
}

/// Seeds the geocoded Vienna answer and the weather for the point it resolves to.
///
/// A geocoded location is what gives the alert selection a country code (an `@lat,lon` pair has
/// none), and `AT` is what pulls `MeteoAlarm` into the auto-selected set.
fn seed_vienna(sandbox: &Sandbox) {
    let geocode = fs::read_to_string(fixture_path("geo/open_meteo_geocode_vienna.json"))
        .expect("the geocode fixture is readable");
    seed_entry(
        sandbox,
        &CacheKey::hash("geocode", "open-meteo|vienna|10|en"),
        &geocode,
        Utc::now(),
        2_592_000,
    );
    let weather = fs::read_to_string(fixture_path("open_meteo/forecast_berlin_2026-03-29.json"))
        .expect("the weather fixture is readable");
    let today = Utc::now().with_timezone(&Tz::Europe__Vienna).date_naive();
    seed_entry(
        sandbox,
        &CacheKey::weather("open-meteo", 48.20849, 16.37208, 3, today),
        &weather,
        Utc::now(),
        600,
    );
}

/// A successful offline run's stdout.
fn stdout(sandbox: &Sandbox, args: &[&str]) -> String {
    let assert = sandbox
        .cirrocast()
        .args(args)
        .arg("--offline")
        .arg("--alerts-from")
        .arg("fpas")
        .arg("Beijing")
        .assert()
        .success();
    String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8")
}

#[test]
fn a_cached_alert_set_renders_in_every_format() {
    let sandbox = Sandbox::new();
    seed(&sandbox, true, false);

    let listing = stdout(&sandbox, &["--lang", "en-US", "-f", "alerts"]);
    assert!(listing.contains("gale — Minor"), "{listing}");
    assert!(
        listing.contains("FOSS Public Alert Server · Beijing"),
        "{listing}"
    );
    assert!(listing.contains("Secure loose objects."), "{listing}");
    assert!(
        listing.contains("Warnings via the FOSS Public Alert Server (alerts.kde.org)"),
        "{listing}"
    );

    let plain = stdout(&sandbox, &["--lang", "en-US", "-f", "plain"]);
    assert!(
        plain
            .lines()
            .any(|line| line.starts_with("alert: gale — Minor · ")),
        "{plain}"
    );

    let one_line = stdout(
        &sandbox,
        &[
            "--lang",
            "en-US",
            "-f",
            "one-line",
            "--template",
            "alert=%A",
        ],
    );
    assert!(one_line.contains("alert=gale"), "{one_line}");

    let json = stdout(&sandbox, &["--lang", "en-US", "-f", "json"]);
    let document: serde_json::Value = serde_json::from_str(&json).expect("the document is JSON");
    assert_eq!(document["schema_version"], serde_json::json!(2));
    assert_eq!(document["alerts"][0]["event"], serde_json::json!("gale"));
    assert_eq!(
        document["alerts"][0]["severity"],
        serde_json::json!("minor")
    );
    assert!(
        document["alert_credits"][0]
            .as_str()
            .unwrap_or_default()
            .contains("FOSS Public Alert Server"),
        "{json}"
    );

    // The dumb table folds the em dash and the middle dot to ASCII, so only the glyph and the
    // words are asserted.
    let dumb = stdout(&sandbox, &["--lang", "en-US", "-f", "dumb"]);
    assert!(
        dumb.lines().any(|line| line.starts_with("! gale")),
        "{dumb}"
    );
}

#[test]
fn an_expired_cache_entry_is_replayed_offline_with_a_stale_read() {
    let sandbox = Sandbox::new();
    seed(&sandbox, true, true);
    let listing = stdout(&sandbox, &["--lang", "en-US", "-f", "alerts"]);
    assert!(listing.contains("gale — Minor"), "{listing}");
}

#[test]
fn an_empty_alert_set_is_not_an_error_and_empties_the_token() {
    let sandbox = Sandbox::new();
    seed(&sandbox, false, false);
    seed_entry(
        &sandbox,
        &CacheKey::alert("fpas", LAT, LON, Utc::now()),
        "[]",
        Utc::now(),
        300,
    );

    let listing = stdout(&sandbox, &["--lang", "en-US", "-f", "alerts"]);
    assert_eq!(listing.trim(), "no active weather alerts");
    let plain = stdout(&sandbox, &["--lang", "en-US", "-f", "plain"]);
    assert!(
        !plain.lines().any(|line| line.starts_with("alert:")),
        "{plain}"
    );
    let one_line = stdout(&sandbox, &["-f", "one-line", "--template", "alert=%A"]);
    assert!(
        one_line.lines().any(|line| line == "alert="),
        "%A is empty without alerts: {one_line}"
    );
}

#[test]
fn no_alerts_suppresses_a_cached_set() {
    let sandbox = Sandbox::new();
    seed(&sandbox, true, false);
    // `--no-alerts` and `--alerts-from` are mutually exclusive, so this run uses the
    // coverage-selected path; the cached FPAS set is the only source that can answer offline.
    let assert = sandbox
        .cirrocast()
        .args([
            "--offline",
            "--lang",
            "en-US",
            "-f",
            "plain",
            "--no-alerts",
            "Beijing",
        ])
        .assert()
        .success();
    let plain = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    assert!(
        !plain.lines().any(|line| line.starts_with("alert:")),
        "{plain}"
    );
}

#[test]
fn an_explicit_source_outside_its_coverage_is_a_usage_error() {
    let sandbox = Sandbox::new();
    // No cache is seeded: the policy is resolved before the forecast is fetched, so this must not
    // reach any network path (the sandbox has the guard on).
    let assert = sandbox
        .cirrocast()
        .args([
            "--alerts",
            "--lat",
            "39.9",
            "--lon",
            "116.4",
            "--alerts-from",
            "nws",
        ])
        .assert()
        .code(2);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
    assert_eq!(
        stderr.trim(),
        "error: alert source `nws` does not cover 39.90,116.40; covered here: qweather, wmoswic, fpas"
    );
}

#[test]
fn meteoalarm_degrades_and_reports_the_missing_or_bad_token() {
    let sandbox = Sandbox::new();
    seed_vienna(&sandbox);

    // Without a token the source is skipped with a verbose note and the run succeeds.
    let assert = sandbox
        .cirrocast()
        .env_remove("CIRROCAST_METEOALARM_KEY")
        .args(["--alerts", "-v", "--offline", "--lang", "en-US", "Vienna"])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
    assert!(
        stderr.contains("no CIRROCAST_METEOALARM_KEY configured; skipping"),
        "{stderr}"
    );

    // A token that cannot be used is a verbose note too, not a failed run.
    let assert = sandbox
        .cirrocast()
        .env("CIRROCAST_METEOALARM_KEY", "not-a-token")
        .args(["--alerts", "-v", "--offline", "--lang", "en-US", "Vienna"])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
    assert!(stderr.contains("alerts: meteoalarm:"), "{stderr}");
}

#[test]
fn meteoalarm_filters_the_country_index_by_geometry_and_reads_the_cap() {
    let sandbox = Sandbox::new();
    seed_vienna(&sandbox);
    let hour = Utc::now();
    seed_entry(
        &sandbox,
        &CacheKey::alert("meteoalarm", 48.20849, 16.37208, hour),
        &alert_fixture("meteoalarm-at.json"),
        Utc::now(),
        300,
    );
    seed_entry(
        &sandbox,
        &CacheKey::hash("alerts", "meteoalarm|cap|AT-2026-101"),
        &alert_fixture("meteoalarm-heat-cap.xml"),
        Utc::now(),
        300,
    );

    let assert = sandbox
        .cirrocast()
        .env("CIRROCAST_METEOALARM_KEY", "test-token")
        .args([
            "--offline",
            "--alerts-from",
            "meteoalarm",
            "--lang",
            "en-US",
            "-f",
            "alerts",
            "Vienna",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    assert!(stdout.contains("Heat warning — Severe"), "{stdout}");
    assert!(stdout.contains("MeteoAlarm · Wien, Vienna"), "{stdout}");
    assert!(stdout.contains("Find shade and drink water."), "{stdout}");
    // The second index feature's polygon does not contain Vienna: only one CAP document is
    // cached, so a geometry-filter miss would show up as a failed document, not as a second alert.
    assert!(!stdout.contains("Alerte chaleur"), "{stdout}");
}

#[test]
fn provider_info_names_the_alert_source_a_provider_brings() {
    let sandbox = Sandbox::new();
    let assert = sandbox
        .cirrocast()
        .args(["provider", "info", "qweather"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    assert!(
        stdout
            .lines()
            .any(|line| line.trim_start().starts_with("alerts:") && line.contains("qweather")),
        "{stdout}"
    );

    let assert = sandbox
        .cirrocast()
        .args(["provider", "info", "smhi"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    assert!(
        stdout
            .lines()
            .any(|line| line.trim_start().starts_with("alerts:") && line.contains("none")),
        "{stdout}"
    );
}
