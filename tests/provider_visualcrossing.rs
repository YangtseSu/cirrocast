// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Visual Crossing backend, driven by `tests/fixtures/visualcrossing/timeline.json`.
//!
//! No test here opens a socket: `StubTransport` answers from the fixtures, `FakeClock` supplies the
//! fetch instant, and the API key comes from a throwaway `keys.toml` the harness writes.
//!
//! **Fixture provenance.** No Visual Crossing API key exists in this repository, so the payload is
//! **hand-authored from the published timeline schema** rather than recorded from a live session:
//! it carries the documented top-level metadata, a `currentConditions` block, three days with 24
//! hourly records each and one `alerts[]` object whose `severity`/`urgency`/`certainty` triple is the
//! CAP spelling the decoder reads defensively (the published alert object documents only `event`,
//! `headline`, `description`, `onset` and `ends`). The epochs and every local `datetime` were
//! computed from the same instants, so the two spellings agree; a live recording, when one is
//! available, replaces this file without touching the decoder.

// Exact comparison is the point in this file: every value comes from the hand-authored fixture.
#![allow(clippy::float_cmp)]

mod common;

use chrono::NaiveDate;
use chrono_tz::Tz;
use predicates::prelude::*;

use cirrocast::cache::CacheMode;
use cirrocast::http::StubReply;
use cirrocast::model::{
    AlertSource, Certainty, Condition, DayForecast, DayPart, DayPartKind, Location, LocationSource,
    ReportMode, Severity, Urgency,
};
use cirrocast::provider::visualcrossing::{BASE, VisualCrossing};
use cirrocast::provider::{DateWindow, FetchRequest, HourlyResolution, Provider};
use common::{ProviderRun, Sandbox, fixture, fixture_reply, provider_clock};

/// The key the tests store; the fixture carries no real one.
const KEY: &str = "test-key-0123456789abcdef";

/// The coordinates the fixture was authored for (Reston, VA).
const LAT: f64 = 38.9697;
const LON: f64 = -77.385;

/// The location the fixture answers for, as a geocoded place.
fn reston() -> Location {
    Location {
        name: "Reston".to_owned(),
        admin1: Some("Virginia".to_owned()),
        country: "United States".to_owned(),
        country_code: Some("US".to_owned()),
        lat: LAT,
        lon: LON,
        tz: Tz::America__New_York,
        elevation_m: None,
        population: None,
        source: LocationSource::Geocoder,
        station: None,
    }
}

/// The same point as raw coordinates, whose provisional zone the response must correct.
fn reston_coordinates() -> Location {
    Location {
        name: format!("{LAT}, {LON}"),
        admin1: None,
        country: String::new(),
        country_code: None,
        lat: LAT,
        lon: LON,
        tz: Tz::UTC,
        elevation_m: None,
        population: None,
        source: LocationSource::Coordinates,
        station: None,
    }
}

/// A run over `replies` at the fixture's first local day.
fn run(replies: Vec<StubReply>, mode: CacheMode) -> ProviderRun {
    let run = ProviderRun::new(replies, provider_clock(2026, 10, 6), mode);
    run.with_key("visualcrossing", KEY);
    run
}

/// A run over the recorded timeline response.
fn timeline_run(mode: CacheMode) -> ProviderRun {
    run(vec![fixture_reply("visualcrossing", "timeline.json")], mode)
}

/// The four parts of `day`, in `DayPartKind::ALL` order.
fn part(day: &DayForecast, kind: DayPartKind) -> &DayPart {
    &day.parts[kind.index()]
}

/// `YYYY-MM-DD` as a date.
fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("a valid date")
}

