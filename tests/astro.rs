// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Astronomy end to end: the recorded fixtures, the book instants, the transition and polar days,
//! the moonrise references, and the CLI surfaces over a seeded cache.
//!
//! Nothing here touches the network. The sunrise/sunset rows are the recorded Open-Meteo archive
//! payloads under `tests/fixtures/astro/`; the moon instants, the Meeus book instants and the
//! `PyEphem` moonrise references are tables in this file, with their provenance written down in
//! `tests/fixtures/astro/README.md`. The CLI runs use `--offline` over cache entries this test
//! writes itself, in a sandbox that forbids the network for every child process.

mod common;

use std::fs;

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use chrono_tz::Tz;

use cirrocast::astro;
use cirrocast::cache::{CACHE_SCHEMA_VERSION, CacheKey};
use cirrocast::config::UnitOverrides;
use cirrocast::i18n::{I18n, LanguageRequest};
use cirrocast::model::astro::{MoonPhase, Polar, SunSource};
use cirrocast::model::units::UnitSystem;
use cirrocast::model::{Astro, Location, LocationSource, Report};
use cirrocast::render::{ColorMode, Format, RenderContext, TermCaps, renderer_for};

use common::{Sandbox, fixture, fixture_path};

/// One recorded moon instant: UTC instant, Horizons' illuminated percentage, expected phase.
const MOON_FIXTURES: [(&str, f64, MoonPhase); 8] = [
    ("2026-01-18T12:00:00Z", 0.20848, MoonPhase::New),
    ("2026-01-22T16:00:00Z", 15.00160, MoonPhase::WaxingCrescent),
    ("2026-01-26T00:00:00Z", 47.87048, MoonPhase::FirstQuarter),
    ("2026-01-31T00:00:00Z", 94.97307, MoonPhase::WaxingGibbous),
    ("2026-02-02T00:00:00Z", 99.95555, MoonPhase::Full),
    ("2026-02-04T04:00:00Z", 93.95135, MoonPhase::WaningGibbous),
    ("2026-02-09T00:00:00Z", 55.15727, MoonPhase::LastQuarter),
    ("2026-02-12T12:00:00Z", 23.44829, MoonPhase::WaningCrescent),
];

/// The two Meeus book instants: the TD Julian Ephemeris Day, the phase and the example.
const BOOK_INSTANTS: [(f64, MoonPhase, &str); 2] = [
    (2_443_192.651_18, MoonPhase::New, "example 49.a (1977)"),
    (
        2_467_636.491_86,
        MoonPhase::LastQuarter,
        "example 49.b (2044)",
    ),
];

/// One recorded sun day: fixture file, point, zone and the local date it records.
const SUN_CASES: [(&str, f64, f64, &str, &str); 5] = [
    (
        "open-meteo-ny-2026-03-08.json",
        40.7128,
        -74.0060,
        "America/New_York",
        "2026-03-08",
    ),
    (
        "open-meteo-ny-2025-03-09.json",
        40.7128,
        -74.0060,
        "America/New_York",
        "2025-03-09",
    ),
    (
        "open-meteo-longyearbyen-2026-06-21.json",
        78.2232,
        15.6469,
        "Arctic/Longyearbyen",
        "2026-06-21",
    ),
    (
        "open-meteo-longyearbyen-2025-12-21.json",
        78.2232,
        15.6469,
        "Arctic/Longyearbyen",
        "2025-12-21",
    ),
    (
        "open-meteo-beijing-2024-02-29.json",
        39.9042,
        116.4074,
        "Asia/Shanghai",
        "2024-02-29",
    ),
];

/// One `PyEphem` moonrise/moonset reference: point, zone, date, rise and set (`None` = the event
/// happens on another local day).
#[allow(clippy::type_complexity)]
const MOONRISE_REFERENCES: [(f64, f64, &str, &str, Option<&str>, Option<&str>); 3] = [
    (
        39.9042,
        116.4074,
        "Asia/Shanghai",
        "2026-10-01",
        Some("2026-10-01T20:28:27"),
        Some("2026-10-01T11:24:16"),
    ),
    (
        40.7128,
        -74.0060,
        "America/New_York",
        "2026-03-08",
        None,
        Some("2026-03-08T09:22:58"),
    ),
    (
        78.2232,
        15.6469,
        "Arctic/Longyearbyen",
        "2026-06-21",
        Some("2026-06-21T12:28:05"),
        Some("2026-06-21T01:18:07"),
    ),
];

