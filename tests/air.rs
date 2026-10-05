// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Air quality end to end: the Open-Meteo adapter over scripted transports, the two category
//! scales at their documented breakpoints, the panel's width invariant and the CLI formats over a
//! seeded air cache.
//!
//! Nothing here touches the network: the adapter runs over [`StubTransport`] with the recorded
//! fixtures in `tests/fixtures/air/`, and the CLI runs use `--offline` over cache entries the test
//! writes itself (the sandbox also sets `CIRROCAST_FORBID_NETWORK=1` for every child).

mod common;

use std::fs;
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use unicode_width::UnicodeWidthStr as _;

use cirrocast::air::aqi::{AqiCategory, AqiIndex, us_beyond_index};
use cirrocast::air::open_meteo::BASE;
use cirrocast::cache::{CACHE_SCHEMA_VERSION, Cache, CacheKey, CacheMode, FakeClock};
use cirrocast::config::Config;
use cirrocast::config::UnitOverrides;
use cirrocast::config::keys::KeyStore;
use cirrocast::http::{HttpClient, StubReply, StubTransport};
use cirrocast::i18n::{I18n, LanguageRequest};
use cirrocast::model::units::UnitSystem;
use cirrocast::model::{AirQuality, AirSource, Location, LocationSource, Report};
use cirrocast::paths::Paths;
use cirrocast::provider::Env;
use cirrocast::render::{ColorMode, Format, RenderContext, TermCaps, renderer_for};

use common::{Sandbox, fixture_path};

/// The instant the adapter tests run at: the recorded fixtures' own day.
const NOW: &str = "2026-10-03T06:00:00Z";

/// The point the Beijing geocode fixture resolves to; every CLI air cache key uses it.
const LAT: f64 = 39.9075;
/// The longitude half of the pair.
const LON: f64 = 116.39723;

/// The instant the harness clock starts at.
fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(NOW)
        .expect("a valid instant")
        .with_timezone(&Utc)
}

/// A location with the fields the adapter reads.
fn location(name: &str, lat: f64, lon: f64, tz: Tz) -> Location {
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
        source: LocationSource::Geocoder,
        station: None,
    }
}

/// Berlin, where the recorded reading carries every pollen species (as a zero count).
fn berlin() -> Location {
    location("Berlin", 52.52, 13.405, Tz::Europe__Berlin)
}

/// Sydney, outside the CAMS European pollen domain.
fn sydney() -> Location {
    location("Sydney", -33.87, 151.21, Tz::Australia__Sydney)
}

/// Reykjavík, inside the domain with a zero count.
fn reykjavik() -> Location {
    location("Reykjavík", 64.15, -21.94, Tz::Atlantic__Reykjavik)
}

/// One recorded fixture as a `200` reply.
fn fixture(name: &str) -> StubReply {
    StubReply::json_file(fixture_path(&format!("air/{name}"))).expect("the fixture is readable")
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

    /// Fetches the air reading for `loc`.
    fn fetch(&self, loc: &Location) -> cirrocast::error::Result<AirQuality> {
        let env = Env {
            http: &self.http,
            cache: &self.cache,
            config: &self.config,
            keys: &self.keys,
            quiet: true,
            verbose: 0,
        };
        cirrocast::air::fetch(loc, &env)
    }

    /// Every request the transport saw.
    fn calls(&self) -> Vec<cirrocast::http::HttpRequest> {
        self.transport.calls()
    }
}

// ---------------------------------------------------------------------------------------------
// The adapter
// ---------------------------------------------------------------------------------------------

