// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Climate normals end to end: the NCEI two-step flow over scripted transports, the averaging and
//! its three gates, the cache and offline semantics, the rendered surfaces and the CLI over a
//! seeded normals cache.
//!
//! Nothing here touches the network: the decoder runs over [`StubTransport`] with the recorded
//! fixtures in `tests/fixtures/normals/` and the CLI runs use `--offline` over cache entries the
//! test writes itself (the sandbox also sets `CIRROCAST_FORBID_NETWORK=1` for every child).
//!
//! The fixture payloads are recordings (see the directory's `README.md`); the *gate* payloads are
//! trimmed from them in the tests rather than stored as extra fixtures, because a gate needs a
//! real record with one condition changed, not a different station's data.

mod common;

use std::fs;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use unicode_width::UnicodeWidthStr as _;

use cirrocast::cache::{CACHE_SCHEMA_VERSION, Cache, CacheKey, CacheMode, FakeClock};
use cirrocast::config::keys::KeyStore;
use cirrocast::config::{Config, UnitOverrides};
use cirrocast::http::{HttpClient, StubReply, StubTransport};
use cirrocast::i18n::{I18n, LanguageRequest};
use cirrocast::model::units::UnitSystem;
use cirrocast::model::{Location, LocationSource, Report};
use cirrocast::normals::ncei::{DATA_BASE, SEARCH_BASE};
use cirrocast::paths::Paths;
use cirrocast::provider::Env;
use cirrocast::render::{ColorMode, Format, RenderContext, TermCaps, renderer_for};

use common::{Sandbox, fixture_path};

/// The instant the decoder tests run at; the month is a parameter, so the clock only ages cache
/// entries.
const NOW: &str = "2026-10-06T06:00:00Z";

/// The point the recorded Beijing station answers for (its own box is the default 60 km).
const BEIJING: (f64, f64) = (39.9042, 116.4074);

/// The point the recorded Madison station answers for.
const MADISON: (f64, f64) = (43.0731, -89.4012);

/// The instant the harness clock starts at.
fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(NOW)
        .expect("a valid instant")
        .with_timezone(&Utc)
}

/// A location with the fields the decoder reads.
fn location(name: &str, (lat, lon): (f64, f64), tz: Tz) -> Location {
    Location {
        name: name.to_owned(),
        admin1: None,
        country: String::new(),
        country_code: None,
        lat,
        lon,
        tz,
        elevation_m: None,
        population: None,
        source: LocationSource::Coordinates,
        station: None,
        named_by: None,
    }
}

/// Beijing at the recorded station's own point.
fn beijing() -> Location {
    location("Beijing", BEIJING, Tz::Asia__Shanghai)
}

/// Madison at the recorded station's own point.
fn madison() -> Location {
    location("Madison", MADISON, Tz::America__Chicago)
}

/// One recorded fixture as a `200` reply.
fn fixture(name: &str) -> StubReply {
    StubReply::json_file(fixture_path(&format!("normals/{name}"))).expect("the fixture is readable")
}

/// The recorded body of `normals/<name>`.
fn fixture_body(name: &str) -> String {
    fs::read_to_string(fixture_path(&format!("normals/{name}"))).expect("the fixture is readable")
}

/// The recorded GSOM rows of one fixture, as a JSON array of objects.
fn recorded_rows(file: &str) -> Vec<serde_json::Value> {
    let body = fixture_body(file);
    serde_json::from_str(&body).expect("the recorded rows are JSON")
}