#[test]
fn the_request_uses_the_keyword_period_with_metric_units_and_the_key_in_the_query() {
    let run = timeline_run(CacheMode::Normal);
    run.fetch_with(&VisualCrossing, &reston(), 3)
        .expect("the fixture parses");

    let calls = run.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url(), format!("{BASE}/38.9697,-77.3850/next3days"));
    let pairs: Vec<(&str, &str)> = calls[0]
        .query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("unitGroup", "metric"),
            ("include", "current,days,hours"),
            ("key", KEY),
        ]
    );
    // The key is a query parameter, so the wire URL carries it; every printed spelling redacts it.
    assert!(calls[0].full_url().contains(&format!("key={KEY}")));
    assert!(!calls[0].redacted_url().contains(KEY));
    assert!(!format!("{:?}", calls[0]).contains(KEY));
}

#[test]
fn a_current_only_request_asks_for_no_days_or_hours() {
    let run = timeline_run(CacheMode::Normal);
    let report = run
        .fetch_with(&VisualCrossing, &reston(), 0)
        .expect("the fixture parses");

    let calls = run.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url(), format!("{BASE}/38.9697,-77.3850/today"));
    let pairs: Vec<(&str, &str)> = calls[0]
        .query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("unitGroup", "metric"),
            ("include", "current"),
            ("key", KEY)
        ]
    );
    assert!(report.current.is_some());
    assert_eq!(report.days, Vec::new());
}

#[test]
fn the_current_block_decodes_from_the_epoch() {
    let run = timeline_run(CacheMode::Normal);
    let report = run
        .fetch_with(&VisualCrossing, &reston(), 3)
        .expect("the fixture parses");

    let current = report.current.expect("the fixture has current conditions");
    assert_eq!(
        current.observed_at.to_rfc3339(),
        "2026-10-06T08:00:00-04:00"
    );
    assert_eq!(current.temp_c, 18.5);
    assert_eq!(current.feels_like_c, Some(18.0));
    assert_eq!(current.humidity_pct, Some(72));
    assert_eq!(current.precip_mm, 0.0);
    assert_eq!(current.weather, Condition::from_u8(2));
    assert_eq!(current.cloud_cover_pct, Some(40));
    assert_eq!(current.pressure_hpa, 1015.0);
    assert_eq!(current.wind_kmh, 10.0);
    assert_eq!(current.wind_dir_deg, Some(200));
    assert_eq!(current.wind_gust_kmh, Some(15.0));
    assert_eq!(current.visibility_km, Some(16.0));
    assert_eq!(current.uv_index, Some(2.0));
    // 120 W/m² of solar radiation: the observation is in daylight.
    assert!(current.is_day);
}

