// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `WorldWeatherOnline` backend, driven by the recorded response in `tests/fixtures/wwo/`.
//!
//! No test here opens a socket: `StubTransport` answers from the fixture, a fixed clock supplies the
//! fetch instant, and the API key comes from a throwaway `keys.toml` the harness writes. The
//! expected numbers were computed from the recorded JSON by a separate script applying the
//! documented aggregation rules.

// Exact comparison is the point in this file: every value comes from the recorded JSON.
#![allow(clippy::float_cmp)]

mod common;

use std::sync::Arc;
use std::time::SystemTime;

use chrono::{TimeZone as _, Utc};

use cirrocast::cache::{CacheMode, FakeClock};
use cirrocast::error::Error;
use cirrocast::geo::from_coordinates;
use cirrocast::http::StubReply;
use cirrocast::model::{Condition, DayForecast, DayPart, DayPartKind};
use cirrocast::provider::worldweatheronline::{WEATHER_URL, WorldWeatherOnline};
use common::{ProviderRun, fixture, fixture_location, fixture_reply};

/// The key the tests store; the fixture was scrubbed of the real one.
const KEY: &str = "test-key-0123456789abcdef";

/// The instant the fixture was recorded at (its `observation_time` is 04:29 PM UTC).
fn recording_clock() -> Arc<FakeClock> {
    let start = Utc
        .with_ymd_and_hms(2026, 9, 30, 16, 29, 0)
        .single()
        .expect("a valid instant");
    Arc::new(FakeClock::new(SystemTime::from(start)))
}

/// A run over the recorded response, at the recording instant.
fn run(mode: CacheMode) -> ProviderRun {
    let run = ProviderRun::new(
        vec![fixture_reply("wwo", "weather_ashx.json")],
        recording_clock(),
        mode,
    );
    run.with_key("worldweatheronline", KEY);
    run
}

/// The four parts of `day`, in `DayPartKind::ALL` order.
fn part(day: &DayForecast, kind: DayPartKind) -> &DayPart {
    &day.parts[kind.index()]
}

#[test]
fn the_request_asks_for_json_and_three_hourly_steps() {
    let run = run(CacheMode::Normal);
    run.fetch_with(&WorldWeatherOnline, &fixture_location("beijing"), 3)
        .expect("the fixture parses");

    let calls = run.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url(), WEATHER_URL);
    let pairs: Vec<(&str, &str)> = calls[0]
        .query_pairs()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("key", KEY),
            ("q", "39.9042,116.4074"),
            ("format", "json"),
            ("num_of_days", "3"),
            ("tp", "3"),
        ]
    );
}

#[test]
fn the_current_block_unwraps_the_string_values() {
    let run = run(CacheMode::Normal);
    let report = run
        .fetch_with(&WorldWeatherOnline, &fixture_location("beijing"), 3)
        .expect("the fixture parses");

    let current = report.current.expect("the fixture has current conditions");
    // `observation_time` is the UTC wall clock (04:29 PM), converted to the location's zone.
    assert_eq!(
        current.observed_at.to_rfc3339(),
        "2026-10-01T00:29:00+08:00"
    );
    assert_eq!(current.temp_c, 15.0);
    assert_eq!(current.feels_like_c, Some(8.0));
    assert_eq!(current.humidity_pct, Some(13));
    assert_eq!(current.pressure_hpa, 1023.0);
    assert_eq!(current.wind_kmh, 21.0);
    assert_eq!(current.wind_dir_deg, Some(302));
    assert_eq!(current.visibility_km, Some(10.0));
    assert_eq!(current.cloud_cover_pct, Some(0));
    assert_eq!(current.uv_index, Some(0.0));
    assert_eq!(current.weather, Condition::from_u8(0));
    assert!(!current.is_day);
}