/// The recorded rows with `keep` deciding which survive, re-encoded as the transport's body.
fn rows_body(rows: &[serde_json::Value]) -> String {
    serde_json::to_string(rows).expect("the rows encode")
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
        Self::at(replies, mode, now(), Config::default())
    }

    /// A run at `start` under `config`.
    fn at(replies: Vec<StubReply>, mode: CacheMode, start: DateTime<Utc>, config: Config) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let cache_dir = directory.path().join("cache");
        fs::create_dir_all(&cache_dir).expect("the cache directory");
        Self::build(directory, cache_dir, replies, mode, start, config)
    }

    /// A run whose *cache root* is `root`, for replaying a warm run's entries offline: config and
    /// data directories are its own scratch, the entries are the warm run's.
    fn replay(root: &std::path::Path, replies: Vec<StubReply>, mode: CacheMode) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        Self::build(
            directory,
            root.to_path_buf(),
            replies,
            mode,
            now(),
            Config::default(),
        )
    }

    /// The one constructor: `holder` keeps the scratch directories alive for the run's lifetime
    /// (or, for a replay, holds nothing the run reads).
    fn build(
        holder: tempfile::TempDir,
        cache_dir: std::path::PathBuf,
        replies: Vec<StubReply>,
        mode: CacheMode,
        start: DateTime<Utc>,
        config: Config,
    ) -> Self {
        let paths = Paths {
            config_dir: holder.path().join("config"),
            config_file: holder.path().join("config/config.toml"),
            keys_file: holder.path().join("config/keys.toml"),
            cache_dir,
            data_dir: holder.path().join("data"),
        };
        let clock: Arc<dyn cirrocast::cache::Clock> = Arc::new(FakeClock::new(start.into()));
        let transport = Arc::new(StubTransport::new(replies));
        let http = HttpClient::new(
            Box::new(Arc::clone(&transport)),
            match mode {
                CacheMode::Offline | CacheMode::NoCache => 0,
                _ => 3,
            },
            Arc::clone(&clock),
            0,
        );
        let cache = Cache::with_root(&paths.cache_dir, mode, Arc::clone(&clock), 0);
        let keys = KeyStore::new(&paths);
        Self {
            _directory: holder,
            http,
            cache,
            config,
            keys,
            transport,
        }
    }

    /// The environment a decoder call runs under.
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

    /// Every request the scripted transport saw, in order.
    fn calls(&self) -> Vec<cirrocast::http::HttpRequest> {
        self.transport.calls()
    }

    /// Decodes the normal for `month` at `loc`.
    fn fetch(
        &self,
        loc: &Location,
        month: u8,
    ) -> cirrocast::error::Result<Option<cirrocast::model::Normals>> {
        cirrocast::normals::fetch(loc, month, &self.env())
    }
}

// ---------------------------------------------------------------------------------------------
// The request shape and the decode
// ---------------------------------------------------------------------------------------------

#[test]
fn the_adapter_requests_the_documented_urls_and_decodes_the_normal() {
    let harness = Harness::new(
        vec![
            fixture("search-beijing.json"),
            fixture("gsom-54511-1991-2020.json"),
        ],
        CacheMode::Normal,
    );
    let normal = harness
        .fetch(&beijing(), 10)
        .expect("the recorded pair decodes")
        .expect("the month has a normal");

    assert_eq!(normal.station, "CHM00054511");
    assert_eq!(normal.station_name, "BEIJING, CH");
    assert_eq!(normal.period, "1991-2020");
    assert_eq!(normal.month, 10);
    assert_eq!(normal.years, 22, "the complete Octobers of the record");
    assert!(
        (normal.distance_km - 11.0819).abs() < 0.01,
        "the nearest entry wins: {}",
        normal.distance_km
    );

    let calls = harness.calls();
    assert_eq!(calls.len(), 2, "one search, one values request");
    assert_eq!(calls[0].url(), SEARCH_BASE);
    assert_eq!(
        calls[0].query_pairs(),
        [
            (
                "dataset".to_owned(),
                "global-summary-of-the-month".to_owned()
            ),
            // The bbox order is the endpoint's trap: north-west corner first, then south-east.
            // Swapping the two corners is an HTTP 500 (measured), and this is the regression gate.
            (
                "bbox".to_owned(),
                "40.4447,115.7028,39.3637,117.1120".to_owned()
            ),
            ("limit".to_owned(), "5".to_owned()),
        ]
        .as_slice()
    );
    assert_eq!(calls[1].url(), DATA_BASE);
    assert_eq!(
        calls[1].query_pairs(),
        [
            (
                "dataset".to_owned(),
                "global-summary-of-the-month".to_owned()
            ),
            ("stations".to_owned(), "CHM00054511".to_owned()),
            ("startDate".to_owned(), "1991-01-01".to_owned()),
            ("endDate".to_owned(), "2020-12-31".to_owned()),
            ("format".to_owned(), "json".to_owned()),
            ("units".to_owned(), "metric".to_owned()),
            ("dataTypes".to_owned(), "TAVG,TMAX,TMIN,PRCP".to_owned()),
        ]
        .as_slice()
    );
}