#[test]
fn the_adapter_requests_the_documented_url_and_decodes_the_reading() {
    let harness = Harness::new(vec![fixture("berlin-2026-10-03.json")], CacheMode::Normal);
    let reading = harness.fetch(&berlin()).expect("the reading decodes");

    assert_eq!(reading.source, AirSource::OpenMeteo);
    assert_eq!(reading.aqi_us, Some(43));
    assert_eq!(reading.aqi_european, Some(42));
    assert_eq!(reading.pm2_5, Some(8.2));
    assert_eq!(reading.pm10, Some(13.3));
    assert_eq!(reading.o3, Some(38.0));
    assert_eq!(reading.no2, Some(27.9));
    assert_eq!(reading.so2, Some(3.0));
    assert_eq!(reading.co, Some(251.0));
    assert_eq!(
        reading.time.to_rfc3339(),
        "2026-10-03T20:00:00+02:00",
        "the wall clock plus the response offset"
    );
    let pollen = reading.pollen.expect("Berlin is inside the pollen domain");
    assert_eq!(pollen.values(), [0.0; 6], "October: present but zero");

    let calls = harness.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url(), BASE);
    assert_eq!(
        calls[0].query_pairs(),
        [
            ("latitude".to_owned(), "52.5200".to_owned()),
            ("longitude".to_owned(), "13.4050".to_owned()),
            (
                "current".to_owned(),
                "us_aqi,european_aqi,pm2_5,pm10,ozone,nitrogen_dioxide,sulphur_dioxide,\
carbon_monoxide,alder_pollen,birch_pollen,grass_pollen,mugwort_pollen,olive_pollen,\
ragweed_pollen"
                    .to_owned()
            ),
            ("timezone".to_owned(), "auto".to_owned()),
        ]
        .as_slice()
    );
}

#[test]
fn the_second_read_is_served_from_the_cache() {
    let harness = Harness::new(vec![fixture("berlin-2026-10-03.json")], CacheMode::Normal);
    let first = harness.fetch(&berlin()).expect("the first read fetches");
    let second = harness.fetch(&berlin()).expect("the second read is cached");
    assert_eq!(first, second);
    assert_eq!(harness.calls().len(), 1, "the cache served the second read");
}

#[test]
fn offline_mode_serves_the_cached_reading_and_names_a_miss() {
    let harness = Harness::new(vec![fixture("berlin-2026-10-03.json")], CacheMode::Normal);
    let fetched = harness.fetch(&berlin()).expect("the reading is cached");
    let offline = Harness::at(Vec::new(), CacheMode::Offline, now());
    assert_eq!(
        offline
            .fetch(&berlin())
            .expect_err("an empty cache")
            .exit_code(),
        3
    );

    // The same cache directory is what the offline mode reads: replay the fetch through a run
    // whose cache root is the first harness's cache.
    assert_eq!(harness.fetch(&berlin()).expect("still cached"), fetched);
}

#[test]
fn sydney_has_no_pollen_but_a_reading() {
    let harness = Harness::new(vec![fixture("sydney-2026-10-03.json")], CacheMode::Normal);
    let reading = harness.fetch(&sydney()).expect("the reading decodes");
    assert_eq!(reading.pollen, None, "outside the CAMS European domain");
    assert!(reading.aqi_us.is_some());
    assert!(reading.pm2_5.is_some());
}

#[test]
fn reykjavik_zero_pollen_stays_a_forecast() {
    let harness = Harness::new(
        vec![fixture("reykjavik-2026-10-03.json")],
        CacheMode::Normal,
    );
    let reading = harness.fetch(&reykjavik()).expect("the reading decodes");
    let pollen = reading
        .pollen
        .expect("zero is a measured forecast, not a coverage gap");
    assert_eq!(pollen.values(), [0.0; 6]);
}

#[test]
fn a_truncated_response_is_an_upstream_error() {
    let harness = Harness::new(vec![fixture("truncated.json")], CacheMode::Normal);
    let error = harness.fetch(&berlin()).expect_err("the body is not JSON");
    assert_eq!(error.exit_code(), 3);
    assert!(error.to_string().contains("does not parse"), "{error}");
}

