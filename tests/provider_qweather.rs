// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `QWeather` v1 backend, driven by the recorded responses in `tests/fixtures/qweather/`.
//!
//! No test here opens a socket: `StubTransport` answers from the fixtures, a fixed clock supplies
//! the fetch instant, and the API key comes from a throwaway `keys.toml` the harness writes. The
//! expected numbers were computed from the recorded JSON by a separate script applying the
//! documented aggregation rules.

// Exact comparison is the point in this file: every value comes from the recorded JSON.
#![allow(clippy::float_cmp)]

mod common;

use std::sync::Arc;
use std::time::SystemTime;

use chrono::{TimeZone as _, Utc};

use cirrocast::cache::{CacheMode, FakeClock};
use cirrocast::geo::from_coordinates;
use cirrocast::http::StubReply;
use cirrocast::model::{Condition, DayForecast, DayPart, DayPartKind};
use cirrocast::provider::qweather::QWeather;
use common::{ProviderRun, fixture, fixture_location, fixture_reply};

/// The key the tests store; the fixtures were scrubbed of the real one.
const KEY: &str = "test-key-0123456789abcdef";

/// The account host the tests configure.
const HOST: &str = "https://example.qweatherapi.com";

/// The instant the fixtures were recorded at (their first hour is `2026-10-01T00:00Z`).
fn recording_clock() -> Arc<FakeClock> {
    let start = Utc
        .with_ymd_and_hms(2026, 10, 1, 0, 5, 0)
        .single()
        .expect("a valid instant");
    Arc::new(FakeClock::new(SystemTime::from(start)))
}

/// A run over the recorded responses, with the host configured.
fn run(replies: Vec<StubReply>, mode: CacheMode) -> ProviderRun {
    let mut config = cirrocast::config::Config::default();
    HOST.clone_into(&mut config.providers.qweather.host);
    let run = ProviderRun::with_config(replies, recording_clock(), mode, config);
    run.with_key("qweather", KEY);
    run
}

/// A run over the current and hourly fixtures.
fn fixture_run(mode: CacheMode) -> ProviderRun {
    run(
        vec![
            fixture_reply("qweather", "current.json"),
            fixture_reply("qweather", "hourly.json"),
        ],
        mode,
    )
}

/// The four parts of `day`, in `DayPartKind::ALL` order.
fn part(day: &DayForecast, kind: DayPartKind) -> &DayPart {
    &day.parts[kind.index()]
}

#[test]
fn the_two_requests_use_the_configured_host_and_the_key_header() {
    let run = fixture_run(CacheMode::Normal);
    run.fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect("the fixtures parse");

    let calls = run.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0].url(),
        format!("{HOST}/weather/v1/current/39.9042/116.4074")
    );
    assert_eq!(
        calls[1].url(),
        format!("{HOST}/weather/v1/hourly/39.9042/116.4074")
    );
    assert_eq!(
        calls[1]
            .query_pairs()
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect::<Vec<_>>(),
        vec![("lang", "en"), ("hours", "120")]
    );
    assert_eq!(
        calls[0]
            .headers()
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect::<Vec<_>>(),
        vec![("X-QW-Api-Key", KEY)]
    );
}

#[test]
fn the_current_block_converts_the_measures() {
    let run = fixture_run(CacheMode::Normal);
    let report = run
        .fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect("the fixtures parse");

    let current = report.current.expect("the fixture has current conditions");
    // v1 reports no observation time; the fetch instant stands in.
    assert_eq!(
        current.observed_at.to_rfc3339(),
        "2026-10-01T08:05:00+08:00"
    );
    assert_eq!(current.temp_c, 11.78);
    assert_eq!(current.feels_like_c, Some(10.23));
    assert_eq!(current.humidity_pct, 48);
    assert_eq!(current.pressure_hpa, 1023.84);
    // m/s → km/h, metres → km, fractions → percent.
    assert_eq!(current.wind_kmh, 1.0 * 3.6);
    assert_eq!(current.wind_gust_kmh, Some(2.43 * 3.6));
    assert_eq!(current.wind_dir_deg, 236);
    assert_eq!(current.visibility_km, Some(21.0));
    assert_eq!(current.cloud_cover_pct, 0);
    assert_eq!(current.uv_index, Some(1.0));
    assert_eq!(current.weather, Condition::from_u8(0));
    assert!(current.is_day);
}