#[test]
fn the_box_scales_with_the_configured_radius_and_stays_north_west_first() {
    // The box is the radius in degrees — 111 km per degree of latitude, the longitude half-width
    // divided by `cos(lat)` — with the north-west corner first. Swapping the corners answers
    // HTTP 500 (measured 2026-10-06), so both the order and the arithmetic are pinned.
    for (radius, bbox, requests) in [
        (120_u16, "44.1542,-90.8812,41.9920,-87.9212", 2_usize),
        (5, "43.1181,-89.4629,43.0281,-89.3395", 1),
    ] {
        let mut config = Config::default();
        config.normals.max_distance_km = radius;
        let harness = Harness::at(
            vec![fixture("search-madison.json")],
            CacheMode::Normal,
            now(),
            config,
        );
        // The second request has no scripted reply at 120 km, so its failure is expected here;
        // what this test pins is the box and the number of requests the gate let through.
        let _ = harness.fetch(&madison(), 10);
        let calls = harness.calls();
        assert_eq!(
            calls.len(),
            requests,
            "radius {radius}: a station inside it is queried, one beyond it is not"
        );
        let bbox_seen = calls[0]
            .query_pairs()
            .iter()
            .find(|(name, _)| name == "bbox")
            .map(|(_, value)| value.clone());
        assert_eq!(bbox_seen.as_deref(), Some(bbox), "radius {radius}");
    }
}

#[test]
fn the_configured_period_bounds_the_rows_the_mean_uses() {
    // The payload holds 1991–2020; a window that ends in 2019 must average only the rows inside
    // it, because the fetched body is the whole station record and the window is the meaning.
    let mut config = Config::default();
    config.normals.period = "2000-2019".to_owned();
    let harness = Harness::at(
        vec![
            fixture("search-madison.json"),
            fixture("gsom-14837-1991-2020.json"),
        ],
        CacheMode::Normal,
        now(),
        config,
    );
    let normal = harness
        .fetch(&madison(), 10)
        .expect("the recorded pair decodes")
        .expect("a normal");
    assert_eq!(normal.period, "2000-2019");
    assert_eq!(normal.years, 20, "2000 through 2019");

    let calls = harness.calls();
    let range: Vec<String> = calls[1]
        .query_pairs()
        .iter()
        .filter(|(name, _)| name == "startDate" || name == "endDate")
        .map(|(_, value)| value.clone())
        .collect();
    assert_eq!(
        range,
        ["2000-01-01".to_owned(), "2019-12-31".to_owned()],
        "the window travels to the request"
    );
}

#[test]
fn a_full_thirty_year_month_decodes_to_the_hand_averaged_means() {
    let harness = Harness::new(
        vec![
            fixture("search-madison.json"),
            fixture("gsom-14837-1991-2020.json"),
        ],
        CacheMode::Normal,
    );
    let normal = harness
        .fetch(&madison(), 10)
        .expect("the recorded pair decodes")
        .expect("the month has a normal");

    // The values averaged by hand in `tests/fixtures/normals/README.md`: the mean of the 30
    // complete Octobers, in canonical units.
    assert_eq!(normal.years, 30);
    assert!(
        (normal.temp_mean_c - 9.9033).abs() < 0.001,
        "{}",
        normal.temp_mean_c
    );
    assert!(
        (normal.temp_max_c - 15.4933).abs() < 0.001,
        "{}",
        normal.temp_max_c
    );
    assert!(
        (normal.temp_min_c - 4.31).abs() < 0.001,
        "{}",
        normal.temp_min_c
    );
    assert!(
        (normal.precip_mm - 70.28).abs() < 0.001,
        "{}",
        normal.precip_mm
    );
}