#[test]
fn the_days_aggregate_into_the_four_parts() {
    let run = timeline_run(CacheMode::Normal);
    let report = run
        .fetch_with(&VisualCrossing, &reston(), 3)
        .expect("the fixture parses");

    let dates: Vec<NaiveDate> = report.days.iter().map(|day| day.date).collect();
    assert_eq!(
        dates,
        vec![date(2026, 10, 6), date(2026, 10, 7), date(2026, 10, 8)]
    );
    assert_eq!(report.mode, ReportMode::Forecast);

    let day = &report.days[0];
    // Representative samples: the hour closest to each part's midpoint.
    assert_eq!(part(day, DayPartKind::Morning).temp_c, 14.5);
    assert_eq!(part(day, DayPartKind::Noon).temp_c, 17.5);
    assert_eq!(part(day, DayPartKind::Evening).temp_c, 20.5);
    assert_eq!(part(day, DayPartKind::Night).temp_c, 11.5);
    assert_eq!(part(day, DayPartKind::Morning).humidity_pct, Some(69));
    assert_eq!(part(day, DayPartKind::Morning).wind_kmh, 10.0);
    assert_eq!(part(day, DayPartKind::Morning).wind_dir_deg, Some(207));
    assert_eq!(part(day, DayPartKind::Morning).visibility_km, Some(16.0));

    // Precipitation is the part sum (three 1.0 mm hours), the probability its maximum.
    assert_eq!(part(day, DayPartKind::Evening).precip_mm, 3.0);
    assert_eq!(part(day, DayPartKind::Evening).precip_prob_pct, Some(80));
    assert_eq!(part(day, DayPartKind::Morning).precip_mm, 0.0);
    assert_eq!(part(day, DayPartKind::Morning).precip_prob_pct, Some(5));

    // `rain` dominates the evening's three cloudy hours; the rest is the icon's own code.
    assert_eq!(
        part(day, DayPartKind::Evening).weather,
        Condition::from_u8(63)
    );
    assert_eq!(
        part(day, DayPartKind::Morning).weather,
        Condition::from_u8(2)
    );
    assert_eq!(part(day, DayPartKind::Noon).weather, Condition::from_u8(2));
    assert_eq!(part(day, DayPartKind::Night).weather, Condition::from_u8(0));

    // The daily extremes and sun times come from the day block.
    assert_eq!(day.temp_min_c, 10.0);
    assert_eq!(day.temp_max_c, 24.0);
    assert_eq!(
        day.sunrise.expect("a sunrise").to_rfc3339(),
        "2026-10-06T07:00:00-04:00"
    );
    assert_eq!(
        day.sunset.expect("a sunset").to_rfc3339(),
        "2026-10-06T18:10:00-04:00"
    );

    // A later day is decoded with its own values.
    assert_eq!(report.days[1].date, date(2026, 10, 7));
    assert_eq!(report.days[1].temp_min_c, 11.0);
    assert_eq!(report.days[1].temp_max_c, 23.0);
    assert_eq!(part(&report.days[1], DayPartKind::Night).temp_c, 12.5);
}

#[test]
fn a_coordinate_location_gets_the_response_zone() {
    let run = timeline_run(CacheMode::Normal);
    let report = run
        .fetch_with(&VisualCrossing, &reston_coordinates(), 3)
        .expect("the fixture parses");
    assert_eq!(report.location.tz, Tz::America__New_York);
}

#[test]
fn the_fixture_alert_maps_into_the_canonical_model() {
    let run = timeline_run(CacheMode::Normal);
    let report = run
        .fetch_with(&VisualCrossing, &reston(), 3)
        .expect("the fixture parses");

    assert_eq!(report.alerts.len(), 1);
    let alert = &report.alerts[0];
    assert_eq!(alert.source, AlertSource::VisualCrossing);
    assert_eq!(alert.event, "Flood Watch");
    assert_eq!(
        alert.headline,
        "Flood Watch in effect from Tuesday afternoon through late Tuesday night"
    );
    assert_eq!(
        alert.description.as_deref(),
        Some("Heavy rainfall may cause flooding of low-lying and poor-drainage areas.")
    );
    assert_eq!(alert.severity, Severity::Severe);
    assert_eq!(alert.urgency, Urgency::Expected);
    assert_eq!(alert.certainty, Certainty::Likely);
    assert_eq!(
        alert.onset.map(|at| at.to_rfc3339()),
        Some("2026-10-06T12:00:00-04:00".to_owned())
    );
    assert_eq!(
        alert.ends.map(|at| at.to_rfc3339()),
        Some("2026-10-07T02:00:00-04:00".to_owned())
    );
    assert!(alert.expires.is_none());
    // Without a source id the identity is the event and its onset.
    assert_eq!(
        alert.id,
        "visualcrossing:Flood Watch@2026-10-06T12:00:00-04:00"
    );
    // Liveness reads `ends`, which the fixture carries.
    assert_eq!(alert.effective_end(), alert.ends);
}

#[test]
fn the_standalone_alert_source_reuses_the_timeline_payload() {
    let run = timeline_run(CacheMode::Normal);
    let alerts = cirrocast::alerts::visualcrossing::fetch(&reston(), &run.env(), "en")
        .expect("the fixture's alerts decode");
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].source, AlertSource::VisualCrossing);
    assert_eq!(alerts[0].event, "Flood Watch");

    let calls = run.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url(), format!("{BASE}/38.9697,-77.3850"));
    let pairs: Vec<(&str, &str)> = calls[0]
        .query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![("unitGroup", "metric"), ("include", "alerts"), ("key", KEY)]
    );
}