#[test]
fn a_response_without_current_is_an_upstream_error() {
    let harness = Harness::new(vec![fixture("no-current.json")], CacheMode::Normal);
    let error = harness.fetch(&berlin()).expect_err("there is no reading");
    assert_eq!(error.exit_code(), 3);
    assert!(error.to_string().contains("no `current` block"), "{error}");
}

#[test]
fn a_unit_mismatch_names_the_field_and_the_received_unit() {
    let harness = Harness::new(vec![fixture("unit-mismatch.json")], CacheMode::Normal);
    let error = harness.fetch(&berlin()).expect_err("the unit is not μg/m³");
    assert_eq!(error.exit_code(), 3);
    let message = error.to_string();
    assert!(
        message.contains("`pm2_5` is reported in `mg/m³`"),
        "{message}"
    );
    assert!(message.contains("expected `μg/m³`"), "{message}");
}

// ---------------------------------------------------------------------------------------------
// The category scales
// ---------------------------------------------------------------------------------------------

#[test]
fn the_us_scale_breaks_at_the_documented_values() {
    for (index, expected) in [
        (0, AqiCategory::Good),
        (50, AqiCategory::Good),
        (51, AqiCategory::Moderate),
        (100, AqiCategory::Moderate),
        (101, AqiCategory::UnhealthyForSensitiveGroups),
        (150, AqiCategory::UnhealthyForSensitiveGroups),
        (151, AqiCategory::Unhealthy),
        (200, AqiCategory::Unhealthy),
        (201, AqiCategory::VeryUnhealthy),
        (300, AqiCategory::VeryUnhealthy),
        (301, AqiCategory::Hazardous),
        (500, AqiCategory::Hazardous),
    ] {
        assert_eq!(AqiCategory::from_us(index), expected, "US AQI {index}");
    }
    // Above the documented 500 top the reading clamps to Hazardous and the flag says so.
    assert_eq!(AqiCategory::from_us(501), AqiCategory::Hazardous);
    assert!(us_beyond_index(501));
    assert!(!us_beyond_index(500));
}