#[test]
fn a_year_contributes_only_when_all_four_values_are_present() {
    let harness = Harness::new(
        vec![
            fixture("search-beijing.json"),
            fixture("gsom-54511-1991-2020.json"),
        ],
        CacheMode::Normal,
    );
    let normal = harness
        .fetch(&beijing(), 10)
        .expect("the recorded pair decodes")
        .expect("the month has a normal");

    // The record holds 23 Octobers; `2020-10` carries a precipitation total and no temperatures,
    // so it is not one of the 22 the mean is over.
    assert_eq!(normal.years, 22);
    assert!(
        (normal.temp_mean_c - 14.1).abs() < 0.001,
        "{}",
        normal.temp_mean_c
    );
    assert!(
        (normal.temp_max_c - 19.3636).abs() < 0.001,
        "{}",
        normal.temp_max_c
    );
    assert!(
        (normal.precip_mm - 29.1091).abs() < 0.001,
        "{}",
        normal.precip_mm
    );
}

// ---------------------------------------------------------------------------------------------
// The gates: no station, a thin record, a month the record does not cover
// ---------------------------------------------------------------------------------------------

#[test]
fn a_station_beyond_the_configured_radius_is_no_normal() {
    let mut config = Config::default();
    config.normals.max_distance_km = 5;
    let harness = Harness::at(
        vec![fixture("search-madison.json")],
        CacheMode::Normal,
        now(),
        config,
    );
    assert_eq!(
        harness
            .fetch(&madison(), 10)
            .expect("a radius gate is not an error"),
        None,
        "the nearest station is 8.8 km away"
    );
}

#[test]
fn an_empty_box_is_no_normal_and_makes_no_second_request() {
    let harness = Harness::new(
        vec![fixture("search-empty-pacific.json")],
        CacheMode::Normal,
    );
    assert_eq!(
        harness
            .fetch(&location("Pacific", (0.0, -140.0), Tz::UTC), 10)
            .expect("an empty box is not an error"),
        None
    );
    assert_eq!(harness.calls().len(), 1, "nothing to ask values for");
}

#[test]
fn a_record_thinner_than_twenty_years_is_no_normal() {
    let rows: Vec<serde_json::Value> = recorded_rows("gsom-14837-1991-2020.json")
        .into_iter()
        .filter(|row| {
            let year: u16 = row["DATE"]
                .as_str()
                .expect("a date")
                .get(..4)
                .expect("a year")
                .parse()
                .expect("a year");
            year <= 2002
        })
        .collect();
    let harness = Harness::new(
        vec![
            fixture("search-madison.json"),
            StubReply::ok(200, rows_body(&rows)),
        ],
        CacheMode::Normal,
    );
    assert_eq!(
        harness
            .fetch(&madison(), 10)
            .expect("a gate is not an error"),
        None
    );
}

#[test]
fn a_month_the_record_does_not_cover_is_no_normal() {
    let rows: Vec<serde_json::Value> = recorded_rows("gsom-14837-1991-2020.json")
        .into_iter()
        .filter(|row| row["DATE"].as_str().expect("a date").get(5..7) != Some("06"))
        .collect();
    let harness = Harness::new(
        vec![
            fixture("search-madison.json"),
            StubReply::ok(200, rows_body(&rows)),
        ],
        CacheMode::Normal,
    );
    assert_eq!(
        harness
            .fetch(&madison(), 6)
            .expect("a gate is not an error"),
        None
    );
    // The October read is a different cache key, so it needs its own scripted answer.
    harness.transport.push(StubReply::ok(200, rows_body(&rows)));
    assert_eq!(
        harness
            .fetch(&madison(), 10)
            .expect("the other months are untouched")
            .expect("October survives")
            .years,
        30
    );
}

#[test]
fn a_month_that_is_not_a_calendar_month_is_a_usage_error() {
    let harness = Harness::new(Vec::new(), CacheMode::Normal);
    assert_eq!(
        harness
            .fetch(&beijing(), 13)
            .expect_err("thirteen is not a month")
            .exit_code(),
        2
    );
    assert!(harness.calls().is_empty(), "rejected before any request");
}

// ---------------------------------------------------------------------------------------------
// Cache and offline semantics
// ---------------------------------------------------------------------------------------------