#[test]
fn a_response_without_current_conditions_is_an_upstream_error() {
    let run = run(
        vec![fixture_reply(
            "visualcrossing",
            "timeline_without_current.json",
        )],
        CacheMode::Normal,
    );
    let error = run
        .fetch_with(&VisualCrossing, &reston(), 3)
        .expect_err("the payload has no currentConditions");
    assert_eq!(error.exit_code(), 3);
    assert!(error.to_string().contains("`currentConditions`"), "{error}");
}

#[test]
fn a_rejected_keyword_falls_back_to_the_explicit_date_range() {
    let run = run(
        vec![
            StubReply::status(400, Vec::new(), fixture("visualcrossing/error_400.json")),
            fixture_reply("visualcrossing", "timeline.json"),
        ],
        CacheMode::Normal,
    );
    let report = run
        .fetch_with(&VisualCrossing, &reston(), 3)
        .expect("the fallback answers");

    let calls = run.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].url(), format!("{BASE}/38.9697,-77.3850/next3days"));
    // 06:00Z is still 2 October's small hours in New York, so the window starts on the 6th and
    // spans the requested three days.
    assert_eq!(
        calls[1].url(),
        format!("{BASE}/38.9697,-77.3850/2026-10-06/2026-10-08")
    );
    assert_eq!(report.days.len(), 3);
}

#[test]
fn a_failed_fallback_does_not_leak_the_key() {
    let run = run(
        vec![
            StubReply::status(400, Vec::new(), fixture("visualcrossing/error_400.json")),
            StubReply::status(400, Vec::new(), fixture("visualcrossing/error_400.json")),
        ],
        CacheMode::Normal,
    );
    let error = run
        .fetch_with(&VisualCrossing, &reston(), 3)
        .expect_err("both attempts fail");
    assert_eq!(error.exit_code(), 3);
    assert!(!error.to_string().contains(KEY), "the key leaked: {error}");
    assert_eq!(run.calls().len(), 2, "one keyword attempt, one fallback");
}

#[test]
fn a_second_fetch_is_served_from_the_cache() {
    let run = timeline_run(CacheMode::Normal);
    run.fetch_with(&VisualCrossing, &reston(), 3)
        .expect("the fixture parses");
    run.fetch_with(&VisualCrossing, &reston(), 3)
        .expect("the cached fixture parses");
    assert_eq!(run.calls().len(), 1, "the cache must not refetch");
}

#[test]
fn the_history_backstop_is_a_usage_error() {
    let run = timeline_run(CacheMode::Normal);
    let window = DateWindow::day(date(2026, 9, 14));
    let error = VisualCrossing
        .fetch(
            &reston(),
            &FetchRequest::for_window(window, HourlyResolution::Hourly),
            &run.env(),
        )
        .expect_err("visualcrossing serves no archive");
    assert_eq!(error.exit_code(), 2);
    assert!(error.to_string().contains("no archive"), "{error}");
    assert_eq!(
        run.calls(),
        Vec::<cirrocast::http::HttpRequest>::new(),
        "no request may be sent"
    );
}

#[test]
fn a_run_without_a_key_names_both_ways_to_store_one() {
    // The CLI clears every `CIRROCAST_*` variable for the child, so the key store lookup is
    // deterministic and the environment variable is the one the registry names.
    let sandbox = Sandbox::new();
    sandbox
        .cirrocast()
        .args(["-p", "visualcrossing", "--lat", "38.97", "--lon", "-77.35"])
        .assert()
        .code(6)
        .stderr(
            predicate::str::contains("cirrocast key set visualcrossing")
                .and(predicate::str::contains("CIRROCAST_VISUALCROSSING_KEY")),
        );
}