// ---------------------------------------------------------------------------------------------
// Small readers
// ---------------------------------------------------------------------------------------------

/// An RFC 3339 instant, as UTC.
fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|error| panic!("{text}: {error}"))
        .with_timezone(&Utc)
}

/// A `YYYY-MM-DD` date.
fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap_or_else(|error| panic!("{text}: {error}"))
}

/// An IANA zone name.
fn zone(name: &str) -> Tz {
    name.parse()
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// A recorded Open-Meteo archive payload.
fn archive(name: &str) -> serde_json::Value {
    serde_json::from_str(&fixture(&format!("astro/{name}")))
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// The local instant of a recorded `YYYY-MM-DDTHH:MM[:SS]` string.
fn recorded_at(text: &str, tz: Tz) -> DateTime<FixedOffset> {
    let naive = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M"))
        .unwrap_or_else(|error| panic!("{text}: {error}"));
    cirrocast::model::resolve_local(tz, naive)
        .unwrap_or_else(|error| panic!("{text}: {error}"))
        .fixed_offset()
}

/// A report for a bare point, with no days: the sun block is then computed locally.
fn report_at(name: &str, lat: f64, lon: f64, tz: Tz) -> Report {
    let mut report = common::fixture_report("beijing-1d.json");
    report.location = Location {
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
    };
    report.current = None;
    report.days.clear();
    report
}

/// Local noon of `day`, the instant a report is computed at in these tests.
fn local_noon(tz: Tz, day: NaiveDate) -> DateTime<FixedOffset> {
    cirrocast::model::resolve_local(tz, day.and_hms_opt(12, 0, 0).expect("a valid time"))
        .expect("a valid local noon")
        .fixed_offset()
}

// ---------------------------------------------------------------------------------------------
// The moon
// ---------------------------------------------------------------------------------------------

#[test]
fn the_recorded_moon_instants_keep_their_phase_and_fraction() {
    for (text, expected_percent, expected_phase) in MOON_FIXTURES {
        let jd_tt = astro::julian::jde(astro::julian_day(instant(text)));
        let percent = astro::moon::illuminated_fraction(jd_tt) * 100.0;
        assert!(
            (percent - expected_percent).abs() <= 0.5,
            "{text}: {percent:.5}% illuminated, Horizons recorded {expected_percent:.5}%"
        );
        assert_eq!(
            astro::moon::phase_at(jd_tt),
            expected_phase,
            "{text}: wrong phase"
        );
    }
}

#[test]
fn the_book_instants_are_reproduced_within_two_minutes() {
    for (jd_td, phase, label) in BOOK_INSTANTS {
        let (_, found) = astro::moon::next_phases(jd_td - 2.0, 4)
            .into_iter()
            .find(|(candidate, _)| *candidate == phase)
            .unwrap_or_else(|| panic!("{label}: no {phase:?} in the next four phases"));
        let jd_ut = astro::julian_day(found);
        let jd_ephemeris = jd_ut + astro::delta_t_seconds(jd_ut) / 86_400.0;
        let minutes = (jd_ephemeris - jd_td).abs() * 1440.0;
        assert!(minutes <= 2.0, "{label}: off by {minutes:.1} min");
    }
}

#[test]
fn the_moon_phase_and_its_fraction_always_agree_at_an_instant() {
    // The phase name is a window of the same elongation the fraction is computed from, so the
    // two can never disagree about where the cycle is.
    for (text, _, _) in MOON_FIXTURES {
        let jd_tt = astro::julian::jde(astro::julian_day(instant(text)));
        let elongation = astro::moon::synodic_elongation(jd_tt);
        assert_eq!(
            astro::moon::phase_of(elongation),
            astro::moon::phase_at(jd_tt)
        );
    }
}

#[test]
fn the_moonrise_references_agree_within_ten_minutes() {
    for (lat, lon, tz_name, day, rise, set) in MOONRISE_REFERENCES {
        let tz = zone(tz_name);
        let date = date(day);
        let (found_rise, found_set) = astro::moon::moonrise_moonset(date, lat, lon, tz);
        for (label, found, reference) in [("rise", found_rise, rise), ("set", found_set, set)] {
            match (found, reference) {
                (Some(found), Some(reference)) => {
                    let expected = recorded_at(reference, tz);
                    let seconds = (found - expected).num_seconds().abs();
                    assert!(
                        seconds <= 600,
                        "{tz_name} {day} {label}: {found} is {seconds}s from the reference \
                         {expected}"
                    );
                }
                (None, None) => {}
                (found, reference) => {
                    panic!("{tz_name} {day} {label}: found {found:?}, reference {reference:?}")
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The sun
// ---------------------------------------------------------------------------------------------

#[test]
fn the_recorded_sun_days_are_reproduced_within_two_minutes() {
    for (file, lat, lon, tz_name, day) in SUN_CASES {
        let payload = archive(file);
        let tz = zone(tz_name);
        let date = date(day);
        let astro = Astro::compute(&report_at("fixture", lat, lon, tz), local_noon(tz, date));
        let sun = &astro.sun;
        assert_eq!(
            sun.source,
            SunSource::Local,
            "{file}: the point has no days"
        );

        let recorded_rise = payload["daily"]["sunrise"][0]
            .as_str()
            .expect("a recorded sunrise");
        let recorded_set = payload["daily"]["sunset"][0]
            .as_str()
            .expect("a recorded sunset");
        let recorded_daylight = payload["daily"]["daylight_duration"][0]
            .as_f64()
            .expect("a recorded daylight span");

        // The polar days are the clamp case: the provider reports midnight and a whole (or empty)
        // day; the module must refuse the clamp and name the state instead.
        let polar = matches!(
            recorded_daylight,
            seconds if seconds <= 0.5 || seconds >= 86_399.5
        );
        if polar {
            assert_eq!(sun.sunrise, None, "{file}: a clamped sunrise was copied");
            assert_eq!(sun.sunset, None, "{file}: a clamped sunset was copied");
            assert_eq!(
                sun.polar,
                Some(if recorded_daylight >= 86_399.5 {
                    Polar::Day
                } else {
                    Polar::Night
                }),
                "{file}: the polar state is not named"
            );
            let expected_daylight = if recorded_daylight >= 86_399.5 {
                86_400
            } else {
                0
            };
            assert_eq!(
                sun.daylight_secs,
                Some(expected_daylight),
                "{file}: the daylight span"
            );
            continue;
        }

        assert_eq!(sun.polar, None, "{file}");
        let rise = sun.sunrise.expect("a sunrise");
        let set = sun.sunset.expect("a sunset");
        for (label, found, recorded) in [
            ("sunrise", rise, recorded_rise),
            ("sunset", set, recorded_set),
        ] {
            let expected = recorded_at(recorded, tz);
            let seconds = (found - expected).num_seconds().abs();
            assert!(
                seconds <= 120,
                "{file} {label}: {found} is {seconds}s from the recorded {expected}"
            );
        }
        let daylight = u32::try_from((set - rise).num_seconds()).unwrap_or_default();
        assert!(
            (f64::from(daylight) - recorded_daylight).abs() <= 120.0,
            "{file}: {daylight}s of daylight, recorded {recorded_daylight}s"
        );
    }
}

#[test]
fn the_provider_sun_wins_and_still_matches_the_local_computation() {
    // `beijing-1d.json` carries the provider's sunrise/sunset for its day.
    let report = common::fixture_report("beijing-1d.json");
    let day = report.days[0].clone();
    let now = common::fixture_now(&report);
    let astro = Astro::compute(&report, now);

    assert_eq!(astro.sun.source, SunSource::Provider);
    assert_eq!(astro.sun.sunrise, day.sunrise);
    assert_eq!(astro.sun.sunset, day.sunset);
    assert_eq!(astro.sun.polar, None);
    assert_eq!(
        astro.sun.daylight_secs,
        Some(
            u32::try_from(
                (day.sunset.expect("a sunset") - day.sunrise.expect("a sunrise")).num_seconds()
            )
            .unwrap_or_default()
        )
    );

    // The same day computed here, to keep the two paths honest: the hand-written fixture's sun
    // times are not a recording, so this only catches a gross drift (the recorded Open-Meteo
    // payloads carry the tight comparison in the test above).
    let (rise, set, _) = astro::sun::sunrise_sunset(
        day.date,
        report.location.lat,
        report.location.lon,
        report.location.tz,
    );
    for (label, provider, local) in [("sunrise", day.sunrise, rise), ("sunset", day.sunset, set)] {
        let seconds = (provider.expect("a provider time") - local.expect("a local time"))
            .num_seconds()
            .abs();
        assert!(seconds <= 600, "{label}: {seconds}s apart");
    }
}

#[test]
fn the_leap_day_is_an_ordinary_day() {
    let tz = zone("Asia/Shanghai");
    let day = date("2024-02-29");
    let astro = Astro::compute(
        &report_at("Beijing", 39.9042, 116.4074, tz),
        local_noon(tz, day),
    );
    let sun = &astro.sun;
    assert_eq!(sun.source, SunSource::Local);
    assert_eq!(sun.polar, None);
    assert!(sun.sunrise.is_some() && sun.sunset.is_some());
    assert!(astro.moon.illuminated_fraction.is_finite());
    assert_ne!(
        astro.moon.next,
        [] as [(
            cirrocast::model::MoonPhase,
            chrono::DateTime<chrono::FixedOffset>
        ); 0]
    );
}

// ---------------------------------------------------------------------------------------------
// Rendering: the polar labels and the transition day
// ---------------------------------------------------------------------------------------------

/// The English catalog.
fn english() -> I18n {
    I18n::load(&LanguageRequest::Tag("en-US".to_owned()), |_| None)
}

/// A terminal that can draw the unicode block and the middle dot.
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

/// Renders `report` in `format` with `i18n`, at the run's own clock.
fn render(report: &Report, format: Format, i18n: &I18n, now: DateTime<FixedOffset>) -> String {
    let times = cirrocast::model::LocalTimes::new(now, report.location.tz);
    let ctx = RenderContext {
        units: UnitSystem::Metric
            .resolve(&UnitOverrides::default())
            .expect("the default overrides resolve"),
        color: ColorMode::Never,
        width: 80,
        term: capable(),
        times: times.clone(),
        lang: i18n.lang(),
        i18n,
        alert_credits: &[],
        aqi_index: cirrocast::air::aqi::AqiIndex::Us,
    };
    renderer_for(format, &capable(), None)
        .expect("the format has a renderer")
        .render(report, &ctx)
        .expect("the report renders")
}

#[test]
fn the_polar_labels_are_localised_in_both_catalogs() {
    let cases = [
        ("2026-06-21", Polar::Day, "polar day", "极昼"),
        ("2025-12-21", Polar::Night, "polar night", "极夜"),
    ];
    for (day, expected, english_text, chinese_text) in cases {
        let tz = zone("Arctic/Longyearbyen");
        let date = date(day);
        let mut report = report_at("Longyearbyen", 78.2232, 15.6469, tz);
        let now = local_noon(tz, date);
        report.astro = Some(Astro::compute(&report, now));
        assert_eq!(
            report.astro.as_ref().expect("an astro block").sun.polar,
            Some(expected)
        );

        for (i18n, text) in [
            (english(), english_text),
            (
                I18n::load(&LanguageRequest::Tag("zh-CN".to_owned()), |_| None),
                chinese_text,
            ),
        ] {
            let view = render(&report, Format::Moon, &i18n, now);
            assert!(view.contains(text), "{day}: {view}");
            // The sun line is the polar name, not a clamped midnight.
            assert!(
                !view.contains("Sunrise"),
                "{day}: a rise line leaked in: {view}"
            );
            assert!(
                !view.contains("Sunset"),
                "{day}: a set line leaked in: {view}"
            );
            assert!(!view.contains("NaN"), "{day}");
        }
    }
}

#[test]
fn a_transition_day_keeps_its_own_clock() {
    // New York, 2026-03-08: the spring-forward day is 23 hours long, and the sunrise/sunset are
    // the recorded 07:19 / 18:55 EDT.
    let tz = zone("America/New_York");
    let date = date("2026-03-08");
    let mut report = report_at("New York", 40.7128, -74.0060, tz);
    let now = local_noon(tz, date);
    report.astro = Some(Astro::compute(&report, now));
    let astro = report.astro.as_ref().expect("an astro block");
    for (label, found, recorded) in [
        (
            "sunrise",
            astro.sun.sunrise.expect("a sunrise"),
            "2026-03-08T07:19",
        ),
        (
            "sunset",
            astro.sun.sunset.expect("a sunset"),
            "2026-03-08T18:55",
        ),
    ] {
        let seconds = (found - recorded_at(recorded, tz)).num_seconds().abs();
        assert!(seconds <= 120, "{label}: {seconds}s from the recorded time");
    }

    // The rendered clock is the location's own, not UTC: 07:18 local is 11:18 UTC.
    let view = render(&report, Format::Moon, &english(), now);
    assert!(view.contains("Sunrise 07:1"), "{view}");
    assert!(view.contains("Sunset 18:5"), "{view}");
    assert!(!view.contains("11:1"), "{view}");
}

// ---------------------------------------------------------------------------------------------
// The CLI over a seeded cache
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

/// Seeds the geocode answer for `Beijing` and its weather, so `--offline` can replay both.
fn seed(sandbox: &Sandbox) {
    const LAT: f64 = 39.9075;
    const LON: f64 = 116.39723;
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
}

/// A successful offline run's stdout.
fn offline(sandbox: &Sandbox, args: &[&str]) -> String {
    let assert = sandbox
        .cirrocast()
        .env("TERM", "xterm-256color")
        .env("LANG", "en_US.UTF-8")
        .args(args)
        .args(["--offline", "--no-alerts", "--lang", "en-US"])
        .arg("Beijing")
        .assert()
        .success();
    String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8")
}

#[test]
fn every_moon_surface_works_from_the_cache_offline() {
    let sandbox = Sandbox::new();
    seed(&sandbox);

    // `plain`: one record per block.
    let plain = offline(&sandbox, &["--moon", "-f", "plain"]);
    assert!(plain.contains("\nmoon: "), "{plain}");
    assert!(plain.contains("\nsun: "), "{plain}");
    assert!(!plain.contains("NaN"), "{plain}");

    // The standalone view names its provenance: computed locally, no request.
    let view = offline(&sandbox, &["-f", "moon"]);
    assert!(view.contains("computed locally (no network)"), "{view}");
    assert!(view.contains("Next phases:"), "{view}");
    assert!(!view.contains("NaN"), "{view}");

    // `json`: the astro object, gated by the flag.
    let json = offline(&sandbox, &["--moon", "-f", "json"]);
    let document: serde_json::Value = serde_json::from_str(&json).expect("the document is JSON");
    assert!(document["astro"]["moon"]["phase"].is_string(), "{json}");
    assert_eq!(document["astro"]["sun"]["source"], "provider");
    assert!(
        document["astro"]["moon"]["next"]
            .as_array()
            .is_some_and(|next| next.len() == 4)
    );

    // Without `--moon` the object stays null: the renderer never invents a request.
    let without = offline(&sandbox, &["-f", "json"]);
    let document: serde_json::Value = serde_json::from_str(&without).expect("valid JSON");
    assert!(document["astro"].is_null(), "{without}");

    // The table appends the block below the forecast.
    let table = offline(&sandbox, &["--moon"]);
    assert!(table.contains("Moon: "), "{table}");
    assert!(
        table.contains("Weather report:") || table.contains("天气报告："),
        "{table}"
    );

    // The one-line tokens need no flag at all.
    let line = offline(&sandbox, &["-f", "one-line", "--template", "%M (%m) %t"]);
    assert!(!line.trim().is_empty(), "{line}");
    assert!(!line.contains('%'), "{line}");
    assert!(!line.contains("n/a"), "{line}");
}

#[test]
fn a_moon_flag_without_a_moon_surface_is_a_usage_error() {
    let sandbox = Sandbox::new();
    let assert = sandbox
        .cirrocast()
        .args(["--moon", "-f", "one-line", "--offline", "Beijing"])
        .assert()
        .code(2);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr is UTF-8");
    assert!(stderr.contains("--moon"), "{stderr}");
    assert!(stderr.contains("%m"), "{stderr}");
}

#[test]
fn the_help_documents_the_moon_tokens_next_to_the_others() {
    let sandbox = Sandbox::new();
    let assert = sandbox.cirrocast().arg("--help").assert().success();
    let help = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    assert!(help.contains("%m moon glyph"), "{help}");
    assert!(help.contains("%M moon phase"), "{help}");
    assert!(help.contains("%A strongest alert event"), "{help}");
    assert!(help.contains("%q air-quality index"), "{help}");
}