#[test]
fn the_cache_keys_are_the_station_period_and_month_with_a_thirty_day_ttl() {
    let harness = Harness::new(
        vec![
            fixture("search-beijing.json"),
            fixture("gsom-54511-1991-2020.json"),
        ],
        CacheMode::Normal,
    );
    harness
        .fetch(&beijing(), 10)
        .expect("the recorded pair decodes");
    let search = CacheKey::normals_search(BEIJING.0, BEIJING.1, 60);
    let rows = CacheKey::normals_month("CHM00054511", "1991-2020", 10);
    assert_eq!(
        search.path().to_string_lossy(),
        "normals/search-39.90-116.41-60km.json"
    );
    assert_eq!(search.normalised(), "normals|search|39.90|116.41|60");
    assert_eq!(
        rows.path().to_string_lossy(),
        "normals/CHM00054511-1991-2020-10.json"
    );
    assert_eq!(
        rows.normalised(),
        "normals|station|CHM00054511|1991-2020|10"
    );
    for key in [&search, &rows] {
        let entry = harness
            .cache
            .read(key)
            .expect("a read succeeds")
            .expect("the entry is cached");
        assert_eq!(entry.ttl_secs, 30 * 24 * 60 * 60);
    }
}

#[test]
fn the_second_read_is_served_from_the_cache() {
    let harness = Harness::new(
        vec![
            fixture("search-beijing.json"),
            fixture("gsom-54511-1991-2020.json"),
        ],
        CacheMode::Normal,
    );
    let first = harness
        .fetch(&beijing(), 10)
        .expect("the first read")
        .expect("a normal");
    assert_eq!(harness.calls().len(), 2);
    let second = harness
        .fetch(&beijing(), 10)
        .expect("the second read")
        .expect("a normal");
    assert_eq!(
        harness.calls().len(),
        2,
        "the cache served the second read without a request"
    );
    assert_eq!(first, second);
}

#[test]
fn offline_serves_the_cached_normal_and_a_cold_cache_is_no_normal() {
    let warm = Harness::new(
        vec![
            fixture("search-beijing.json"),
            fixture("gsom-54511-1991-2020.json"),
        ],
        CacheMode::Normal,
    );
    warm.fetch(&beijing(), 10)
        .expect("the warm run")
        .expect("a normal");

    // The same cache root, an offline client: the entries are served and the (empty) scripted
    // transport is never consulted.
    let offline = Harness::replay(warm.cache.root(), Vec::new(), CacheMode::Offline);
    let served = offline
        .fetch(&beijing(), 10)
        .expect("the offline read succeeds")
        .expect("the cached normal is served");
    assert_eq!(served.station, "CHM00054511");
    assert_eq!(
        offline.calls().len(),
        0,
        "the offline read made no request of its own"
    );

    // A cold cache is `Ok(None)`: the run loses its comparison, not its exit code.
    let cold = Harness::new(Vec::new(), CacheMode::Offline);
    assert_eq!(cold.fetch(&beijing(), 10).expect("a miss is a note"), None);
    assert_eq!(cold.calls().len(), 0);
}

#[test]
fn an_unparsable_body_is_an_upstream_error() {
    let harness = Harness::new(
        vec![
            fixture("search-beijing.json"),
            StubReply::ok(200, "not json at all"),
        ],
        CacheMode::Normal,
    );
    let error = harness
        .fetch(&beijing(), 10)
        .expect_err("a body that is not JSON is an upstream failure");
    assert_eq!(error.exit_code(), 3);
    assert!(error.to_string().contains("noaa-ncei"), "{error}");
}

// ---------------------------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------------------------

/// A capable terminal, so what renders is the UTF-8 form whatever the test runner's environment.
fn capable() -> TermCaps {
    TermCaps::read(
        |name| match name {
            "TERM" => Some("xterm-256color".to_owned()),
            "LANG" => Some("en_US.UTF-8".to_owned()),
            _ => None,
        },
        true,
    )
}

/// English, the catalog every expectation here is written against.
fn english() -> I18n {
    I18n::load(&LanguageRequest::Tag("en-US".to_owned()), |_| None)
}