#[test]
fn the_hourly_series_becomes_the_covered_local_days() {
    let run = fixture_run(CacheMode::Normal);
    let report = run
        .fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect("the fixtures parse");

    // The series starts at the next UTC midnight (08:00 local), so the location-local today is
    // incomplete and the two complete days that follow are what a three-day request gets.
    let dates: Vec<String> = report.days.iter().map(|day| day.date.to_string()).collect();
    assert_eq!(dates, vec!["2026-10-02", "2026-10-03"]);

    let first = &report.days[0];
    assert_eq!(part(first, DayPartKind::Morning).temp_c, 16.06);
    assert_eq!(part(first, DayPartKind::Noon).temp_c, 22.64);
    assert_eq!(part(first, DayPartKind::Evening).temp_c, 16.17);
    assert_eq!(part(first, DayPartKind::Night).temp_c, 10.96);
    assert_eq!(part(first, DayPartKind::Morning).wind_kmh, 2.26 * 3.6);
    assert_eq!(part(first, DayPartKind::Morning).wind_dir_deg, Some(86));
    assert_eq!(part(first, DayPartKind::Morning).humidity_pct, Some(28));
    assert_eq!(part(first, DayPartKind::Morning).precip_prob_pct, Some(0));
    assert_eq!(
        part(first, DayPartKind::Morning).visibility_km,
        Some(20.835)
    );
    assert_eq!(
        part(first, DayPartKind::Morning).weather,
        Condition::from_u8(0)
    );
    assert_eq!(first.temp_min_c, 10.17);
    assert_eq!(first.temp_max_c, 22.65);
    // v1's daily block is not consumed, so there are no sun times.
    assert!(first.sunrise.is_none() && first.sunset.is_none());

    // The third day mixes 103 (partly cloudy → 2) and 102 (few clouds → 1); the most severe code
    // present wins each part.
    let second = &report.days[1];
    assert_eq!(
        part(second, DayPartKind::Morning).weather,
        Condition::from_u8(2)
    );
    assert_eq!(
        part(second, DayPartKind::Noon).weather,
        Condition::from_u8(2)
    );
    assert_eq!(
        part(second, DayPartKind::Evening).weather,
        Condition::from_u8(2)
    );
    assert_eq!(
        part(second, DayPartKind::Night).weather,
        Condition::from_u8(1)
    );
}

#[test]
fn a_missing_host_is_a_configuration_error_with_the_hint() {
    let run = ProviderRun::new(
        vec![fixture_reply("qweather", "current.json")],
        recording_clock(),
        CacheMode::Normal,
    );
    run.with_key("qweather", KEY);
    let error = run
        .fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect_err("no host is configured");
    assert_eq!(error.exit_code(), 4);
    let text = error.to_string();
    assert!(text.contains("providers.qweather.host"), "{text}");
    assert!(text.contains("console.qweather.com/setting"), "{text}");
    assert!(text.contains("provider info qweather"), "{text}");
    assert_eq!(
        run.calls(),
        Vec::<cirrocast::http::HttpRequest>::new(),
        "no request may be sent"
    );
}

#[test]
fn a_provisional_zone_is_refused_before_any_request() {
    let run = fixture_run(CacheMode::Normal);
    let error = run
        .fetch_with(&QWeather, &from_coordinates(39.9042, 116.4074), 3)
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
    let run = fixture_run(CacheMode::Normal);
    run.fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect("the fixtures parse");
    run.fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect("the cached fixtures parse");
    assert_eq!(
        run.calls().len(),
        2,
        "the cache must not refetch either call"
    );
}

#[test]
fn a_rejected_key_is_an_invalid_key_error() {
    let run = run(
        vec![StubReply::status(
            401,
            Vec::new(),
            fixture("qweather/error_401.json"),
        )],
        CacheMode::Normal,
    );
    let error = run
        .fetch_with(&QWeather, &fixture_location("beijing"), 3)
        .expect_err("a 401 is a rejected key");
    assert_eq!(error.exit_code(), 6);
    let text = error.to_string();
    assert!(text.contains("cirrocast key set qweather"), "{text}");
    assert!(!text.contains(KEY), "the key leaked: {text}");
}