#[test]
fn the_days_join_the_unpadded_hour_strings_to_the_dates() {
    let run = run(CacheMode::Normal);
    let report = run
        .fetch_with(&WorldWeatherOnline, &fixture_location("beijing"), 3)
        .expect("the fixture parses");

    let dates: Vec<String> = report.days.iter().map(|day| day.date.to_string()).collect();
    assert_eq!(dates, vec!["2026-10-01", "2026-10-02", "2026-10-03"]);

    let first = &report.days[0];
    assert_eq!(part(first, DayPartKind::Morning).temp_c, 16.0);
    assert_eq!(part(first, DayPartKind::Noon).temp_c, 22.0);
    assert_eq!(part(first, DayPartKind::Evening).temp_c, 18.0);
    assert_eq!(part(first, DayPartKind::Night).temp_c, 15.0);
    assert_eq!(part(first, DayPartKind::Morning).wind_kmh, 19.0);
    assert_eq!(part(first, DayPartKind::Morning).wind_dir_deg, Some(319));
    assert_eq!(part(first, DayPartKind::Morning).humidity_pct, Some(12));
    assert_eq!(part(first, DayPartKind::Morning).precip_prob_pct, Some(0));
    // 149 (smoky haze) maps into the atmosphere family.
    assert_eq!(
        part(first, DayPartKind::Evening).weather,
        Condition::from_u8(45)
    );

    // The daily extremes come from the response's own `maxtempC`/`mintempC`.
    assert_eq!(first.temp_min_c, 14.0);
    assert_eq!(first.temp_max_c, 22.0);

    // The `astronomy` array's 12-hour clock strings are joined to the day's local date.
    assert_eq!(
        first.sunrise.expect("a sunrise").to_rfc3339(),
        "2026-10-01T06:10:00+08:00"
    );
    assert_eq!(
        first.sunset.expect("a sunset").to_rfc3339(),
        "2026-10-01T17:58:00+08:00"
    );
}

#[test]
fn a_provisional_zone_is_refused_before_any_request() {
    let run = run(CacheMode::Normal);
    let error = run
        .fetch_with(&WorldWeatherOnline, &from_coordinates(39.9042, 116.4074), 3)
        .expect_err("raw coordinates carry no zone");
    assert_eq!(error.exit_code(), 2);
    assert!(error.to_string().contains("pass a place name"), "{error}");
    assert_eq!(
        run.calls(),
        Vec::<cirrocast::http::HttpRequest>::new(),
        "no request may be sent"
    );
}

#[test]
fn a_second_fetch_is_served_from_the_cache() {
    let run = run(CacheMode::Normal);
    run.fetch_with(&WorldWeatherOnline, &fixture_location("beijing"), 3)
        .expect("the fixture parses");
    run.fetch_with(&WorldWeatherOnline, &fixture_location("beijing"), 3)
        .expect("the cached fixture parses");
    assert_eq!(run.calls().len(), 1, "the cache must not refetch");
}

#[test]
fn a_rejected_key_is_an_invalid_key_error() {
    let run = ProviderRun::new(
        vec![StubReply::status(
            401,
            Vec::new(),
            fixture("wwo/error_401.json"),
        )],
        recording_clock(),
        CacheMode::Normal,
    );
    run.with_key("worldweatheronline", KEY);
    let error = run
        .fetch_with(&WorldWeatherOnline, &fixture_location("beijing"), 3)
        .expect_err("a 401 is a rejected key");
    assert_eq!(error.exit_code(), 6);
    let text = error.to_string();
    assert!(
        text.contains("cirrocast key set worldweatheronline"),
        "{text}"
    );
    assert!(!text.contains(KEY), "the key leaked: {text}");
}

#[test]
fn an_error_envelope_is_an_upstream_error_not_an_empty_report() {
    // Upstream answers a bad key with HTTP 200 and `{"data":{"error":[…]}}`; `-d 0` used to skip
    // the empty-forecast guard and return a successful report with no current and no days.
    let run = ProviderRun::new(
        vec![StubReply::ok(200, fixture("wwo/error_401.json"))],
        recording_clock(),
        CacheMode::Normal,
    );
    run.with_key("worldweatheronline", KEY);
    let error = run
        .fetch_with(&WorldWeatherOnline, &fixture_location("beijing"), 0)
        .expect_err("an error envelope must not become an empty report");
    assert_eq!(error.exit_code(), 3);
    assert!(matches!(&error, Error::Upstream { .. }), "{error:?}");
    assert!(error.to_string().contains("API key is invalid"), "{error}");
}