/// A context for `report` at `width` under `units` and `color`.
fn context<'a>(
    report: &Report,
    width: usize,
    units: UnitSystem,
    color: ColorMode,
    i18n: &'a I18n,
) -> RenderContext<'a> {
    RenderContext {
        units: units
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color,
        width,
        term: capable(),
        times: common::fixture_times(report),
        lang: i18n.lang(),
        i18n,
        alert_credits: &[],
        aqi_index: cirrocast::air::aqi::AqiIndex::Us,
    }
}

/// Renders `format` for the normals-carrying fixture.
fn render(width: usize, format: Format, units: UnitSystem, color: ColorMode) -> String {
    let report = common::fixture_report("beijing-normals.json");
    let i18n = english();
    renderer_for(format, &capable(), None)
        .expect("the format has a renderer")
        .render(&report, &context(&report, width, units, color, &i18n))
        .expect("the fixture renders")
}

#[test]
fn the_line_renders_within_every_documented_width() {
    for width in [40_usize, 59, 60, 80, 120] {
        let text = render(
            width,
            Format::ArtTable,
            UnitSystem::Metric,
            ColorMode::Never,
        );
        for line in text.lines() {
            assert!(
                line.width() <= width,
                "at {width}: {line:?} is {} columns",
                line.width()
            );
            assert!(!line.contains("NaN"), "{line:?}");
            assert!(!line.contains("inf"), "{line:?}");
        }
        // Every line stays inside the width; the short fragments survive the wrap, the long
        // ones are asserted at the widths where they fit on one line.
        assert!(text.contains("vs normal"), "at {width}: {text}");
        assert!(text.contains("1991–2020"), "at {width}: {text}");
        if width >= 80 {
            assert!(text.contains("high 26.4°C (-2.4°C)"), "at {width}: {text}");
            // The credit line is long enough to wrap at 80; the opening is what is asserted
            // there, the whole sentence in the standalone snapshot.
            assert!(
                text.contains("Climate normals computed from NOAA NCEI"),
                "at {width}: {text}"
            );
        }
    }
}

#[test]
fn the_units_convert_the_normal_and_its_differences() {
    let metric = render(120, Format::ArtTable, UnitSystem::Metric, ColorMode::Never);
    assert!(metric.contains("high 26.4°C (-2.4°C)"), "{metric}");
    assert!(metric.contains("precip 48.9 mm/mo"), "{metric}");

    let us = render(120, Format::ArtTable, UnitSystem::Us, ColorMode::Never);
    assert!(us.contains("high 79.4°F (-4.2°F)"), "{us}");
    assert!(us.contains("low 60.9°F (-3.7°F)"), "{us}");
    assert!(us.contains("precip 1.92 in/mo"), "{us}");
    assert!(
        !us.contains("(+32"),
        "a temperature *difference* must not take Fahrenheit's offset: {us}"
    );
}

#[test]
fn a_colourless_run_keeps_the_signs_and_a_coloured_one_paints_them() {
    let plain = render(120, Format::ArtTable, UnitSystem::Metric, ColorMode::Never);
    assert!(!plain.contains('\u{1b}'), "{plain}");
    assert!(
        plain.contains("(-2.4°C)") && plain.contains("(-2.1°C)"),
        "{plain}"
    );

    let coloured = render(120, Format::ArtTable, UnitSystem::Metric, ColorMode::Always);
    assert!(coloured.contains("\u{1b}[38;5;"), "{coloured}");
    // The signs survive the paint: the escape wraps the text, it does not replace it.
    assert!(coloured.contains("-2.4°C"), "{coloured}");
}

#[test]
fn the_standalone_view_names_the_station_the_period_and_the_credit() {
    let text = render(200, Format::Normals, UnitSystem::Metric, ColorMode::Never);
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("Beijing, Beijing, China (39.90, 116.41) Asia/Shanghai"),
        "{text}"
    );
    assert_eq!(
        lines.next(),
        Some(
            "Climate normals: 1991\u{2013}2020 · BEIJING, CH (CHM00054511) 11 km · high 26.4°C (-2.4°C) · low 16.1°C (-2.1°C) · precip 48.9 mm/mo (-100%) · 22 years"
        ),
        "{text}"
    );
    assert_eq!(
        lines.next(),
        Some("Climate normals computed from NOAA NCEI Global Summary of the Month (public domain)"),
        "{text}"
    );
    assert_eq!(lines.next(), None, "{text}");
}