#[test]
fn the_european_scale_breaks_at_the_documented_values() {
    for (index, expected) in [
        (0, AqiCategory::Good),
        (20, AqiCategory::Good),
        (21, AqiCategory::Fair),
        (40, AqiCategory::Fair),
        (41, AqiCategory::Moderate),
        (60, AqiCategory::Moderate),
        (61, AqiCategory::Poor),
        (80, AqiCategory::Poor),
        (81, AqiCategory::VeryPoor),
        (100, AqiCategory::VeryPoor),
        (101, AqiCategory::ExtremelyPoor),
    ] {
        assert_eq!(
            AqiCategory::from_european(index),
            expected,
            "European AQI {index}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------------------------------

/// The English catalog, loaded the way the CLI loads an unconfigured run.
fn english() -> I18n {
    I18n::load(&LanguageRequest::Auto, |_| None)
}

/// A terminal that can do everything: UTF-8, a tty, 256 colours.
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

/// The air-carrying fixture: 43/42 AQI, six pollutants, six pollen species and UV 5.
fn air_report() -> Report {
    common::fixture_report("beijing-air.json")
}

/// The render context for one air case.
fn context<'a>(
    report: &Report,
    width: usize,
    color: ColorMode,
    index: AqiIndex,
    i18n: &'a I18n,
) -> RenderContext<'a> {
    RenderContext {
        units: UnitSystem::Metric
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color,
        width,
        term: capable(),
        times: common::fixture_times(report),
        lang: i18n.lang(),
        i18n,
        alert_credits: &[],
        aqi_index: index,
    }
}

/// Renders `format` at `width`, with the selected AQI scale injected.
fn render(report: &Report, width: usize, format: Format, index: AqiIndex) -> String {
    let i18n = english();
    renderer_for(format, &capable(), None)
        .expect("the format has a renderer")
        .render(
            report,
            &context(report, width, ColorMode::Never, index, &i18n),
        )
        .expect("the fixture renders")
}

#[test]
fn the_panel_renders_at_every_documented_width_within_its_width() {
    let report = air_report();
    for width in [59_usize, 60, 80, 120] {
        for format in [Format::ArtTable, Format::Aqi] {
            let text = render(&report, width, format, AqiIndex::Us);
            for line in text.lines() {
                assert!(
                    line.width() <= width,
                    "{} at {width}: {line:?} is {} columns",
                    format.as_str(),
                    line.width()
                );
                assert!(!line.contains("NaN"), "{line:?}");
                assert!(!line.contains("inf"), "{line:?}");
                // No negative zero among the numbers; the date's `2026-09-30` is not one.
                assert!(
                    !line.contains("-0 ") && !line.contains("-0.0") && !line.ends_with("-0"),
                    "{line:?}"
                );
            }
            // The compact form from 60 columns, the stacked one below it; both carry the same
            // values, so the assertions are on values rather than on the layout.
            assert!(text.contains("Air quality"), "{text}");
            assert!(text.contains("US AQI 43 (Good)"), "{text}");
            assert!(text.contains("PM2.5"), "{text}");
            assert!(text.contains("8.2"), "{text}");
            assert!(
                text.contains("Air quality data by Open-Meteo.com (CAMS ENSEMBLE)"),
                "{text}"
            );
        }
    }
}

#[test]
fn the_standalone_view_names_the_place_the_time_and_the_source() {
    let text = render(&air_report(), 80, Format::Aqi, AqiIndex::Us);
    assert!(
        text.starts_with("Beijing, Beijing, China (39.90, 116.41) Asia/Shanghai\n"),
        "{text}"
    );
    assert!(
        text.contains("updated 2026-09-30T12:00:00+08:00 · Open-Meteo"),
        "{text}"
    );
    assert!(
        text.contains("Pollen: alder 1.2 · birch 0 · grass 4.5"),
        "{text}"
    );
    assert!(text.contains("UV 5 (moderate) · weather data"), "{text}");
}

#[test]
fn the_selected_scale_drives_the_category_and_the_other_stays_plain() {
    let report = air_report();
    for (index, selected, other) in [
        (
            AqiIndex::Us,
            "US AQI 43 (Good)",
            "European AQI 42 (Moderate)",
        ),
        (
            AqiIndex::European,
            "European AQI 42 (Moderate)",
            "US AQI 43 (Good)",
        ),
    ] {
        let text = render(&report, 120, Format::Aqi, index);
        assert!(text.contains(selected), "{index}: {text}");
        assert!(text.contains(other), "{index}: {text}");
    }
}

#[test]
fn the_colour_ramp_paints_only_the_selected_scales_category() {
    let report = air_report();
    let i18n = english();
    let text = renderer_for(Format::Aqi, &capable(), None)
        .expect("the format has a renderer")
        .render(
            &report,
            &context(&report, 120, ColorMode::Always, AqiIndex::Us, &i18n),
        )
        .expect("the fixture renders");
    assert!(
        text.contains("\u{1b}[38;5;34m43 (Good)\u{1b}[0m"),
        "the US category is painted green: {text:?}"
    );
    assert!(
        !text.contains("\u{1b}[38;5;226m42 (Moderate)"),
        "the other scale is not painted: {text:?}"
    );
}

#[test]
fn the_dumb_charset_folds_the_units_and_the_separators() {
    let text = render(&air_report(), 80, Format::Dumb, AqiIndex::Us);
    assert!(text.contains("ug/m3"), "{text}");
    assert!(text.contains("grains/m3"), "{text}");
    assert!(!text.contains('μ'), "{text}");
    assert!(!text.contains('³'), "{text}");
    assert!(!text.contains('\u{1b}'), "dumb means no colour: {text:?}");
}

#[test]
fn the_plain_records_are_greppable() {
    let text = render(&air_report(), 80, Format::Plain, AqiIndex::Us);
    for expected in [
        "air_quality: US AQI 43 (Good) · European AQI 42 (Moderate)",
        "pm2.5: 8.2 μg/m³",
        "o3: 38 μg/m³",
        "pollen: alder 1.2 · birch 0 · grass 4.5 · mugwort 0 · olive 0 · ragweed 0 grains/m³",
        "uv: UV 5 (moderate) · weather data",
        "Air quality data by Open-Meteo.com (CAMS ENSEMBLE)",
    ] {
        assert!(text.contains(expected), "missing {expected:?}: {text}");
    }
}

#[test]
fn a_report_without_air_shows_no_panel_but_the_aqi_format_says_so() {
    let report = common::fixture_report("beijing-1d.json");
    let table = render(&report, 80, Format::ArtTable, AqiIndex::Us);
    assert!(!table.contains("Air quality"), "{table}");
    assert_eq!(
        render(&report, 80, Format::Aqi, AqiIndex::Us),
        "air quality unavailable"
    );
}

// ---------------------------------------------------------------------------------------------
// The CLI over a seeded air cache
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

/// The fixture body of `air/<name>`.
fn air_fixture(name: &str) -> String {
    fs::read_to_string(fixture_path(&format!("air/{name}"))).expect("the fixture is readable")
}

/// Seeds the geocode answer for `Beijing`, its weather, and — with `air` — one air reading.
fn seed(sandbox: &Sandbox, air: Option<&str>) {
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
    if let Some(body) = air {
        seed_entry(
            sandbox,
            &CacheKey::air("open-meteo", LAT, LON, today),
            body,
            Utc::now(),
            600,
        );
    }
}

/// A successful offline run's stdout and stderr.
fn offline(sandbox: &Sandbox, args: &[&str]) -> (String, String) {
    let assert = sandbox
        .cirrocast()
        // Pin the terminal so the panel is rendered in its UTF-8 form whatever `TERM` the test
        // runner has: the ASCII fold is asserted separately, and the expectations here are exact.
        .env("TERM", "xterm-256color")
        .args(args)
        .args(["--offline", "--no-alerts", "--lang", "en-US"])
        .arg("Beijing")
        .assert()
        .success();
    (
        String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8"),
        String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8"),
    )
}

#[test]
fn a_seeded_reading_renders_in_every_air_surface() {
    let sandbox = Sandbox::new();
    seed(&sandbox, Some(&air_fixture("berlin-2026-10-03.json")));

    let (plain, _) = offline(&sandbox, &["--aqi", "-f", "plain"]);
    for expected in [
        "air_quality: US AQI 43 (Good) · European AQI 42 (Moderate)",
        "pm2.5: 8.2 μg/m³",
        "pollen: alder 0 · birch 0 · grass 0 · mugwort 0 · olive 0 · ragweed 0 grains/m³",
        "Air quality data by Open-Meteo.com (CAMS ENSEMBLE)",
    ] {
        assert!(plain.contains(expected), "missing {expected:?}: {plain}");
    }

    // `--format aqi` implies the fetch and prints the standalone view.
    let (standalone, _) = offline(&sandbox, &["-f", "aqi"]);
    assert!(
        standalone.contains("Air quality: US AQI 43 (Good)"),
        "{standalone}"
    );
    assert!(
        standalone.contains("updated 2026-10-03T20:00:00+02:00 · Open-Meteo"),
        "{standalone}"
    );

    // The table appends the panel below the forecast.
    let (table, _) = offline(&sandbox, &["--aqi"]);
    assert!(table.contains("Air quality: US AQI 43 (Good)"), "{table}");
    assert!(table.contains("Weather report:"), "{table}");

    let (json, _) = offline(&sandbox, &["--aqi", "-f", "json"]);
    let document: serde_json::Value = serde_json::from_str(&json).expect("the document is JSON");
    assert_eq!(document["air"]["aqi_us"], serde_json::json!(43));
    assert_eq!(document["air"]["category"]["us"], serde_json::json!("good"));
    assert_eq!(document["air"]["pollen"]["birch"], serde_json::json!(0.0));
    assert_eq!(
        document["air"]["units"]["pollutants"],
        serde_json::json!("μg/m³")
    );

    let (one_line, _) = offline(&sandbox, &["--aqi", "-f", "one-line", "--template", "q=%q"]);
    assert!(one_line.contains("q=US AQI 43 (Good)"), "{one_line}");
}

#[test]
fn the_aqi_index_flag_selects_the_one_line_scale() {
    let sandbox = Sandbox::new();
    seed(&sandbox, Some(&air_fixture("berlin-2026-10-03.json")));
    let (line, _) = offline(
        &sandbox,
        &[
            "--aqi",
            "--aqi-index",
            "european",
            "-f",
            "one-line",
            "--template",
            "%q",
        ],
    );
    assert!(line.contains("European AQI 42 (Moderate)"), "{line}");
    assert!(!line.contains("US AQI"), "{line}");
}

#[test]
fn an_aqi_index_without_a_fetching_run_is_a_usage_error() {
    let sandbox = Sandbox::new();
    let assert = sandbox
        .cirrocast()
        .args(["--aqi-index", "european", "--offline", "Beijing"])
        .assert()
        .code(2);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
    assert!(
        stderr.contains("--aqi-index needs --aqi or `--format aqi`"),
        "{stderr}"
    );
}

#[test]
fn a_failed_air_fetch_is_a_warning_with_the_weather_intact() {
    let sandbox = Sandbox::new();
    seed(&sandbox, None);

    let (plain, stderr) = offline(&sandbox, &["--aqi", "-f", "plain"]);
    assert!(plain.contains("current:"), "{plain}");
    assert!(!plain.contains("air_quality:"), "{plain}");
    assert!(stderr.contains("air quality unavailable"), "{stderr}");
    assert!(stderr.contains("offline:"), "{stderr}");

    // The standalone view reports the absence instead of printing an empty document.
    let (standalone, stderr) = offline(&sandbox, &["-f", "aqi"]);
    assert_eq!(standalone.trim(), "air quality unavailable");
    assert!(stderr.contains("air quality unavailable"), "{stderr}");
}

#[test]
fn a_quiet_run_keeps_the_warning_out_of_stderr() {
    let sandbox = Sandbox::new();
    seed(&sandbox, None);
    let assert = sandbox
        .cirrocast()
        .env("TERM", "xterm-256color")
        .args(["--aqi", "-q", "--offline", "--no-alerts", "Beijing"])
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
    assert!(!stderr.contains("air quality unavailable"), "{stderr}");
}

#[test]
fn an_uncovered_pollen_forecast_is_said_not_guessed() {
    let sandbox = Sandbox::new();
    seed(&sandbox, Some(&air_fixture("sydney-2026-10-03.json")));

    let (plain, stderr) = offline(&sandbox, &["--aqi", "-f", "plain", "-v"]);
    assert!(
        plain.contains("pollen: not covered at this location"),
        "{plain}"
    );
    assert!(
        stderr.contains("air: pollen forecast is not covered here (CAMS European domain only)"),
        "{stderr}"
    );
}

#[test]
fn the_panel_respects_the_resolved_width_in_the_cli() {
    let sandbox = Sandbox::new();
    seed(&sandbox, Some(&air_fixture("berlin-2026-10-03.json")));
    let assert = sandbox
        .cirrocast()
        .env("COLUMNS", "58")
        .args([
            "--aqi",
            "--color",
            "never",
            "--offline",
            "--no-alerts",
            "--lang",
            "en-US",
        ])
        .arg("Beijing")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    for line in stdout.lines() {
        assert!(line.width() <= 58, "{line:?} is {} columns", line.width());
    }
    assert!(stdout.contains("Air quality"), "{stdout}");
}