#[test]
fn a_report_without_a_normal_says_so_in_the_standalone_view_only() {
    let report = common::fixture_report("beijing-1d.json");
    let i18n = english();
    let ctx = context(&report, 80, UnitSystem::Metric, ColorMode::Never, &i18n);
    assert_eq!(
        renderer_for(Format::Normals, &capable(), None)
            .expect("the format has a renderer")
            .render(&report, &ctx)
            .expect("the fixture renders"),
        "climate normals unavailable"
    );
    let table = renderer_for(Format::ArtTable, &capable(), None)
        .expect("the format has a renderer")
        .render(&report, &ctx)
        .expect("the fixture renders");
    assert!(
        !table.contains("normal"),
        "no comparison line without a normal: {table}"
    );
}

#[test]
fn the_plain_records_are_greppable_and_carry_the_credit() {
    let text = render(80, Format::Plain, UnitSystem::Metric, ColorMode::Never);
    assert!(
        text.contains(
            "climate_normals: 1991\u{2013}2020 · BEIJING, CH (CHM00054511) 11 km · high 26.4°C (-2.4°C) · low 16.1°C (-2.1°C) · precip 48.9 mm/mo (-100%) · 22 years"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "Climate normals computed from NOAA NCEI Global Summary of the Month (public domain)"
        ),
        "{text}"
    );
}

// ---------------------------------------------------------------------------------------------
// The CLI over a seeded normals cache
// ---------------------------------------------------------------------------------------------

/// The fixture body of `normals/<name>`.
fn normals_fixture(name: &str) -> String {
    fixture_body(name)
}

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

/// Seeds the weather report for the Beijing coordinates and, with `normals`, the two normals
/// entries the search resolves to. The weather recording's days are 2026-07-15..17, so the normal
/// the run asks for is July's.
fn seed(sandbox: &Sandbox, normals: bool) {
    let weather = fs::read_to_string(fixture_path("open_meteo/forecast_beijing_2026-07-15.json"))
        .expect("the weather fixture is readable");
    common::seed_weather(
        sandbox,
        "open-meteo",
        BEIJING.0,
        BEIJING.1,
        3,
        Tz::UTC,
        &weather,
    );
    if normals {
        seed_entry(
            sandbox,
            &CacheKey::normals_search(BEIJING.0, BEIJING.1, 60),
            &normals_fixture("search-beijing.json"),
            Utc::now(),
            30 * 24 * 60 * 60,
        );
        seed_entry(
            sandbox,
            &CacheKey::normals_month("CHM00054511", "1991-2020", 7),
            &normals_fixture("gsom-54511-1991-2020.json"),
            Utc::now(),
            30 * 24 * 60 * 60,
        );
    }
}

/// A successful offline run's stdout, against the coordinate location.
fn offline(sandbox: &Sandbox, args: &[&str]) -> String {
    let assert = sandbox
        .cirrocast()
        .env("TERM", "xterm-256color")
        .args(args)
        .args(["--offline", "--no-alerts", "--lang", "en-US"])
        .arg("@39.9042,116.4074")
        .assert()
        .success();
    String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8")
}

#[test]
fn a_run_without_the_flag_never_asks_ncei() {
    let sandbox = Sandbox::new();
    seed(&sandbox, false);
    // Under `CIRROCAST_FORBID_NETWORK=1` a normals request would fail the run; a clean plain run
    // is the observable proof that none was made.
    let plain = offline(&sandbox, &["-f", "plain"]);
    assert!(plain.contains("location: "), "{plain}");
    assert!(!plain.contains("climate_normals"), "{plain}");
    assert!(
        !sandbox.cache_dir().join("normals").exists(),
        "no normals namespace is even created"
    );
}

#[test]
fn a_seeded_normal_renders_in_every_surface() {
    let sandbox = Sandbox::new();
    seed(&sandbox, true);

    let (plain, table, standalone, json) = (
        offline(&sandbox, &["--normals", "-f", "plain"]),
        offline(&sandbox, &["--normals"]),
        offline(&sandbox, &["--normals", "-f", "normals"]),
        offline(&sandbox, &["--normals", "-f", "json"]),
    );

    // July is the month the seeded weather recording's days are in, and its normal is the mean of
    // the record's 22 complete Julys (27.2591 °C mean, 31.6955 °C high, 22.8318 °C low,
    // 167.2455 mm).
    assert!(
        plain.contains("climate_normals: 1991\u{2013}2020 · BEIJING, CH (CHM00054511)"),
        "{plain}"
    );
    assert!(plain.contains("22 years"), "{plain}");
    assert!(
        plain.contains(
            "Climate normals computed from NOAA NCEI Global Summary of the Month (public domain)"
        ),
        "{plain}"
    );
    assert!(table.contains("vs normal 1991\u{2013}2020:"), "{table}");
    assert!(table.contains("high 31.7°C"), "{table}");
    assert!(
        standalone.contains("Climate normals: 1991\u{2013}2020 ·"),
        "{standalone}"
    );

    let document: serde_json::Value = serde_json::from_str(&json).expect("the document is JSON");
    assert_eq!(
        document["normals"]["station"],
        serde_json::json!("CHM00054511")
    );
    assert_eq!(
        document["normals"]["period"],
        serde_json::json!("1991-2020")
    );
    assert_eq!(document["normals"]["month"], serde_json::json!(7));
    assert_eq!(document["normals"]["years"], serde_json::json!(22));
    let distance = document["normals"]["distance_km"]
        .as_f64()
        .expect("a number");
    assert!(
        (distance - 11.0819).abs() < 0.01,
        "the computed distance from the run's own point: {distance}"
    );
}

#[test]
fn the_verbose_gates_explain_themselves_offline() {
    // A cold normals cache under `--offline`: the run keeps its forecast, says why the comparison
    // is missing and exits 0.
    let sandbox = Sandbox::new();
    seed(&sandbox, false);
    let assert = sandbox
        .cirrocast()
        .env("TERM", "xterm-256color")
        .args([
            "--normals",
            "-v",
            "--offline",
            "--no-alerts",
            "--lang",
            "en-US",
        ])
        .arg("@39.9042,116.4074")
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
    assert!(
        stderr.contains("normals: offline: no cached noaa-ncei station search"),
        "{stderr}"
    );

    // A station the search resolves, whose record is too thin for the requested month: the note
    // names the count and the threshold.
    let sandbox = Sandbox::new();
    seed(&sandbox, true);
    let thin: Vec<serde_json::Value> = recorded_rows("gsom-54511-1991-2020.json")
        .into_iter()
        .filter(|row| {
            let year: u16 = row["DATE"]
                .as_str()
                .expect("a date")
                .get(..4)
                .expect("a year")
                .parse()
                .expect("a year");
            year <= 2002
        })
        .collect();
    seed_entry(
        &sandbox,
        &CacheKey::normals_month("CHM00054511", "1991-2020", 7),
        &rows_body(&thin),
        Utc::now(),
        30 * 24 * 60 * 60,
    );
    let assert = sandbox
        .cirrocast()
        .env("TERM", "xterm-256color")
        .args([
            "--normals",
            "-v",
            "--offline",
            "--no-alerts",
            "--lang",
            "en-US",
        ])
        .arg("@39.9042,116.4074")
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
    assert!(
        stderr.contains(
            "normals: only 12 usable years for month 7 at CHM00054511 in 1991-2020; a normal needs 20"
        ),
        "{stderr}"
    );
}

#[test]
fn the_defaults_normals_key_turns_the_comparison_on_for_every_run() {
    let sandbox = Sandbox::new();
    seed(&sandbox, true);
    sandbox.write_config("schema_version = 2\n\n[defaults]\nnormals = true\n");
    let plain = offline(&sandbox, &["-f", "plain"]);
    assert!(
        plain.contains("climate_normals: 1991\u{2013}2020 · BEIJING, CH"),
        "{plain}"
    );
}
